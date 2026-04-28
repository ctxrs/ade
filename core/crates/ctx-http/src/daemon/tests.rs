use super::*;
use async_trait::async_trait;
use chrono::Utc;
use ctx_core::ids::{RunId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{ExecutionEnvironment, SessionTurn, SessionTurnStatus, VcsKind};
use ctx_managed_installs::ManagedInstallHost;
use ctx_providers::adapters::{
    ProviderCapabilities, ProviderHealth, ProviderProcessInfo, ProviderRestartMode,
    ProviderSessionSweepConfig, ProviderSessionSweepStats, ProviderStatus, ProviderUsability,
    RunHandle, TurnInput,
};
#[cfg(unix)]
use ctx_sandbox_container_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV;
use ctx_store::manager::WorkspaceStoreAccessKind;
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use tempfile::tempdir;

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

impl EnvVarGuard {
    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

fn sandbox_cli_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn daemon_public_base_url_env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[test]
fn daemon_public_base_url_from_env_accepts_http_or_https_origin_with_path_prefix() {
    let _guard = daemon_public_base_url_env_test_lock()
        .lock()
        .expect("daemon public base url env test lock");
    let _var = EnvVarGuard::set("CTX_DAEMON_PUBLIC_BASE_URL", "https://proxy.example/ctx/");
    assert_eq!(
        daemon_public_base_url_from_env().unwrap(),
        Some("https://proxy.example/ctx".to_string())
    );
}

#[test]
fn daemon_public_base_url_from_env_rejects_credentials_and_query_fragments() {
    let _guard = daemon_public_base_url_env_test_lock()
        .lock()
        .expect("daemon public base url env test lock");
    let _var = EnvVarGuard::set(
        "CTX_DAEMON_PUBLIC_BASE_URL",
        "https://user@example.com/ctx?a=1",
    );
    let err = daemon_public_base_url_from_env().unwrap_err();
    assert!(
        err.to_string().contains("must not embed credentials")
            || err
                .to_string()
                .contains("must not include query or fragment")
    );
}

#[test]
fn daemon_public_base_url_from_env_treats_absent_var_as_none() {
    let _guard = daemon_public_base_url_env_test_lock()
        .lock()
        .expect("daemon public base url env test lock");
    let _var = EnvVarGuard::remove("CTX_DAEMON_PUBLIC_BASE_URL");
    assert_eq!(daemon_public_base_url_from_env().unwrap(), None);
}

#[derive(Default)]
struct RecordingProviderAdapter {
    restart_calls: StdMutex<Vec<(String, ProviderRestartMode)>>,
    reap_calls: StdMutex<Vec<ProviderSessionSweepConfig>>,
    reap_result: StdMutex<ProviderSessionSweepStats>,
    pin_calls: StdMutex<Vec<(String, bool)>>,
}

impl RecordingProviderAdapter {
    fn restart_calls(&self) -> Vec<(String, ProviderRestartMode)> {
        self.restart_calls
            .lock()
            .expect("recording adapter restart lock")
            .clone()
    }

    fn reap_calls(&self) -> Vec<ProviderSessionSweepConfig> {
        self.reap_calls
            .lock()
            .expect("recording adapter reap lock")
            .clone()
    }

    fn set_reap_result(&self, stats: ProviderSessionSweepStats) {
        *self
            .reap_result
            .lock()
            .expect("recording adapter reap result lock") = stats;
    }

    fn pin_calls(&self) -> Vec<(String, bool)> {
        self.pin_calls
            .lock()
            .expect("recording adapter pin lock")
            .clone()
    }
}

#[derive(Default)]
struct BlockingInspectAdapter {
    inspect_started: AtomicBool,
    release_inspect: tokio::sync::Notify,
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
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> Result<RunHandle> {
        anyhow::bail!("not used in test");
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> Result<()> {
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

    async fn reap_idle_sessions(
        &self,
        config: ProviderSessionSweepConfig,
    ) -> Result<ProviderSessionSweepStats> {
        self.reap_calls
            .lock()
            .expect("recording adapter reap lock")
            .push(config);
        Ok(*self
            .reap_result
            .lock()
            .expect("recording adapter reap result lock"))
    }

    async fn set_session_pinned(&self, session_key: String, pinned: bool) -> Result<()> {
        self.pin_calls
            .lock()
            .expect("recording adapter pin lock")
            .push((session_key, pinned));
        Ok(())
    }
}

#[async_trait]
impl ProviderAdapter for BlockingInspectAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        self.inspect_started.store(true, Ordering::SeqCst);
        self.release_inspect.notified().await;
        Ok(ProviderStatus {
            provider_id: "blocking".into(),
            installed: true,
            detected_path: None,
            version: Some("test".into()),
            capabilities: None,
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
        _hooks: ctx_providers::adapters::ProviderRunHooks,
    ) -> Result<RunHandle> {
        anyhow::bail!("not used in test");
    }

    async fn cancel(&self, _handle: &mut RunHandle) -> Result<()> {
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        Vec::new()
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

async fn create_session_with_turn_status(
    state: &Arc<AppState>,
    root: &Path,
    environment: ExecutionEnvironment,
    status: SessionTurnStatus,
) -> (WorkspaceId, ctx_core::ids::SessionId) {
    let workspace = state
        .global_store()
        .create_workspace(
            format!("ws-{}", uuid::Uuid::new_v4()),
            root.join(format!("ws-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            root.join(format!("worktree-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
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
            environment,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    let now = Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id: TurnId::new(),
            session_id: session.id,
            run_id: Some(RunId::new()),
            user_message_id: None,
            status,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    (workspace.id, session.id)
}

#[tokio::test]
async fn startup_provider_status_refresh_runs_in_background() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let adapter = Arc::new(BlockingInspectAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("blocking".into(), adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));

    spawn_startup_provider_status_refresh(state.clone());

    tokio::time::timeout(Duration::from_secs(1), async {
        while !adapter.inspect_started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup refresh should begin inspect work");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        state.provider_statuses().lock().await.is_empty(),
        "provider status refresh should no longer block startup"
    );

    adapter.release_inspect.notify_waiters();

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if state
                .provider_statuses()
                .lock()
                .await
                .contains_key("blocking")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background provider status refresh should complete after inspect unblocks");
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

    assert!(
        state
            .transport
            .merge_queue
            .begin_workspace_drain(workspace.id)
            .await
    );

    let config = CacheSweepConfig {
        session_ttl: Duration::from_secs(0),
        workspace_ttl: Duration::from_secs(0),
        interval: Duration::from_secs(30),
    };
    let _ = state.sweep_idle_caches(Instant::now(), config).await;
    assert_eq!(stores.stats().await.workspace_store_count, 1);

    let _ = state
        .transport
        .merge_queue
        .finish_workspace_drain(workspace.id)
        .await;

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
async fn reconcile_running_turns_keeps_cached_workspace_store_usable() {
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
            temp.path().join("ws").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let cached_store = state.store_for_workspace(workspace.id).await.unwrap();
    cached_store
        .create_task(workspace.id, "before-reconcile".to_string(), None)
        .await
        .unwrap();

    reconcile_running_turns(&state).await.unwrap();

    let reopened = state.store_for_workspace(workspace.id).await.unwrap();
    reopened
        .create_task(workspace.id, "after-reconcile".to_string(), None)
        .await
        .expect("cached workspace store should remain usable after reconcile");
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
async fn prune_archived_session_data_keeps_cached_workspace_store_usable() {
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
            temp.path().join("ws").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let cached_store = state.store_for_workspace(workspace.id).await.unwrap();
    cached_store
        .create_task(workspace.id, "before-prune".to_string(), None)
        .await
        .unwrap();

    prune_archived_session_data_for_all_workspaces(&stores, 30)
        .await
        .unwrap();

    let reopened = state.store_for_workspace(workspace.id).await.unwrap();
    reopened
        .create_task(workspace.id, "after-prune".to_string(), None)
        .await
        .expect("cached workspace store should remain usable after prune");
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
        provider_login_executables: HashMap::new(),
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

#[tokio::test]
async fn running_state_updates_provider_worker_pin_once_per_transition() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let adapter = Arc::new(RecordingProviderAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    let session_id = ctx_core::ids::SessionId(uuid::Uuid::new_v4());

    state.set_running(session_id, true).await;
    state.set_running(session_id, true).await;
    state.set_running(session_id, false).await;
    state.set_running(session_id, false).await;

    assert_eq!(
        adapter.pin_calls(),
        vec![
            (session_id.0.to_string(), true),
            (session_id.0.to_string(), false),
        ]
    );
}

#[tokio::test]
async fn attachment_state_updates_provider_worker_pin_once_per_connection_lifecycle() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let adapter = Arc::new(RecordingProviderAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    let session_id = ctx_core::ids::SessionId(uuid::Uuid::new_v4());

    state.attach_session(session_id).await;
    state.attach_session(session_id).await;
    state.detach_session(session_id).await;
    state.detach_session(session_id).await;

    assert_eq!(
        adapter.pin_calls(),
        vec![
            (session_id.0.to_string(), true),
            (session_id.0.to_string(), false),
        ]
    );
}

#[tokio::test]
async fn running_and_attachment_leases_share_one_provider_pin_state() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let adapter = Arc::new(RecordingProviderAdapter::default());
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), adapter.clone());
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        providers,
        "http://localhost".to_string(),
        None,
    ));
    let session_id = ctx_core::ids::SessionId(uuid::Uuid::new_v4());

    state.set_running(session_id, true).await;
    state.attach_session(session_id).await;
    state.set_running(session_id, false).await;
    state.detach_session(session_id).await;

    assert_eq!(
        adapter.pin_calls(),
        vec![
            (session_id.0.to_string(), true),
            (session_id.0.to_string(), false),
        ]
    );
}

#[tokio::test]
async fn sweep_provider_workers_once_dedupes_shared_adapters_and_aggregates_stats() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let shared_adapter = Arc::new(RecordingProviderAdapter::default());
    shared_adapter.set_reap_result(ProviderSessionSweepStats {
        reaped: 1,
        skipped_busy: 2,
        dead_removed: 0,
        status_errors: 0,
    });
    let other_adapter = Arc::new(RecordingProviderAdapter::default());
    other_adapter.set_reap_result(ProviderSessionSweepStats {
        reaped: 0,
        skipped_busy: 0,
        dead_removed: 1,
        status_errors: 1,
    });

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("root".into(), shared_adapter.clone());
    providers.insert("other".into(), other_adapter.clone());
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
        .insert("root@host".into(), shared_adapter.clone());

    let config = ProviderSessionSweepConfig {
        idle_ttl: Duration::from_secs(7),
        max_idle_sessions: 3,
        interval: Duration::from_secs(11),
    };
    let stats = crate::daemon::lifecycle::sweep_provider_workers_once(&state, config).await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            skipped_busy: 2,
            dead_removed: 1,
            status_errors: 1,
        }
    );
    assert_eq!(shared_adapter.reap_calls().len(), 1);
    assert_eq!(other_adapter.reap_calls().len(), 1);
    assert_eq!(shared_adapter.reap_calls()[0].idle_ttl, config.idle_ttl);
    assert_eq!(
        shared_adapter.reap_calls()[0].max_idle_sessions,
        config.max_idle_sessions
    );
    assert_eq!(shared_adapter.reap_calls()[0].interval, config.interval);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn shutdown_shared_substrate_requests_save_or_stop_when_shared_backend_available() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
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
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );

    let record = crate::daemon::lifecycle::shutdown_shared_substrate(&state, "test shutdown")
        .await
        .expect("shared substrate shutdown")
        .expect("shared substrate record");

    assert_eq!(
        record.shutdown_outcome,
        Some(ctx_avf_linux_runtime::SubstrateShutdownOutcome::Saved)
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
        provider_login_executables: HashMap::new(),
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

/// Queued turns must remain queued after a daemon restart; reconcile_running_turns
/// should only interrupt turns that were actually Running.
#[tokio::test]
async fn reconcile_running_turns_leaves_queued_turns_queued() {
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
            temp.path().join("ws").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            temp.path().join("ws").to_string_lossy().to_string(),
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

    // Register the session in the global index so store_for_session can resolve it.
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

    let now = Utc::now();
    let queued_turn_id = TurnId::new();
    let queued_turn = SessionTurn {
        turn_id: queued_turn_id,
        session_id: session.id,
        run_id: None,
        user_message_id: None,
        status: SessionTurnStatus::Queued,
        start_seq: None,
        end_seq: None,
        started_at: now,
        updated_at: now,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    store.insert_session_turn(queued_turn).await.unwrap();

    let running_turn_id = TurnId::new();
    let running_turn = SessionTurn {
        turn_id: running_turn_id,
        session_id: session.id,
        run_id: Some(RunId::new()),
        user_message_id: None,
        status: SessionTurnStatus::Running,
        start_seq: Some(1),
        end_seq: None,
        started_at: now,
        updated_at: now,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    store.insert_session_turn(running_turn).await.unwrap();

    let activity = daemon_turn_activity_summary(&state).await.unwrap();
    assert!(!activity.idle);
    assert_eq!(activity.active_turn_count, 2);
    assert_eq!(activity.queued_turn_count, 1);
    assert_eq!(activity.running_turn_count, 1);

    reconcile_running_turns(&state).await.unwrap();

    let queued_after = store
        .get_session_turn(session.id, queued_turn_id)
        .await
        .unwrap()
        .expect("queued turn must still exist");
    assert_eq!(
        queued_after.status,
        SessionTurnStatus::Queued,
        "queued turn must remain queued after reconcile_running_turns"
    );

    let running_after = store
        .get_session_turn(session.id, running_turn_id)
        .await
        .unwrap()
        .expect("running turn must still exist");
    assert_eq!(
        running_after.status,
        SessionTurnStatus::Interrupted,
        "running turn must be interrupted after reconcile_running_turns"
    );

    let activity_after = daemon_turn_activity_summary(&state).await.unwrap();
    assert!(!activity_after.idle);
    assert_eq!(activity_after.active_turn_count, 1);
    assert_eq!(activity_after.queued_turn_count, 1);
    assert_eq!(activity_after.running_turn_count, 0);
}

#[tokio::test]
async fn update_drain_blocks_new_work_until_released() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    assert!(state
        .acquire_update_drain("test_update", "unit_test")
        .await
        .is_some());
    let err = state
        .reject_if_update_draining()
        .await
        .expect_err("drain should reject new work");
    assert!(err
        .to_string()
        .contains("daemon maintenance is in progress"));
    assert!(state.release_update_drain().await);
    state
        .reject_if_update_draining()
        .await
        .expect("released drain should allow work");
}

#[tokio::test]
async fn sandbox_work_activity_ignores_host_turns() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Host,
        SessionTurnStatus::Running,
    )
    .await;

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(!activity.active);
    assert_eq!(activity.active_sandbox_turn_count, 0);
    assert_eq!(activity.running_sandbox_turn_count, 0);
    assert!(activity.turns.is_empty());
}

#[tokio::test]
async fn sandbox_work_activity_counts_sandbox_turns() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Sandbox,
        SessionTurnStatus::Queued,
    )
    .await;
    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Sandbox,
        SessionTurnStatus::Running,
    )
    .await;

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.active_sandbox_turn_count, 2);
    assert_eq!(activity.queued_sandbox_turn_count, 1);
    assert_eq!(activity.running_sandbox_turn_count, 1);
    assert_eq!(activity.turns.len(), 2);
}

