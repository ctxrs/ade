use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fs, path::Path};

use anyhow::{Context, Result};
use axum::Router;
use chrono::Utc;
use directories::BaseDirs;
use serde_json::json;
use which::which;

use ctx_core::models::SessionTurnStatus;
use ctx_lsp::LspManagerConfig;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::{StoreManager, StoreManagerConfig};

use crate::api;
use crate::installer;
use crate::memleak_debug;
use crate::provider_child_reclassifier;
use crate::provider_guard;
use crate::provider_restart;
use crate::provider_usage;
use crate::resource_governance;
use crate::resource_telemetry;
use crate::scheduler::reconcile_turn_terminal_state;
use crate::settings;
use crate::telemetry::TelemetryConfig;
use crate::tool_cgroup;

mod auth;
mod edit_plans;
mod sessions;
mod state;
mod workspaces;

pub use state::{
    AppState, CacheSweepConfig, CacheSweepStats, CachedFileCompletions, CachedProviderOptions,
    CachedProviderVerify, GitStatusSnapshotCacheEntry, SessionHeadCacheKey, TimedEntry,
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorktreeVcsSnapshotCacheEntry,
};

fn fallback_provider_command(command: &str, args: Vec<String>) -> installer::AgentServerCommand {
    installer::AgentServerCommand {
        command: command.to_string(),
        args,
        dependencies: Vec::new(),
        managed: None,
    }
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

fn acp_bridge_adapter(
    id: &str,
    bridge_cmd: &installer::AgentServerCommand,
    acp_cmd: installer::AgentServerCommand,
) -> Arc<dyn ProviderAdapter> {
    let acp_command = format_shell_command(&acp_cmd.command, &acp_cmd.args);
    let mut args = bridge_cmd.args.clone();
    args.push("--acp-command".to_string());
    args.push(acp_command);
    Arc::new(Tier1CrpAdapter::from_raw(
        id,
        bridge_cmd.command.clone(),
        args,
    ))
}

fn maybe_wrap_gemini_acp_command(
    data_root: &Path,
    mut cmd: installer::AgentServerCommand,
) -> installer::AgentServerCommand {
    fn file_stem_matches(path: &Path, name: &str) -> bool {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(name))
            .unwrap_or(false)
    }

    fn resolve_path(value: &str) -> Option<PathBuf> {
        let path = Path::new(value);
        if value.contains(std::path::MAIN_SEPARATOR) || path.is_absolute() {
            return fs::canonicalize(path)
                .ok()
                .or_else(|| Some(path.to_path_buf()));
        }
        which(value).ok()
    }

    fn find_node_modules(path: &Path) -> Option<PathBuf> {
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

    let mut candidate = None;
    let cmd_path = Path::new(&cmd.command);
    let cmd_is_gemini = file_stem_matches(cmd_path, "gemini");
    let cmd_is_node = file_stem_matches(cmd_path, "node");
    if cmd_is_gemini || cmd_is_node {
        if cmd_is_gemini {
            candidate = resolve_path(&cmd.command);
        }
        if candidate.is_none() {
            if let Some(arg0) = cmd.args.first() {
                let arg0_path = Path::new(arg0);
                if file_stem_matches(arg0_path, "gemini") {
                    candidate = resolve_path(arg0);
                }
            }
        }
    } else if let Some(arg0) = cmd.args.first() {
        let arg0_path = Path::new(arg0);
        if file_stem_matches(arg0_path, "gemini") {
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

async fn reconcile_running_turns(state: &Arc<AppState>) -> Result<()> {
    let workspaces = state.global_store().list_workspaces().await?;
    let mut running_turns = Vec::new();
    for workspace in workspaces {
        let store = state.store_for_workspace(workspace.id).await?;
        let mut turns = store
            .list_session_turns_by_statuses(&[SessionTurnStatus::Running])
            .await?;
        running_turns.append(&mut turns);
    }

    for turn in running_turns {
        if let Err(err) = reconcile_turn_terminal_state(
            state,
            turn.session_id,
            turn.run_id,
            turn.turn_id,
            "daemon_restart",
        )
        .await
        {
            tracing::warn!(
                session_id = %turn.session_id.0,
                turn_id = %turn.turn_id.0,
                err = %err,
                "failed to reconcile running turn after daemon restart"
            );
        }
    }

    Ok(())
}

fn spawn_cache_sweeper(state: Arc<AppState>) {
    let config = CacheSweepConfig::from_env();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(config.interval) => {
                    let stats = state.sweep_idle_caches(Instant::now(), config).await;
                    if stats.total_evicted() > 0 {
                        tracing::info!(
                            session_head_evicted = stats.session_head_evicted,
                            session_meta_evicted = stats.session_meta_evicted,
                            schedulers_evicted = stats.schedulers_evicted,
                            broadcasters_evicted = stats.broadcasters_evicted,
                            session_event_heads_evicted = stats.session_event_heads_evicted,
                            file_completions_evicted = stats.file_completions_evicted,
                            workspace_file_completions_evicted =
                                stats.workspace_file_completions_evicted,
                            git_status_evicted = stats.git_status_evicted,
                            workspace_snapshot_evicted = stats.workspace_snapshot_evicted,
                            workspace_heads_evicted = stats.workspace_heads_evicted,
                            worktree_bootstrap_evicted = stats.worktree_bootstrap_evicted,
                            workspace_stores_evicted = stats.workspace_stores_evicted,
                            "cache sweep completed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });
}

pub async fn serve(bind: String, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".ctx")
        }
    };
    tokio::fs::create_dir_all(&data_root).await?;
    // Canonicalize so Podman machine mount sources resolve under shared roots on macOS
    // (e.g. /tmp -> /private/tmp). This also reduces accidental duplicate state roots.
    let data_root = tokio::fs::canonicalize(&data_root)
        .await
        .unwrap_or(data_root);
    tokio::fs::create_dir_all(data_root.join("logs")).await.ok();

    let _daemon_lock = auth::acquire_daemon_lock(&data_root)?;

    let settings_data = settings::load_settings(&data_root).await;
    let store_config = settings_data
        .storage
        .as_ref()
        .map(|storage| StoreManagerConfig {
            max_connections: storage.max_connections,
        })
        .unwrap_or_default();
    let stores = StoreManager::open_with_config(&data_root, store_config).await?;

    // Retention policy (configurable via env):
    // - Keep tool summaries and final thoughts for archived tasks for N days.
    // - Do not retain thought chunk events (handled at ingestion time).
    const DEFAULT_TOOL_SUMMARY_RETENTION_DAYS: u64 = 30;
    fn tool_summary_retention_days() -> u64 {
        std::env::var("CTX_TOOL_SUMMARY_RETENTION_DAYS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_TOOL_SUMMARY_RETENTION_DAYS)
    }
    {
        let stores = stores.clone();
        tokio::spawn(async move {
            let mut last_cleanup = None::<String>;
            loop {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                if last_cleanup.as_deref() != Some(&today) {
                    let retention_days = tool_summary_retention_days();
                    match stores.global().list_workspaces().await {
                        Ok(workspaces) => {
                            for workspace in workspaces {
                                match stores.workspace(workspace.id).await {
                                    Ok(store) => {
                                        match store
                                            .prune_session_data_older_than_days(retention_days)
                                            .await
                                        {
                                            Ok(stats) => {
                                                tracing::info!(
                                                    workspace_id = %workspace.id.0,
                                                    tool_summaries_deleted = stats
                                                        .tool_summaries_deleted,
                                                    turn_thoughts_cleared = stats
                                                        .turn_thoughts_cleared,
                                                    retention_days,
                                                    "pruned archived session data",
                                                );
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    workspace_id = %workspace.id.0,
                                                    retention_days,
                                                    "failed to prune old session data: {err:#}",
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            workspace_id = %workspace.id.0,
                                            "failed to open workspace store for pruning: {err:#}",
                                        );
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                retention_days,
                                "failed to list workspaces for pruning: {err:#}",
                            );
                        }
                    }
                    last_cleanup = Some(today);
                }
                tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            }
        });
    }

    let agent_cfg = installer::load_agent_server_config(&data_root)
        .await
        .unwrap_or_default();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let bridge_cmd = installer::resolve_provider_command(&agent_cfg, "acp-crp-bridge")
        .unwrap_or_else(|| {
            let command = std::env::var("CTX_ACP_CRP_BRIDGE_CMD")
                .unwrap_or_else(|_| fallback_provider_command("acp-crp-bridge", vec![]).command);
            installer::AgentServerCommand {
                command,
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            }
        });
    let codex_crp_cmd = installer::resolve_provider_command(&agent_cfg, "codex-crp");
    let codex_crp_adapter: Arc<Tier1CrpAdapter> = Arc::new(match codex_crp_cmd.clone() {
        Some(cmd) => Tier1CrpAdapter::from_raw("codex-crp", cmd.command, cmd.args),
        None => Tier1CrpAdapter::codex(),
    });
    let codex_adapter: Arc<Tier1CrpAdapter> = Arc::new(match codex_crp_cmd {
        Some(cmd) => Tier1CrpAdapter::from_raw("codex", cmd.command, cmd.args),
        None => Tier1CrpAdapter::from_raw("codex", "codex-crp".to_string(), Vec::new()),
    });
    let claude_crp_cmd = installer::resolve_provider_command(&agent_cfg, "claude-crp");
    let claude_crp_adapter: Arc<Tier1CrpAdapter> = Arc::new(match claude_crp_cmd {
        Some(cmd) => Tier1CrpAdapter::from_raw("claude-crp", cmd.command, cmd.args),
        None => Tier1CrpAdapter::claude(),
    });

    let gemini_cmd =
        installer::resolve_provider_command(&agent_cfg, "gemini").unwrap_or_else(|| {
            fallback_provider_command("gemini", vec!["--experimental-acp".to_string()])
        });
    let gemini_cmd = maybe_wrap_gemini_acp_command(&data_root, gemini_cmd);
    let gemini_adapter = acp_bridge_adapter("gemini", &bridge_cmd, gemini_cmd);

    let qwen_cmd = installer::resolve_provider_command(&agent_cfg, "qwen").unwrap_or_else(|| {
        fallback_provider_command("qwen", vec!["--experimental-acp".to_string()])
    });
    let qwen_adapter = acp_bridge_adapter("qwen", &bridge_cmd, qwen_cmd);

    let opencode_cmd = installer::resolve_provider_command(&agent_cfg, "opencode")
        .unwrap_or_else(|| fallback_provider_command("opencode", vec!["acp".to_string()]));
    let opencode_adapter = acp_bridge_adapter("opencode", &bridge_cmd, opencode_cmd);

    let mistral_cmd = installer::resolve_provider_command(&agent_cfg, "mistral")
        .unwrap_or_else(|| fallback_provider_command("vibe-acp", vec![]));
    let mistral_adapter = acp_bridge_adapter("mistral", &bridge_cmd, mistral_cmd);

    let goose_cmd = installer::resolve_provider_command(&agent_cfg, "goose")
        .unwrap_or_else(|| fallback_provider_command("goose", vec!["acp".to_string()]));
    let goose_adapter = acp_bridge_adapter("goose", &bridge_cmd, goose_cmd);

    let kimi_cmd = installer::resolve_provider_command(&agent_cfg, "kimi")
        .unwrap_or_else(|| fallback_provider_command("kimi", vec!["--acp".to_string()]));
    let kimi_adapter = acp_bridge_adapter("kimi", &bridge_cmd, kimi_cmd);

    let auggie_cmd = installer::resolve_provider_command(&agent_cfg, "auggie")
        .unwrap_or_else(|| fallback_provider_command("auggie", vec!["--acp".to_string()]));
    let auggie_adapter = acp_bridge_adapter("auggie", &bridge_cmd, auggie_cmd);

    let cagent_cfg_path = installer::cagent_config_path(&data_root);
    if !cagent_cfg_path.exists() {
        if let Some(parent) = cagent_cfg_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let cfg = r#"agents:
  root:
    model: openai/gpt-5-mini
    description: ctx default agent
    instruction: |
      You are a helpful coding assistant.
"#;
        std::fs::write(&cagent_cfg_path, cfg).ok();
    }
    let mut cagent_cmd =
        installer::resolve_provider_command(&agent_cfg, "cagent").unwrap_or_else(|| {
            let cfg = cagent_cfg_path.to_string_lossy().to_string();
            fallback_provider_command("cagent", vec!["acp".to_string(), cfg])
        });
    let cfg_path_str = cagent_cfg_path.to_string_lossy().to_string();
    for arg in &mut cagent_cmd.args {
        if arg == "{{cagent_config}}" {
            *arg = cfg_path_str.clone();
        }
    }
    let cagent_adapter = acp_bridge_adapter("cagent", &bridge_cmd, cagent_cmd);

    providers.insert("codex-crp".into(), codex_crp_adapter);
    providers.insert("codex".into(), codex_adapter);
    providers.insert("claude-crp".into(), claude_crp_adapter);
    providers.insert("gemini".into(), gemini_adapter.clone());
    providers.insert("qwen".into(), qwen_adapter.clone());
    providers.insert("opencode".into(), opencode_adapter.clone());
    providers.insert("mistral".into(), mistral_adapter.clone());
    providers.insert("goose".into(), goose_adapter.clone());
    providers.insert("kimi".into(), kimi_adapter.clone());
    providers.insert("auggie".into(), auggie_adapter.clone());
    providers.insert("cagent".into(), cagent_adapter.clone());

    let extra_acp_bridge_providers: Vec<(&str, &str, Vec<String>)> = vec![
        ("amp", "amp-acp", vec![]),
        ("droid", "droid-acp", vec![]),
        ("copilot", "copilot-cli-acp", vec![]),
        ("kiro", "kiro-acp", vec![]),
        ("rovo", "rovo-dev-acp", vec![]),
        ("cody", "cody-acp", vec![]),
        ("continue", "cn", vec!["acp".to_string()]),
        ("cline", "cline-acp", vec![]),
        ("swe-agent", "sweagent", vec!["acp".to_string()]),
        ("openhands", "openhands", vec!["acp".to_string()]),
    ];
    for (id, fallback_cmd, fallback_args) in extra_acp_bridge_providers {
        let cmd = installer::resolve_provider_command(&agent_cfg, id)
            .unwrap_or_else(|| fallback_provider_command(fallback_cmd, fallback_args));
        let adapter = acp_bridge_adapter(id, &bridge_cmd, cmd);
        providers.insert(id.into(), adapter);
    }

    // codex/claude CRP adapters are always registered now (legacy ACP bridge removed).

    if std::env::var("CTX_SHOW_FAKE_PROVIDER").ok().as_deref() == Some("1") {
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    }

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let host = match local_addr.ip() {
        std::net::IpAddr::V4(ip) if ip.octets() == [0, 0, 0, 0] => "127.0.0.1".to_string(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => "::1".to_string(),
        ip => ip.to_string(),
    };
    let daemon_url = format!("http://{}:{}", host, local_addr.port());

    let mut auth = auth::load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());

    let mut lsp_cfg = LspManagerConfig::default();
    let _ = installer::apply_managed_lsp_server_config(&data_root, &mut lsp_cfg).await;
    let _ = installer::apply_user_lsp_server_config(&data_root, &mut lsp_cfg).await;
    auth.daemon_url = Some(daemon_url.clone());
    auth::write_daemon_auth_file(&auth::daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_lsp_config(
        data_root,
        stores,
        providers,
        daemon_url.clone(),
        auth_token,
        lsp_cfg,
    ));
    state.transport.web_sessions.clone().start_reaper().await;
    state.transport.terminals.clone().start_reaper().await;
    spawn_cache_sweeper(state.clone());
    if let Err(err) = reconcile_running_turns(&state).await {
        tracing::warn!(err = %err, "failed to reconcile running turns on startup");
    }
    let settings = settings::load_settings(&state.core.data_root).await;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = settings.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = settings
        .telemetry
        .as_ref()
        .map(|t| t.enabled)
        .unwrap_or(true);
    state
        .telemetry
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }

    if let Err(err) = tool_cgroup::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }

    resource_telemetry::spawn_resource_telemetry(state.clone());
    memleak_debug::spawn_memleak_debug(state.clone());
    provider_guard::spawn_provider_guard(state.clone());
    provider_restart::spawn_provider_restart(state.clone());
    provider_child_reclassifier::spawn_provider_child_reclassifier(state.clone());
    crate::merge_queue::spawn_merge_queue_runner(state.clone());
    provider_usage::spawn_provider_usage_poller(state.clone());

    // Reconnect managed mobile access tunnel on daemon start when enabled.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if state.core.auth_token.is_none() {
                return;
            }
            let cfg = match state.global_store().get_mobile_access_config().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("failed to read saved mobile access config: {e:#}");
                    return;
                }
            };
            let Some(cfg) = cfg else {
                return;
            };
            if !cfg.enabled {
                return;
            }

            let start_cfg = crate::mobile_tunnel::StartMobileTunnelConfig {
                relay_base_url: cfg.relay_base_url,
                tunnel_id: cfg.tunnel_id,
                tunnel_secret: cfg.tunnel_secret,
                public_base_url: cfg.public_base_url.trim_end_matches('/').to_string(),
                local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
            };
            if let Err(e) = state.transport.mobile_tunnel.start(start_cfg).await {
                tracing::warn!("failed to start saved mobile tunnel: {e:#}");
            }
        });
    }

    installer::refresh_provider_statuses(&state).await?;
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    let app: Router = api::router(state);

    tracing::info!("ctx daemon listening on {daemon_url}");
    println!("{}", json!({"event":"listening","url": daemon_url}));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
        })
        .await?;
    Ok(())
}

