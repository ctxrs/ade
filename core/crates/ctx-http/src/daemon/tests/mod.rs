use super::*;
use async_trait::async_trait;
use chrono::Utc;
use ctx_core::ids::{RunId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, SessionEventType, SessionTurn, SessionTurnStatus, VcsKind,
};
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
use std::time::{Duration, Instant};
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
    crate::test_support::sandbox_cli_env_test_lock()
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

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
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

mod cache_store;
mod provider_commands;
mod provider_shutdown;
mod reconcile;
mod sandbox_work_activity;
mod update_drain;
