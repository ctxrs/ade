use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, path::Path as StdPath};

use anyhow::Result;
use ctx_providers::adapters::{ProviderAdapter, ProviderHealth, ProviderStatus};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use which::which;

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

fn maybe_wrap_gemini_acp_command(
    data_root: &Path,
    mut cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    fn file_stem_matches(path: &StdPath, name: &str) -> bool {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(name))
            .unwrap_or(false)
    }

    fn resolve_path(value: &str) -> Option<PathBuf> {
        let path = StdPath::new(value);
        if value.contains(std::path::MAIN_SEPARATOR) || path.is_absolute() {
            return fs::canonicalize(path)
                .ok()
                .or_else(|| Some(path.to_path_buf()));
        }
        which(value).ok()
    }

    fn find_node_modules(path: &StdPath) -> Option<PathBuf> {
        for ancestor in path.ancestors() {
            if ancestor.file_name().and_then(|s| s.to_str()) == Some("node_modules") {
                return Some(ancestor.to_path_buf());
            }
        }
        if let Some(bin_dir) = path.parent() {
            if bin_dir.file_name().and_then(|s| s.to_str()) == Some("bin") {
                if let Some(prefix) = bin_dir.parent() {
                    let candidate = prefix.join("lib").join("node_modules");
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    fn is_gemini_cli_entrypoint(path: &StdPath) -> bool {
        if path.file_name().and_then(|s| s.to_str()) != Some("index.js") {
            return false;
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
                return true;
            }
        }
        false
    }

    let mut candidate = None;
    let cmd_path = StdPath::new(&cmd.command);
    let cmd_is_gemini = file_stem_matches(cmd_path, "gemini");
    let cmd_is_node = file_stem_matches(cmd_path, "node");
    if cmd_is_gemini || cmd_is_node {
        if cmd_is_gemini {
            candidate = resolve_path(&cmd.command);
        }
        if candidate.is_none() {
            if let Some(arg0) = cmd.args.first() {
                let arg0_path = StdPath::new(arg0);
                if file_stem_matches(arg0_path, "gemini") || is_gemini_cli_entrypoint(arg0_path) {
                    candidate = resolve_path(arg0);
                }
            }
        }
    } else if let Some(arg0) = cmd.args.first() {
        let arg0_path = StdPath::new(arg0);
        if file_stem_matches(arg0_path, "gemini") || is_gemini_cli_entrypoint(arg0_path) {
            candidate = resolve_path(arg0);
        }
    }

    let bin_path = match candidate {
        Some(path) => path,
        None => return cmd,
    };

    let node_modules_dir = match find_node_modules(&bin_path) {
        Some(dir) => dir,
        None => return cmd,
    };
    let cli_root = node_modules_dir.join("@google").join("gemini-cli");
    let core_root = node_modules_dir.join("@google").join("gemini-cli-core");
    if !cli_root.exists() || !core_root.exists() {
        return cmd;
    }

    let wrapper_path = data_root
        .join("providers")
        .join("agent-servers")
        .join("gemini-acp-wrapper.mjs");
    if let Some(parent) = wrapper_path.parent() {
        let _ = fs::create_dir_all(parent);
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
        core_root.join("dist").join("index.js").to_string_lossy(),
        cli_root.join("dist").join("index.js").to_string_lossy(),
    );

    let write_wrapper = match fs::read_to_string(&wrapper_path) {
        Ok(existing) => existing != wrapper_contents,
        Err(_) => true,
    };
    if write_wrapper {
        let _ = fs::write(&wrapper_path, wrapper_contents);
    }

    let wrapper_arg = wrapper_path.to_string_lossy().to_string();
    if cmd_is_gemini {
        let node_path = match which("node") {
            Ok(path) => path.to_string_lossy().to_string(),
            Err(_) => return cmd,
        };
        cmd.command = node_path;
        cmd.args.insert(0, wrapper_arg);
    } else if let Some(first) = cmd.args.first_mut() {
        *first = wrapper_arg;
    }
    cmd
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
) -> installer::AgentServerCommand {
    let cmd = if provider_id == "gemini" {
        maybe_wrap_gemini_acp_command(data_root, cmd)
    } else {
        cmd
    };
    let cmd = if provider_id == "qwen" {
        maybe_set_qwen_openai_auth_type(cmd)
    } else {
        cmd
    };
    if provider_id == "openhands" {
        maybe_set_bridge_env_override(cmd)
    } else {
        cmd
    }
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
    let normalized = normalize_acp_provider_command(data_root, provider_id, runtime_cmd);
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
                        let cmd = normalize_acp_provider_command(data_root, provider_id, cmd);
                        acp_bridge_adapter(provider_id, bridge, cmd)
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
        let normalized = normalize_acp_provider_command(temp.path(), "qwen", input);
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
}
