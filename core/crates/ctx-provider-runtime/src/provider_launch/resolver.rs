use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, path::Path as StdPath};

use anyhow::{Context, Result};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderProcessInfo, ProviderRestartMode,
    ProviderSessionSweepConfig, ProviderSessionSweepStats, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;

use crate::ProviderRuntimeHost;
use ctx_managed_installs as installer;
use ctx_provider_install::install_state::InstallTarget;

struct StaticStatusAdapter {
    status: ProviderStatus,
}

#[async_trait::async_trait]
impl ProviderAdapter for StaticStatusAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(self.status.clone())
    }

    async fn run(
        &self,
        _input: ctx_providers::adapters::TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> Result<ctx_providers::adapters::RunHandle> {
        let msg = self
            .status
            .diagnostics
            .first()
            .cloned()
            .unwrap_or_else(|| "provider is unavailable".to_string());
        anyhow::bail!("{msg}");
    }

    async fn cancel(&self, _handle: &mut ctx_providers::adapters::RunHandle) -> Result<()> {
        Ok(())
    }
}

fn static_status_adapter(
    provider_id: &str,
    installed: bool,
    health: ProviderHealth,
    error_code: &str,
    message: String,
) -> Arc<dyn ProviderAdapter> {
    let mut details = HashMap::new();
    details.insert("error_code".to_string(), error_code.to_string());
    Arc::new(StaticStatusAdapter {
        status: ProviderStatus {
            provider_id: provider_id.to_string(),
            installed,
            detected_path: None,
            version: None,
            capabilities: None,
            health,
            diagnostics: vec![message],
            details,
            usability: ctx_providers::adapters::ProviderUsability::default(),
        },
    })
}

