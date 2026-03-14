use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, path::Path as StdPath};

use anyhow::{Context, Result};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderProcessInfo, ProviderRestartMode, ProviderStatus,
    RunHandle, TurnInput,
};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;

use crate::daemon::AppState;
use crate::installer;
use crate::installs::InstallTarget;

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

    async fn cancel(&self, _handle: ctx_providers::adapters::RunHandle) -> Result<()> {
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

    async fn cancel(&self, handle: RunHandle) -> Result<()> {
        self.inner.cancel(handle).await
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        self.inner.list_processes().await
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        self.inner.restart(reason, mode).await
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

pub(crate) fn is_acp_provider_id(provider_id: &str) -> bool {
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

pub(crate) fn acp_bridge_command(
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

pub(crate) fn acp_bridge_adapter(
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
    let inner: Arc<dyn ProviderAdapter> =
        Arc::new(Tier1CrpAdapter::from_raw(id, bridged.command, bridged.args));
    if let Some(contract) = contract {
        return Arc::new(OpenHandsRuntimeContractAdapter::new(inner, contract));
    }
    inner
}

#[derive(Debug, Clone)]
pub(crate) struct ExplicitGeminiCliPaths {
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
    let path = StdPath::new(raw);
    anyhow::ensure!(
        path.is_absolute(),
        "{label} must be an explicit absolute path; got '{}'",
        raw
    );
    anyhow::ensure!(path.exists(), "{label} not found: {}", path.display());
    Ok(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn gemini_cli_root_from_entrypoint(path: &StdPath) -> Option<PathBuf> {
    if path.file_name().and_then(|s| s.to_str()) != Some("index.js") {
        return None;
    }
    if path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|s| s.to_str())
        != Some("dist")
    {
        return None;
    }
    for ancestor in path.ancestors() {
        if ancestor.file_name().and_then(|s| s.to_str()) != Some("gemini-cli") {
            continue;
        }
        let is_google_scope = ancestor
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|s| s.to_str())
            == Some("@google");
        if is_google_scope {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub(crate) fn resolve_explicit_gemini_cli_paths(
    command: &str,
    args: &[String],
) -> Result<ExplicitGeminiCliPaths> {
    let node_path = resolve_existing_absolute_path(command, "Gemini ACP runtime command")?;
    anyhow::ensure!(
        file_stem_matches(&node_path, "node"),
        "Gemini ACP runtime must use an explicit absolute node executable plus @google/gemini-cli/dist/index.js; got command '{}'",
        command
    );

    let arg0 = args.first().ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP runtime must pass an explicit absolute @google/gemini-cli/dist/index.js entrypoint as the first argument"
        )
    })?;
    let cli_entry_path = resolve_existing_absolute_path(arg0, "Gemini ACP entrypoint")?;
    let cli_root = gemini_cli_root_from_entrypoint(&cli_entry_path).ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP entrypoint must point to @google/gemini-cli/dist/index.js; got '{}'",
            cli_entry_path.display()
        )
    })?;
    let node_modules_dir = cli_root.parent().and_then(|scope| scope.parent()).ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP entrypoint must live under a node_modules/@google/gemini-cli install tree: {}",
            cli_entry_path.display()
        )
    })?;
    let core_entry_path = node_modules_dir
        .join("@google")
        .join("gemini-cli-core")
        .join("dist")
        .join("index.js");
    anyhow::ensure!(
        core_entry_path.exists(),
        "Gemini ACP companion package is missing: {}",
        core_entry_path.display()
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

    let wrapper_arg = wrapper_path.to_string_lossy().to_string();
    let first = cmd.args.first_mut().ok_or_else(|| {
        anyhow::anyhow!(
            "Gemini ACP runtime must pass an explicit absolute @google/gemini-cli/dist/index.js entrypoint as the first argument"
        )
    })?;
    *first = wrapper_arg;
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

pub(crate) fn normalize_acp_provider_command(
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

pub(crate) fn runtime_probe_command_as_agent_command_for_target(
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
            Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_raw(
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

pub(crate) async fn ensure_provider_adapter_for_target_with_cfg(
    state: &AppState,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if let Some(cache_key) = target_adapter_cache_key(provider_id, target) {
        if let Some(adapter) = state
            .providers
            .target_adapters
            .lock()
            .await
            .get(&cache_key)
            .cloned()
        {
            return adapter;
        }
        let adapter =
            build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
        state
            .providers
            .target_adapters
            .lock()
            .await
            .insert(cache_key, adapter.clone());
        return adapter;
    }

    if let Some(adapter) = state
        .providers
        .adapters
        .lock()
        .await
        .get(provider_id)
        .cloned()
    {
        return adapter;
    }
    let adapter =
        build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
    state
        .providers
        .adapters
        .lock()
        .await
        .insert(provider_id.to_string(), adapter.clone());
    adapter
}

pub(crate) async fn ensure_provider_adapter_for_target(
    state: &AppState,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    let cfg = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    ensure_provider_adapter_for_target_with_cfg(state, &cfg, provider_id, target).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn normalizes_qwen_command_with_openai_auth_type() {
        let temp = tempdir().unwrap();
        let input = installer::AgentServerCommand {
            command: "/tmp/qwen".to_string(),
            args: vec!["--experimental-acp".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };
        let normalized =
            normalize_acp_provider_command(temp.path(), "qwen", input).expect("normalized qwen");
        assert_eq!(
            normalized.args,
            vec![
                "--experimental-acp".to_string(),
                "--auth-type".to_string(),
                "openai".to_string(),
            ]
        );
    }

    #[test]
    fn normalizes_goose_command_with_acp_and_developer_builtin() {
        let temp = tempdir().unwrap();
        let input = installer::AgentServerCommand {
            command: "/tmp/goose".to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        };
        let normalized =
            normalize_acp_provider_command(temp.path(), "goose", input).expect("normalized goose");
        assert_eq!(
            normalized.args,
            vec![
                "acp".to_string(),
                "--with-builtin".to_string(),
                "developer".to_string(),
            ]
        );
    }

    #[test]
    fn preserves_existing_goose_developer_builtin() {
        let temp = tempdir().unwrap();
        let input = installer::AgentServerCommand {
            command: "/tmp/goose".to_string(),
            args: vec![
                "acp".to_string(),
                "--with-builtin".to_string(),
                "developer,computercontroller".to_string(),
            ],
            dependencies: Vec::new(),
            managed: None,
        };
        let normalized =
            normalize_acp_provider_command(temp.path(), "goose", input).expect("normalized goose");
        assert_eq!(
            normalized.args,
            vec![
                "acp".to_string(),
                "--with-builtin".to_string(),
                "developer,computercontroller".to_string(),
            ]
        );
    }

    #[test]
    fn classifies_upstream_openhands_runtime_contract() {
        let cmd = installer::AgentServerCommand {
            command: "/tmp/openhands".to_string(),
            args: vec!["acp".to_string(), "--override-with-envs".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };

        assert_eq!(
            openhands_runtime_contract_for_command(&cmd),
            OpenHandsRuntimeContract::UpstreamAcp
        );
    }

    #[test]
    fn classifies_legacy_openhands_shim_runtime_contract() {
        let cmd = installer::AgentServerCommand {
            command: "/tmp/openhands-acp.js".to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        };

        assert_eq!(
            openhands_runtime_contract_for_command(&cmd),
            OpenHandsRuntimeContract::ShimAcp
        );
    }

    #[tokio::test]
    async fn openhands_bridge_adapter_inspect_surfaces_runtime_contract_details() {
        let temp = tempdir().unwrap();
        let bridge_cmd = temp.path().join("acp-crp-bridge");
        std::fs::write(&bridge_cmd, b"bridge").unwrap();

        let adapter = acp_bridge_adapter(
            "openhands",
            &installer::AgentServerCommand {
                command: bridge_cmd.to_string_lossy().to_string(),
                args: vec!["--stdio".to_string()],
                dependencies: Vec::new(),
                managed: None,
            },
            installer::AgentServerCommand {
                command: "/tmp/openhands-acp.js".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            },
        );

        let status = adapter.inspect().await.expect("inspect status");
        assert_eq!(
            status
                .details
                .get("openhands_runtime_contract")
                .map(String::as_str),
            Some("shim_acp")
        );
        assert_eq!(
            status
                .details
                .get("openhands_real_runtime")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            status
                .details
                .get("openhands_runtime_contract_note")
                .map(String::as_str),
            Some("runtime command still points at the legacy `openhands-acp` shim")
        );
    }

    fn create_gemini_runtime_layout(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let node_bin = root
            .join("bundle")
            .join("runtimes")
            .join("node")
            .join("bin")
            .join("node");
        let cli_entry = root
            .join("bundle")
            .join("providers")
            .join("gemini")
            .join("node_modules")
            .join("@google")
            .join("gemini-cli")
            .join("dist")
            .join("index.js");
        let core_entry = root
            .join("bundle")
            .join("providers")
            .join("gemini")
            .join("node_modules")
            .join("@google")
            .join("gemini-cli-core")
            .join("dist")
            .join("index.js");
        std::fs::create_dir_all(node_bin.parent().unwrap()).unwrap();
        std::fs::create_dir_all(cli_entry.parent().unwrap()).unwrap();
        std::fs::create_dir_all(core_entry.parent().unwrap()).unwrap();
        std::fs::write(&node_bin, b"node").unwrap();
        std::fs::write(&cli_entry, b"cli").unwrap();
        std::fs::write(&core_entry, b"core").unwrap();
        (node_bin, cli_entry, core_entry)
    }

    #[test]
    fn wraps_explicit_gemini_node_entrypoint_for_acp() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        let (node_bin, cli_entry, core_entry) = create_gemini_runtime_layout(temp.path());

        let input = installer::AgentServerCommand {
            command: node_bin.to_string_lossy().to_string(),
            args: vec![
                cli_entry.to_string_lossy().to_string(),
                "--experimental-acp".to_string(),
            ],
            dependencies: Vec::new(),
            managed: None,
        };
        let wrapped =
            normalize_acp_provider_command(&data_root, "gemini", input).expect("wrapped gemini");

        assert_eq!(wrapped.command, node_bin.to_string_lossy().to_string());
        assert_eq!(
            wrapped.args.get(1).map(String::as_str),
            Some("--experimental-acp")
        );
        let wrapper_arg = wrapped.args.first().expect("wrapper arg");
        assert!(wrapper_arg.ends_with("gemini-acp-wrapper.mjs"));
        let wrapper_path = PathBuf::from(wrapper_arg);
        assert!(wrapper_path.exists());
        let wrapper_body = std::fs::read_to_string(wrapper_path).unwrap();
        assert!(wrapper_body.contains("GEMINI_CLI_NO_RELAUNCH"));
        assert!(wrapper_body.contains(cli_entry.to_string_lossy().as_ref()));
        assert!(wrapper_body.contains(core_entry.to_string_lossy().as_ref()));
    }

    #[test]
    fn rejects_path_style_gemini_command() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        let gemini_bin = temp.path().join("bundle").join("bin").join("gemini");
        std::fs::create_dir_all(gemini_bin.parent().unwrap()).unwrap();
        std::fs::write(&gemini_bin, b"gemini").unwrap();

        let input = installer::AgentServerCommand {
            command: gemini_bin.to_string_lossy().to_string(),
            args: vec!["--experimental-acp".to_string()],
            dependencies: Vec::new(),
            managed: None,
        };
        let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

        assert!(err
            .to_string()
            .contains("must use an explicit absolute node executable"));
    }

    #[test]
    fn rejects_relative_gemini_entrypoint() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        let (node_bin, _, _) = create_gemini_runtime_layout(temp.path());

        let input = installer::AgentServerCommand {
            command: node_bin.to_string_lossy().to_string(),
            args: vec![
                "node_modules/@google/gemini-cli/dist/index.js".to_string(),
                "--experimental-acp".to_string(),
            ],
            dependencies: Vec::new(),
            managed: None,
        };
        let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

        assert!(err
            .to_string()
            .contains("Gemini ACP entrypoint must be an explicit absolute path"));
    }

    #[test]
    fn rejects_gemini_runtime_when_core_package_is_missing() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        let (node_bin, cli_entry, core_entry) = create_gemini_runtime_layout(temp.path());
        std::fs::remove_file(core_entry).unwrap();

        let input = installer::AgentServerCommand {
            command: node_bin.to_string_lossy().to_string(),
            args: vec![
                cli_entry.to_string_lossy().to_string(),
                "--experimental-acp".to_string(),
            ],
            dependencies: Vec::new(),
            managed: None,
        };
        let err = normalize_acp_provider_command(&data_root, "gemini", input).unwrap_err();

        assert!(err
            .to_string()
            .contains("Gemini ACP companion package is missing"));
    }

    #[test]
    fn runtime_probe_command_wraps_acp_provider_with_bridge() {
        let temp = tempdir().unwrap();
        let cursor_cmd = temp.path().join("cursor-agent");
        let bridge_cmd = temp.path().join("acp-crp-bridge");
        std::fs::write(&cursor_cmd, b"cursor").unwrap();
        std::fs::write(&bridge_cmd, b"bridge").unwrap();
        let cfg = installer::AgentServerConfigFile {
            providers: HashMap::from([
                (
                    "cursor".to_string(),
                    installer::AgentServerCommand {
                        command: cursor_cmd.to_string_lossy().to_string(),
                        args: vec!["--experimental-acp".to_string()],
                        dependencies: vec!["cursor-dep".to_string()],
                        managed: None,
                    },
                ),
                (
                    "acp-crp-bridge".to_string(),
                    installer::AgentServerCommand {
                        command: bridge_cmd.to_string_lossy().to_string(),
                        args: vec!["--log-level".to_string(), "debug".to_string()],
                        dependencies: vec!["bridge-dep".to_string()],
                        managed: None,
                    },
                ),
            ]),
            provider_login_commands: HashMap::new(),
            managed_installs: HashMap::new(),
            managed_provider_targets: HashMap::new(),
            managed_install_targets: HashMap::new(),
        };

        let resolved = runtime_probe_command_as_agent_command(temp.path(), &cfg, "cursor")
            .expect("probe command")
            .expect("runtime command");

        assert_eq!(
            PathBuf::from(&resolved.command)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("acp-crp-bridge")
        );
        assert_eq!(resolved.args.len(), 4);
        assert_eq!(resolved.args[0], "--log-level");
        assert_eq!(resolved.args[1], "debug");
        assert_eq!(resolved.dependencies, vec!["bridge-dep", "cursor-dep"]);
    }

    #[test]
    fn runtime_probe_command_rejects_path_style_gemini_runtime() {
        let temp = tempdir().unwrap();
        let gemini_bin = temp.path().join("bundle").join("bin").join("gemini");
        let bridge_cmd = temp.path().join("acp-crp-bridge");
        std::fs::create_dir_all(gemini_bin.parent().unwrap()).unwrap();
        std::fs::write(&gemini_bin, b"gemini").unwrap();
        std::fs::write(&bridge_cmd, b"bridge").unwrap();
        let cfg = installer::AgentServerConfigFile {
            providers: HashMap::from([
                (
                    "gemini".to_string(),
                    installer::AgentServerCommand {
                        command: gemini_bin.to_string_lossy().to_string(),
                        args: vec!["--experimental-acp".to_string()],
                        dependencies: Vec::new(),
                        managed: None,
                    },
                ),
                (
                    "acp-crp-bridge".to_string(),
                    installer::AgentServerCommand {
                        command: bridge_cmd.to_string_lossy().to_string(),
                        args: vec!["--log-level".to_string(), "debug".to_string()],
                        dependencies: Vec::new(),
                        managed: None,
                    },
                ),
            ]),
            provider_login_commands: HashMap::new(),
            managed_installs: HashMap::new(),
            managed_provider_targets: HashMap::new(),
            managed_install_targets: HashMap::new(),
        };

        let err = runtime_probe_command_as_agent_command(temp.path(), &cfg, "gemini").unwrap_err();
        assert!(err
            .to_string()
            .contains("must use an explicit absolute node executable"));
    }
}