#[tokio::test]
async fn sandbox_work_activity_counts_runtime_operations() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _runtime_guard = state.execution.harness.begin_runtime_operation();

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.runtime_operation_count, 1);
}

#[tokio::test]
async fn sandbox_work_activity_counts_prewarm_operations() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _prewarm_guard = state.execution.harness.begin_prewarm_artifact_activity();

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.prewarm_artifact_operation_count, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_work_activity_counts_container_backed_terminals() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));
    let terminal = state
        .transport
        .terminals
        .create(crate::terminals::TerminalCreateRequest {
            workspace_id: WorkspaceId::new(),
            task_id: Some(TaskId::new()),
            session_id: None,
            worktree_id: Some(WorktreeId::new()),
            cwd: temp.path().to_path_buf(),
            shell: "/bin/sh".to_string(),
            cols: None,
            rows: None,
            env: HashMap::new(),
            native_container: Some(crate::terminals::NativeContainerTerminalSpec {
                cli_bin: PathBuf::from("/bin/sh"),
                cli_env: HashMap::new(),
                container_name: "ctx-harness-terminal".to_string(),
                workdir: "/workspace".to_string(),
                user: None,
            }),
            shared_vm_container: None,
        })
        .await
        .unwrap();

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert!(activity.running_container_backed_terminal);

    let _ = terminal.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_work_activity_counts_running_workspace_containers() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let cli_path = temp.path().join("sandbox-cli.sh");
    let log_path = temp.path().join("sandbox-cli.log");
    std::fs::write(
        &cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"ls\" ] && [ \"$3\" = \"--format\" ] && [ \"$4\" = \"{{{{.Names}}}}\" ]; then\n  printf 'ctx-harness-one\\nctx-harness-two\\npostgres\\n'\n  exit 0\nfi\necho \"unexpected invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _guard = EnvVarGuard::set(
        CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &cli_path.to_string_lossy(),
    );
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.running_workspace_container_count, 2);
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains("container ls --format {{.Names}}"),
        "expected running-container probe in log:\n{log}"
    );
}