fn runtime_command_as_agent_command_for_target(
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<installer::AgentServerCommand>> {
    let Some(resolved) =
        installer::resolve_runtime_provider_command_for_target(cfg, provider_id, requested_target)?
    else {
        return Ok(None);
    };
    Ok(Some(installer::AgentServerCommand {
        command: resolved.command_abs_path,
        args: resolved.args,
        dependencies: resolved.dependencies,
        managed: None,
    }))
}

fn acp_status_adapter_bridge_missing(provider_id: &str, msg: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "acp_bridge_missing",
        msg,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenHandsRuntimeContract {
    UpstreamAcp,
    ShimAcp,
    Unknown,
}

impl OpenHandsRuntimeContract {
    fn as_str(self) -> &'static str {
        match self {
            Self::UpstreamAcp => "upstream_acp",
            Self::ShimAcp => "shim_acp",
            Self::Unknown => "unknown",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::UpstreamAcp => "runtime command matches the upstream `openhands acp` contract",
            Self::ShimAcp => "runtime command still points at the legacy `openhands-acp` shim",
            Self::Unknown => {
                "runtime command does not clearly match the upstream `openhands acp` contract"
            }
        }
    }

    fn supports_real_runtime(self) -> bool {
        matches!(self, Self::UpstreamAcp)
    }
}

fn path_file_stem_or_raw(raw: &str) -> String {
    StdPath::new(raw)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(raw)
        .to_string()
}

fn path_file_name_or_raw(raw: &str) -> String {
    StdPath::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(raw)
        .to_string()
}

fn openhands_runtime_contract_for_command(
    cmd: &installer::AgentServerCommand,
) -> OpenHandsRuntimeContract {
    let command_stem = path_file_stem_or_raw(&cmd.command).to_ascii_lowercase();
    let command_name = path_file_name_or_raw(&cmd.command).to_ascii_lowercase();

    if command_stem == "openhands" && cmd.args.first().is_some_and(|arg| arg == "acp") {
        return OpenHandsRuntimeContract::UpstreamAcp;
    }

    if command_stem == "openhands-acp" || command_name == "openhands-acp.js" {
        return OpenHandsRuntimeContract::ShimAcp;
    }

    if command_stem.starts_with("python") {
        let module_arg = cmd.args.iter().position(|arg| arg == "-m");
        if let Some(idx) = module_arg {
            let module = cmd
                .args
                .get(idx + 1)
                .map(String::as_str)
                .unwrap_or_default();
            let subcommand = cmd
                .args
                .get(idx + 2)
                .map(String::as_str)
                .unwrap_or_default();
            if matches!(module, "openhands" | "openhands.core.main") && subcommand == "acp" {
                return OpenHandsRuntimeContract::UpstreamAcp;
            }
        }
    }

    if let Some(first_arg) = cmd.args.first() {
        let first_stem = path_file_stem_or_raw(first_arg).to_ascii_lowercase();
        let first_name = path_file_name_or_raw(first_arg).to_ascii_lowercase();
        if first_stem == "openhands-acp" || first_name == "openhands-acp.js" {
            return OpenHandsRuntimeContract::ShimAcp;
        }
    }

    OpenHandsRuntimeContract::Unknown
}

fn apply_openhands_runtime_contract_details(
    status: &mut ProviderStatus,
    contract: OpenHandsRuntimeContract,
) {
    status.details.insert(
        "openhands_runtime_contract".to_string(),
        contract.as_str().to_string(),
    );
    status.details.insert(
        "openhands_real_runtime".to_string(),
        if contract.supports_real_runtime() {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );
    status.details.insert(
        "openhands_runtime_contract_note".to_string(),
        contract.note().to_string(),
    );
}

struct OpenHandsRuntimeContractAdapter {
    inner: Arc<dyn ProviderAdapter>,
    contract: OpenHandsRuntimeContract,
}

impl OpenHandsRuntimeContractAdapter {
    fn new(inner: Arc<dyn ProviderAdapter>, contract: OpenHandsRuntimeContract) -> Self {
        Self { inner, contract }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenHandsRuntimeContractAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let mut status = self.inner.inspect().await?;
        apply_openhands_runtime_contract_details(&mut status, self.contract);
        Ok(status)
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> Result<RunHandle> {
        self.inner.run(input, workdir, env, event_sink).await
    }

    async fn cancel(&self, handle: &mut RunHandle) -> Result<()> {
        self.inner.cancel(handle).await
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        self.inner.list_processes().await
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        self.inner.restart(reason, mode).await
    }

    async fn reap_idle_sessions(
        &self,
        config: ProviderSessionSweepConfig,
    ) -> Result<ProviderSessionSweepStats> {
        self.inner.reap_idle_sessions(config).await
    }

    async fn has_live_session(&self, session_key: &str) -> bool {
        self.inner.has_live_session(session_key).await
    }

    fn supports_resume(&self) -> bool {
        self.inner.supports_resume()
    }

    async fn set_session_model(&self, session_key: String, model_id: String) -> Result<()> {
        self.inner.set_session_model(session_key, model_id).await
    }

    async fn set_session_mode(&self, session_key: String, mode_id: String) -> Result<()> {
        self.inner.set_session_mode(session_key, mode_id).await
    }

    async fn authenticate_session(
        &self,
        session_key: String,
        workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> Result<()> {
        self.inner
            .authenticate_session(session_key, workdir, env, method_id, event_sink)
            .await
    }
}

fn acp_status_adapter_bridge_invalid(provider_id: &str, msg: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "acp_bridge_invalid",
        msg,
    )
}

fn acp_status_adapter_acp_command_invalid(
    provider_id: &str,
    msg: String,
) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        true,
        ProviderHealth::Error,
        "acp_command_invalid",
        msg,
    )
}

fn runtime_command_missing_adapter(provider_id: &str) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Missing,
        "runtime_command_missing",
        format!("runtime command is not configured for provider '{provider_id}'"),
    )
}

fn runtime_command_invalid_adapter(provider_id: &str, err: String) -> Arc<dyn ProviderAdapter> {
    static_status_adapter(
        provider_id,
        false,
        ProviderHealth::Error,
        "runtime_command_invalid",
        format!("invalid runtime command for provider '{provider_id}': {err}"),
    )
}

pub fn is_acp_provider_id(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "gemini"
            | "qwen"
            | "cursor"
            | "pi"
            | "opencode"
            | "mistral"
            | "goose"
            | "kimi"
            | "auggie"
            | "amp"
            | "droid"
            | "copilot"
            | "cline"
            | "openhands"
    )
}

fn escape_shell_arg(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let is_simple = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"@%_-+=:,./".contains(&b));
    if is_simple {
        return value.to_string();
    }
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn format_shell_command(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(escape_shell_arg(command));
    for arg in args {
        parts.push(escape_shell_arg(arg));
    }
    parts.join(" ")
}

pub fn acp_bridge_command(
    bridge_cmd: &installer::AgentServerCommand,
    acp_cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    let acp_command = format_shell_command(&acp_cmd.command, &acp_cmd.args);
    let mut args = bridge_cmd.args.clone();
    args.push("--acp-command".to_string());
    args.push(acp_command);
    installer::AgentServerCommand {
        command: bridge_cmd.command.clone(),
        args,
        dependencies: Vec::new(),
        managed: None,
    }
}

