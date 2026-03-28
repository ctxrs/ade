use super::*;
use async_trait::async_trait;
use ctx_core::models::VcsKind;
use ctx_providers::adapters::{
    ProviderCapabilities, ProviderHealth, ProviderProcessInfo, ProviderRestartMode, ProviderStatus,
    ProviderUsability, RunHandle, TurnInput,
};
use ctx_store::manager::WorkspaceStoreAccessKind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use tempfile::tempdir;

#[derive(Default)]
struct RecordingProviderAdapter {
    restart_calls: StdMutex<Vec<(String, ProviderRestartMode)>>,
}

impl RecordingProviderAdapter {
    fn restart_calls(&self) -> Vec<(String, ProviderRestartMode)> {
        self.restart_calls
            .lock()
            .expect("recording adapter restart lock")
            .clone()
    }
}

#[async_trait]
impl ProviderAdapter for RecordingProviderAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "recording".into(),
            installed: true,
            detected_path: None,
            version: Some("test".into()),
            capabilities: Some(ProviderCapabilities {
                stream_events: false,
                stream_format: "jsonl".into(),
                has_turn_boundaries: true,
                has_tool_call_ids: false,
                has_file_change_events: false,
                has_command_events: false,
                supports_resume: false,
                supports_stable_session_id: false,
                supports_fork_or_rewind: false,
                supports_headless: true,
                supports_server_mode: false,
                supports_interactive_tui: false,
                supports_private_state_dir: false,
                supports_sandbox_flags: false,
                supports_approval_flags: false,
                notes: Vec::new(),
            }),
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
    ) -> Result<RunHandle> {
        anyhow::bail!("not used in test");
    }

    async fn cancel(&self, _handle: RunHandle) -> Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        self.restart_calls
            .lock()
            .expect("recording adapter restart lock")
            .push((reason.to_string(), mode));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

#[cfg(target_os = "macos")]
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

#[cfg(target_os = "macos")]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(target_os = "macos")]
fn write_shared_vm_shutdown_helper(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let helper_path = dir.join("ctx-avf-linux-helper-daemon-shutdown-test.sh");
    let log_path = dir.join("ctx-avf-linux-helper-daemon-shutdown.log");
    let state_path = dir.join("ctx-avf-linux-helper-daemon-shutdown.state");
    let script = format!(
        r#"#!/bin/sh
LOG="{log}"
STATE="{state}"
printf '%s\n' "$*" >> "$LOG"
case "$1" in
  probe)
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","helper_version":"test","host_os":"darwin","host_arch":"arm64","supported":true,"save_restore_supported":true,"rosetta_supported":false,"notes":[]}}\n'
    ;;
  workspace-vm-state)
    if [ -f "$STATE" ] && [ "$(cat "$STATE")" = "stopped" ]; then
      printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"stopped","vm_root":"%s","logs_root":"%s","state_path":"%s/state.json","transition_status":"stopped","last_stop_outcome":"saved_state_written","simulated":true,"notes":["stopped"]}}\n' "$2" "$2" "$2"
    else
      printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"running","vm_root":"%s","logs_root":"%s","state_path":"%s/state.json","last_start_outcome":"restored","simulated":true,"notes":["running"]}}\n' "$2" "$2" "$2"
    fi
    ;;
  stop-workspace-vm)
    printf 'stopped' > "$STATE"
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"stopped","vm_root":"%s","logs_root":"%s","state_path":"%s/state.json","transition_status":"stopped","last_stop_outcome":"saved_state_written","simulated":true,"notes":["stopped"]}}\n' "$2" "$2" "$2"
    ;;
  *)
    echo "unexpected helper invocation: $*" >&2
    exit 1
    ;;
esac
"#,
        log = log_path.display(),
        state = state_path.display(),
    );
    std::fs::write(&helper_path, script).expect("write AVF helper shim");
    let mut perms = std::fs::metadata(&helper_path)
        .expect("helper shim metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&helper_path, perms).expect("chmod AVF helper shim");
    (helper_path, log_path)
}

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
            ctx_core::models::ExecutionEnvironment::Host,
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

