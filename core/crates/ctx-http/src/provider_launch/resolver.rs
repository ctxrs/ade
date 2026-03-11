use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, path::Path as StdPath};

use anyhow::{Context, Result};
use ctx_providers::adapters::{ProviderAdapter, ProviderHealth, ProviderStatus};
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
    let bridged = acp_bridge_command(bridge_cmd, acp_cmd);
    Arc::new(Tier1CrpAdapter::from_raw(id, bridged.command, bridged.args))
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