pub fn acp_bridge_adapter(
    id: &str,
    bridge_cmd: &installer::AgentServerCommand,
    acp_cmd: installer::AgentServerCommand,
) -> Arc<dyn ProviderAdapter> {
    let contract = if id == "openhands" {
        Some(openhands_runtime_contract_for_command(&acp_cmd))
    } else {
        None
    };
    let bridged = acp_bridge_command(bridge_cmd, acp_cmd);
    let inner: Arc<dyn ProviderAdapter> = Arc::new(Tier1CrpAdapter::from_provider_runtime(
        id,
        bridged.command,
        bridged.args,
    ));
    if let Some(contract) = contract {
        return Arc::new(OpenHandsRuntimeContractAdapter::new(inner, contract));
    }
    inner
}

#[derive(Debug, Clone)]
pub struct ExplicitGeminiCliPaths {
    pub cli_entry_path: PathBuf,
    pub core_entry_path: PathBuf,
}

fn file_stem_matches(path: &StdPath, name: &str) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

fn resolve_existing_absolute_path(raw: &str, label: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    anyhow::ensure!(
        path.is_absolute(),
        "{label} must be an absolute path; got '{raw}'"
    );
    anyhow::ensure!(path.exists(), "{label} does not exist: {}", path.display());
    Ok(path)
}

fn gemini_cli_root_from_entrypoint(path: &StdPath) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let bundle_dir = path.parent()?;
    let package_dir = bundle_dir.parent()?;
    let scope_dir = package_dir.parent()?;
    let node_modules_dir = scope_dir.parent()?;
    if file_name != "gemini.js"
        || bundle_dir.file_name()?.to_str()? != "bundle"
        || package_dir.file_name()?.to_str()? != "gemini-cli"
        || scope_dir.file_name()?.to_str()? != "@google"
        || node_modules_dir.file_name()?.to_str()? != "node_modules"
    {
        return None;
    }
    Some(package_dir.to_path_buf())
}

pub fn resolve_explicit_gemini_cli_paths(
    command: &str,
    args: &[String],
) -> Result<ExplicitGeminiCliPaths> {
    let node_path = resolve_existing_absolute_path(command, "Gemini ACP runtime command")?;
    anyhow::ensure!(
        file_stem_matches(&node_path, "node"),
        "Gemini ACP runtime must use an explicit absolute node executable plus @google/gemini-cli/bundle/gemini.js; got command '{command}'"
    );

    let arg0 = args.first().ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP runtime must pass an explicit absolute @google/gemini-cli/bundle/gemini.js entrypoint as the first argument"
        )
    })?;
    let cli_entry_path = resolve_existing_absolute_path(arg0, "Gemini ACP entrypoint")?;
    let cli_root = gemini_cli_root_from_entrypoint(&cli_entry_path).ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP entrypoint must point to @google/gemini-cli/bundle/gemini.js; got '{}'",
            cli_entry_path.display()
        )
    })?;
    let bundle_dir = cli_root.join("bundle");
    let mut core_entries = fs::read_dir(&bundle_dir)
        .with_context(|| format!("reading Gemini ACP bundle dir {}", bundle_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|s| s.to_str()) == Some("js")
                && path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.starts_with("core-") || s.eq_ignore_ascii_case("core.js"))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !core_entries.is_empty(),
        "Gemini ACP bundled core entrypoint is missing under {}",
        bundle_dir.display()
    );
    core_entries.sort();
    anyhow::ensure!(
        core_entries.len() == 1,
        "Gemini ACP bundle must contain exactly one core entrypoint under {}; found: {}",
        bundle_dir.display(),
        core_entries
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let Some(core_entry_path) = core_entries.into_iter().next() else {
        anyhow::bail!(
            "Gemini ACP bundled core entrypoint is missing under {}",
            bundle_dir.display()
        );
    };
    anyhow::ensure!(
        cli_root.join("package.json").exists(),
        "Gemini ACP entrypoint must live under a node_modules/@google/gemini-cli install tree: {}",
        cli_entry_path.display()
    );

    Ok(ExplicitGeminiCliPaths {
        cli_entry_path,
        core_entry_path,
    })
}