#[tokio::test]
async fn sweeper_keeps_merge_queue_running_workspaces_resident() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        HashMap::new(),
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
    let _ = state.store_for_workspace(workspace.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 1);

    {
        let mut schedule_state = state.transport.merge_queue_state.lock().await;
        schedule_state.running.insert(workspace.id);
    }

    let config = CacheSweepConfig {
        session_ttl: Duration::from_secs(0),
        workspace_ttl: Duration::from_secs(0),
        interval: Duration::from_secs(30),
    };
    let _ = state.sweep_idle_caches(Instant::now(), config).await;
    assert_eq!(stores.stats().await.workspace_store_count, 1);

    {
        let mut schedule_state = state.transport.merge_queue_state.lock().await;
        schedule_state.running.remove(&workspace.id);
    }

    let _ = state.sweep_idle_caches(Instant::now(), config).await;
    assert_eq!(stores.stats().await.workspace_store_count, 0);
}

#[tokio::test]
async fn opening_workspace_does_not_evict_active_workspace_store() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open_with_config(
        temp.path(),
        StoreManagerConfig {
            max_cached_workspaces: 2,
            ..StoreManagerConfig::default()
        },
    )
    .await
    .unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace_a = state
        .global_store()
        .create_workspace(
            "ws-a".to_string(),
            temp.path().join("ws-a").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let workspace_b = state
        .global_store()
        .create_workspace(
            "ws-b".to_string(),
            temp.path().join("ws-b").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let workspace_c = state
        .global_store()
        .create_workspace(
            "ws-c".to_string(),
            temp.path().join("ws-c").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();

    let store_a = state.store_for_workspace(workspace_a.id).await.unwrap();
    let worktree_a = store_a
        .create_worktree(
            workspace_a.id,
            temp.path().join("wt-a").to_string_lossy().to_string(),
            "base-a".to_string(),
            None,
        )
        .await
        .unwrap();
    let task_a = store_a
        .create_task(workspace_a.id, "task-a".to_string(), None)
        .await
        .unwrap();
    let session_a = store_a
        .create_session(
            task_a.id,
            workspace_a.id,
            worktree_a.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let _ = state.ensure_scheduler(session_a).await;

    let _ = state.store_for_workspace(workspace_b.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 2);

    let _ = state.store_for_workspace(workspace_c.id).await.unwrap();
    assert_eq!(stores.stats().await.workspace_store_count, 2);

    let workspace_a_cached = state
        .core
        .stores
        .workspace_access(workspace_a.id)
        .await
        .unwrap();
    assert!(
        matches!(workspace_a_cached.kind, WorkspaceStoreAccessKind::Cached),
        "active workspace store should stay cached under the cap"
    );
    let workspace_b_reopened = state
        .core
        .stores
        .workspace_access(workspace_b.id)
        .await
        .unwrap();
    assert!(
        matches!(
            workspace_b_reopened.kind,
            WorkspaceStoreAccessKind::ColdOpen | WorkspaceStoreAccessKind::Reactivated
        ),
        "inactive workspace store should be the one evicted under the cap"
    );
}

#[tokio::test]
async fn reconcile_running_turns_does_not_cache_historical_workspace_stores() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    for idx in 0..4 {
        state
            .global_store()
            .create_workspace(
                format!("ws-{idx}"),
                temp.path()
                    .join(format!("ws-{idx}"))
                    .to_string_lossy()
                    .to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
    }

    reconcile_running_turns(&state).await.unwrap();

    let stats = stores.stats().await;
    assert_eq!(stats.workspace_store_count, 0);
}

#[tokio::test]
async fn prune_archived_session_data_does_not_cache_historical_workspace_stores() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();

    for idx in 0..4 {
        stores
            .global()
            .create_workspace(
                format!("ws-{idx}"),
                temp.path()
                    .join(format!("ws-{idx}"))
                    .to_string_lossy()
                    .to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
    }

    prune_archived_session_data_for_all_workspaces(&stores, 30)
        .await
        .unwrap();

    let stats = stores.stats().await;
    assert_eq!(stats.workspace_store_count, 0);
}

#[tokio::test]
async fn merge_queue_startup_runner_does_not_cache_historical_workspace_stores() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    for idx in 0..4 {
        state
            .global_store()
            .create_workspace(
                format!("ws-{idx}"),
                temp.path()
                    .join(format!("ws-{idx}"))
                    .to_string_lossy()
                    .to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
    }

    crate::merge_queue::spawn_merge_queue_runner(state);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = stores.stats().await;
    assert_eq!(stats.workspace_store_count, 0);
}

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
fn normalizes_openhands_command_with_env_override_flag() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/openhands".to_string(),
        args: vec!["acp".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized = normalize_acp_provider_command(temp.path(), "openhands", input)
        .expect("normalized openhands");
    assert_eq!(
        normalized.args,
        vec!["acp".to_string(), "--override-with-envs".to_string()]
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
    assert_eq!(resolved.args[2], "--acp-command");
    assert!(resolved
        .args
        .get(3)
        .is_some_and(|arg| arg.ends_with("/cursor-agent --experimental-acp")));
    assert_eq!(
        resolved.dependencies,
        vec!["bridge-dep".to_string(), "cursor-dep".to_string()]
    );
}

#[tokio::test]
async fn collect_provider_adapters_for_shutdown_includes_root_and_target_adapters() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let root_adapter = Arc::new(RecordingProviderAdapter::default());
    let target_adapter = Arc::new(RecordingProviderAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), root_adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    state
        .providers
        .target_adapters
        .lock()
        .await
        .insert("root@host".into(), target_adapter.clone());

    let adapters = collect_provider_adapters_for_shutdown(&state).await;
    let mut ids = adapters.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    ids.sort();

    assert_eq!(ids, vec!["root".to_string(), "root@host".to_string()]);
}

#[tokio::test]
async fn shutdown_provider_adapters_requests_immediate_restart_for_all_adapters() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let root_adapter = Arc::new(RecordingProviderAdapter::default());
    let target_adapter = Arc::new(RecordingProviderAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), root_adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    state
        .providers
        .target_adapters
        .lock()
        .await
        .insert("root@host".into(), target_adapter.clone());

    shutdown_provider_adapters(&state, "test shutdown").await;

    assert_eq!(
        root_adapter.restart_calls(),
        vec![("test shutdown".to_string(), ProviderRestartMode::Immediate)]
    );
    assert_eq!(
        target_adapter.restart_calls(),
        vec![("test shutdown".to_string(), ProviderRestartMode::Immediate)]
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn shutdown_shared_substrate_requests_save_or_stop_when_shared_backend_available() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    let (helper_path, log_path) = write_shared_vm_shutdown_helper(temp.path());
    let _helper_guard = EnvGuard::set(
        crate::workspace_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );

    let record = crate::daemon::lifecycle::shutdown_shared_substrate(&state, "test shutdown")
        .await
        .expect("shared substrate shutdown")
        .expect("shared substrate record");

    assert_eq!(
        record.shutdown_outcome,
        Some(crate::workspace_runtime::SubstrateShutdownOutcome::Saved)
    );
    assert_eq!(record.shutdown_reason, None);
    assert!(!record.save_error_present);
    assert!(record.saved_state_written_on_shutdown);

    let log = std::fs::read_to_string(log_path).expect("read helper log");
    assert!(
        log.lines()
            .any(|line| line == format!("stop-workspace-vm {}", temp.path().display())),
        "expected stop-workspace-vm invocation in log:\n{log}"
    );
}

#[test]
fn runtime_probe_command_keeps_native_crp_provider_unwrapped() {
    let temp = tempdir().unwrap();
    let codex_cmd = temp.path().join("codex");
    std::fs::write(&codex_cmd, b"codex").unwrap();
    let cfg = installer::AgentServerConfigFile {
        providers: HashMap::from([(
            "codex".to_string(),
            installer::AgentServerCommand {
                command: codex_cmd.to_string_lossy().to_string(),
                args: vec!["serve".to_string()],
                dependencies: vec!["codex-dep".to_string()],
                managed: None,
            },
        )]),
        provider_login_commands: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };

    let resolved = runtime_probe_command_as_agent_command(temp.path(), &cfg, "codex")
        .expect("probe command")
        .expect("runtime command");

    assert_eq!(
        PathBuf::from(&resolved.command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("codex")
    );
    assert_eq!(resolved.args, vec!["serve".to_string()]);
    assert_eq!(resolved.dependencies, vec!["codex-dep".to_string()]);
}