pub async fn init_workspace(root: Option<String>) -> Result<()> {
    let root_path = root
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("getting current dir")?);
    let vcs = ctx_fs::vcs::driver_for_path(&root_path).await?;
    vcs.assert_repo(&root_path).await?;

    let context_dir = root_path.join(".ctx");
    let pack_dir = context_dir.join("ctx-pack");
    let tmp_dir = pack_dir.join("tmp");

    tokio::fs::create_dir_all(pack_dir.join("specs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("prompts")).await?;
    tokio::fs::create_dir_all(pack_dir.join("docs")).await?;
    tokio::fs::create_dir_all(pack_dir.join("skills")).await?;
    tokio::fs::create_dir_all(&tmp_dir).await?;

    tokio::fs::create_dir_all(context_dir.join("exec-plans")).await?;

    let gitignore_path = root_path.join(".gitignore");
    let ignore_line = ".ctx/ctx-pack/tmp/";
    let mut gitignore = if gitignore_path.exists() {
        tokio::fs::read_to_string(&gitignore_path).await?
    } else {
        String::new()
    };
    if !gitignore.lines().any(|l| l.trim() == ignore_line) {
        if !gitignore.ends_with('\n') && !gitignore.is_empty() {
            gitignore.push('\n');
        }
        gitignore.push_str(ignore_line);
        gitignore.push('\n');
        tokio::fs::write(&gitignore_path, gitignore).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::models::VcsKind;
    use tempfile::tempdir;

    #[tokio::test]
    async fn sweeper_eviction_keeps_active_entries() {
        let temp = tempdir().unwrap();
        let stores = StoreManager::open(temp.path()).await.unwrap();
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
        let state = Arc::new(AppState::new(
            temp.path().to_path_buf(),
            stores.clone(),
            providers,
            "http://localhost".to_string(),
            None,
        ));

        let workspace = state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                temp.path().to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let store = state.store_for_workspace(workspace.id).await.unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                temp.path().to_string_lossy().to_string(),
                "deadbeef".to_string(),
                None,
            )
            .await
            .unwrap();
        let task = store
            .create_task(workspace.id, "task".to_string(), None)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                "fake".to_string(),
                "model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        {
            let mut cache = state.sessions.session_head_cache.lock().await;
            cache.insert(session.id, TimedEntry::new(HashMap::new()));
        }
        let _ = state.get_broadcaster(session.id).await;
        let _ = state.subscribe_session_event_head(session.id).await;
        let _ = state.ensure_scheduler(session.clone()).await;

        let now = Instant::now();
        {
            let mut cache = state.sessions.session_head_cache.lock().await;
            if let Some(entry) = cache.get_mut(&session.id) {
                entry.last_access = now - Duration::from_secs(3600);
            }
        }
        {
            let mut map = state.sessions.broadcasters.lock().await;
            if let Some(entry) = map.get_mut(&session.id) {
                entry.last_access = now;
            }
        }
        {
            let mut map = state.sessions.schedulers.lock().await;
            if let Some(entry) = map.get_mut(&session.id) {
                entry.last_access = now;
            }
        }

        let config = CacheSweepConfig {
            session_ttl: Duration::from_secs(60),
            workspace_ttl: Duration::from_secs(365 * 24 * 60 * 60),
            interval: Duration::from_secs(1),
        };
        let stats = state.sweep_idle_caches(now, config).await;
        assert_eq!(stats.session_head_evicted, 1);
        assert!(state
            .sessions
            .session_head_cache
            .lock()
            .await
            .get(&session.id)
            .is_none());
        assert!(state
            .sessions
            .broadcasters
            .lock()
            .await
            .get(&session.id)
            .is_some());
        assert!(state
            .sessions
            .schedulers
            .lock()
            .await
            .get(&session.id)
            .is_some());
    }
}