fn maybe_wrap_gemini_acp_command(
    data_root: &Path,
    mut cmd: installer::AgentServerCommand,
) -> Result<installer::AgentServerCommand> {
    let paths = resolve_explicit_gemini_cli_paths(&cmd.command, &cmd.args)?;

    let wrapper_path = data_root
        .join("providers")
        .join("agent-servers")
        .join("gemini-acp-wrapper.mjs");
    if let Some(parent) = wrapper_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating Gemini ACP wrapper dir {}", parent.display()))?;
    }

    let wrapper_contents = format!(
        "import {{ coreEvents, CoreEvent, writeToStdout, writeToStderr }} from 'file://{}';\n\
coreEvents.on(CoreEvent.Output, (payload) => {{\n\
  if (payload.isStderr) {{\n\
    writeToStderr(payload.chunk, payload.encoding);\n\
  }} else {{\n\
    writeToStdout(payload.chunk, payload.encoding);\n\
  }}\n\
}});\n\
coreEvents.on(CoreEvent.ConsoleLog, (payload) => {{\n\
  writeToStderr(String(payload?.content ?? '') + '\\n');\n\
}});\n\
const consentRaw = process.env.CTX_GEMINI_AUTO_OAUTH_CONSENT ?? '';\n\
const consentDisabled = consentRaw === '0' || consentRaw.toLowerCase() === 'false';\n\
if (!consentDisabled) {{\n\
  coreEvents.on(CoreEvent.ConsentRequest, (payload) => {{\n\
    if (typeof payload?.onConfirm === 'function') {{\n\
      payload.onConfirm(true);\n\
    }}\n\
  }});\n\
}}\n\
process.env.GEMINI_CLI_NO_RELAUNCH ??= 'true';\n\
await import('file://{}');\n",
        paths.core_entry_path.to_string_lossy(),
        paths.cli_entry_path.to_string_lossy(),
    );

    let write_wrapper = match fs::read_to_string(&wrapper_path) {
        Ok(existing) => existing != wrapper_contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading Gemini ACP wrapper {}", wrapper_path.display()));
        }
    };
    if write_wrapper {
        fs::write(&wrapper_path, wrapper_contents)
            .with_context(|| format!("writing Gemini ACP wrapper {}", wrapper_path.display()))?;
    }

    let first = cmd.args.first_mut().ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP runtime must pass an explicit absolute @google/gemini-cli/bundle/gemini.js entrypoint as the first argument"
        )
    })?;
    *first = wrapper_path.to_string_lossy().to_string();
    Ok(cmd)
}

fn maybe_set_qwen_openai_auth_type(
    mut cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    if cmd.args.iter().any(|arg| arg == "--auth-type") {
        return cmd;
    }
    cmd.args.push("--auth-type".to_string());
    cmd.args.push("openai".to_string());
    cmd
}

fn maybe_set_bridge_env_override(
    mut cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    if cmd.args.iter().any(|arg| arg == "--override-with-envs") {
        return cmd;
    }
    cmd.args.push("--override-with-envs".to_string());
    cmd
}

fn goose_args_include_developer_builtin(args: &[String]) -> bool {
    args.windows(2).any(|window| {
        window[0] == "--with-builtin"
            && window[1]
                .split(',')
                .any(|value| value.trim() == "developer")
    })
}

fn maybe_set_goose_acp_subcommand(
    mut cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    if !cmd.args.iter().any(|arg| arg == "acp")
        && file_stem_matches(StdPath::new(&cmd.command), "goose")
    {
        cmd.args.insert(0, "acp".to_string());
    }
    if !goose_args_include_developer_builtin(&cmd.args) {
        cmd.args.push("--with-builtin".to_string());
        cmd.args.push("developer".to_string());
    }
    cmd
}

pub fn normalize_acp_provider_command(
    data_root: &Path,
    provider_id: &str,
    cmd: installer::AgentServerCommand,
) -> Result<installer::AgentServerCommand> {
    let cmd = if provider_id == "gemini" {
        maybe_wrap_gemini_acp_command(data_root, cmd)?
    } else {
        cmd
    };
    let cmd = if provider_id == "qwen" {
        maybe_set_qwen_openai_auth_type(cmd)
    } else {
        cmd
    };
    let cmd = if provider_id == "goose" {
        maybe_set_goose_acp_subcommand(cmd)
    } else {
        cmd
    };
    let cmd = if provider_id == "openhands" {
        maybe_set_bridge_env_override(cmd)
    } else {
        cmd
    };
    Ok(cmd)
}

pub fn runtime_probe_command_as_agent_command_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<installer::AgentServerCommand>> {
    let Some(runtime_cmd) =
        runtime_command_as_agent_command_for_target(cfg, provider_id, requested_target)?
    else {
        return Ok(None);
    };
    if !is_acp_provider_id(provider_id) {
        return Ok(Some(runtime_cmd));
    }

    let bridge_cmd =
        runtime_command_as_agent_command_for_target(cfg, "acp-crp-bridge", requested_target)?
            .ok_or_else(|| {
                anyhow::anyhow!("runtime command is not configured for provider 'acp-crp-bridge'")
            })?;
    let normalized = normalize_acp_provider_command(data_root, provider_id, runtime_cmd)?;
    let mut bridged = acp_bridge_command(&bridge_cmd, normalized.clone());
    let mut dependencies = bridge_cmd.dependencies.clone();
    for dependency in &normalized.dependencies {
        if !dependencies.contains(dependency) {
            dependencies.push(dependency.clone());
        }
    }
    bridged.dependencies = dependencies;
    bridged.managed = bridge_cmd.managed.clone();
    Ok(Some(bridged))
}

#[cfg(test)]
pub(crate) fn runtime_probe_command_as_agent_command(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
) -> Result<Option<installer::AgentServerCommand>> {
    runtime_probe_command_as_agent_command_for_target(data_root, cfg, provider_id, None)
}

fn target_adapter_cache_key(provider_id: &str, target: InstallTarget) -> Option<String> {
    (!matches!(target, InstallTarget::Host)).then(|| format!("{provider_id}@{}", target.as_str()))
}

fn build_provider_adapter_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if matches!(provider_id, "codex" | "claude-crp") {
        return match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
            Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_provider_runtime(
                provider_id,
                cmd.command.clone(),
                cmd.args.clone(),
            )),
            Ok(None) => runtime_command_missing_adapter(provider_id),
            Err(err) => runtime_command_invalid_adapter(provider_id, err.to_string()),
        };
    }

    if is_acp_provider_id(provider_id) {
        let bridge_cmd = match runtime_command_as_agent_command_for_target(
            cfg,
            "acp-crp-bridge",
            Some(target),
        ) {
            Ok(cmd) => cmd,
            Err(err) => {
                return acp_status_adapter_bridge_invalid(
                    provider_id,
                    format!("invalid runtime command for acp-crp-bridge: {err}"),
                );
            }
        };
        let bridge_missing_message = "ACP bridge runtime is not configured".to_string();
        return match bridge_cmd.as_ref() {
            None => acp_status_adapter_bridge_missing(provider_id, bridge_missing_message),
            Some(bridge) => {
                match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
                    Ok(Some(cmd)) => {
                        match normalize_acp_provider_command(data_root, provider_id, cmd) {
                            Ok(cmd) => acp_bridge_adapter(provider_id, bridge, cmd),
                            Err(err) => acp_status_adapter_acp_command_invalid(
                                provider_id,
                                format!("invalid ACP command for provider '{provider_id}': {err}"),
                            ),
                        }
                    }
                    Ok(None) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("ACP command is not configured for provider '{provider_id}'"),
                    ),
                    Err(err) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("invalid ACP command for provider '{provider_id}': {err}"),
                    ),
                }
            }
        };
    }

    if provider_id == "fake" {
        return Arc::new(FakeProviderAdapter::new());
    }

    runtime_command_missing_adapter(provider_id)
}

pub async fn ensure_provider_adapter_for_target_with_cfg(
    state: &impl ProviderRuntimeHost,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if let Some(cache_key) = target_adapter_cache_key(provider_id, target) {
        if let Some(adapter) = state
            .target_provider_adapters()
            .lock()
            .await
            .get(&cache_key)
            .cloned()
        {
            return adapter;
        }
        let adapter =
            build_provider_adapter_for_target(state.data_root(), cfg, provider_id, target);
        state
            .target_provider_adapters()
            .lock()
            .await
            .insert(cache_key, adapter.clone());
        return adapter;
    }

    if let Some(adapter) = state
        .provider_adapters()
        .lock()
        .await
        .get(provider_id)
        .cloned()
    {
        return adapter;
    }
    let adapter = build_provider_adapter_for_target(state.data_root(), cfg, provider_id, target);
    state
        .provider_adapters()
        .lock()
        .await
        .insert(provider_id.to_string(), adapter.clone());
    adapter
}

pub async fn ensure_provider_adapter_for_target(
    state: &impl ProviderRuntimeHost,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    let cfg = installer::load_agent_server_config(state.data_root())
        .await
        .unwrap_or_default();
    ensure_provider_adapter_for_target_with_cfg(state, &cfg, provider_id, target).await
}

#[cfg(test)]
mod tests;
