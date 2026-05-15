use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ctx_core::ids::{
    ConnectionProfileId, MergeQueueEntryId, MergeQueueRunId, MessageId, MobileDeviceId, RunId,
    SessionEventId, SessionId, TaskId, TerminalId, TurnId, WorkspaceAttachmentId, WorkspaceId,
    WorktreeId,
};
use ctx_core::models::{
    ArchiveVisibility, Artifact, ExecutionEnvironment, MergeQueueEntry, MergeQueueEntryStatus,
    MergeQueuePatchSource, MergeQueueRun, MergeQueueRunStatus, Message, MessageDelivery,
    MessageRole, MobileConnectionProfile, MobileDeviceRegistration, RetentionPolicyRef,
    RunArchiveState, RunRecord, RunStatus, SandboxBinding, SandboxGuestIdentity, SandboxProfile,
    SandboxSubstrate, Session, SessionEvent, SessionEventType, SessionHeadDelta,
    SessionHeadSnapshot, SessionSummary, SessionTurn, SessionTurnStatus, Task, VcsKind, Workspace,
    WorkspaceActiveTaskSummary, WorkspaceAttachmentStatus, Worktree, WorktreeAttachmentMount,
    WorktreeBootstrapStatus, WorktreeVcsSnapshot,
};
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::{provider_usage, CachedProviderOptions, CachedProviderVerify};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_settings_model::{ExecutionSettings, Settings};
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};
use ctx_store::{Store, StoreManager, WorktreeBootstrapResultUpdate};
use sha2::Digest;
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::Mutex as AsyncMutex;

use crate::daemon::{self, AppRuntimeFlags, DaemonHandle, DaemonState};

#[derive(Clone)]
pub struct TestDaemon {
    state: Arc<DaemonState>,
}

pub struct TestMobileAccessForTest<'a> {
    state: &'a Arc<DaemonState>,
}

pub struct CtxUiSizedHeadSeedSpec {
    pub turn_count: i64,
    pub message_count: i64,
    pub tool_count: i64,
    pub event_count: i64,
    pub tool_output_bytes: usize,
}

pub struct CtxUiSizedHeadSeedStats {
    pub event_count: i64,
    pub tool_count: i64,
    pub message_count: i64,
}

pub struct CtxUiSizedToolSummaryProbe {
    pub latest_turn_id: TurnId,
    pub bounded_tool_count: usize,
    pub oldest_loaded_order_seq: i64,
}

pub struct TaskDefaultSessionSnapshot {
    pub task: Option<Task>,
    pub sessions: Vec<Session>,
    pub task_count: usize,
    pub worktree_count: usize,
}

pub struct TaskSessionCreationLockGuardForTest {
    _lock: Arc<tokio::sync::Mutex<()>>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

pub struct ShutdownRunningTurnFixture {
    pub workspace_id: WorkspaceId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
}

pub struct RunArchiveRouteFixture {
    pub workspace_id: WorkspaceId,
    pub run_id: RunId,
}

pub struct AssistantChunkStreamSnapshot {
    pub events: Vec<SessionEvent>,
    pub turns: Vec<SessionTurn>,
}

pub struct TerminalTurnPersistenceSnapshot {
    pub turn: SessionTurn,
    pub events: Vec<SessionEvent>,
    pub assistant_messages: Vec<Message>,
}

pub struct TurnReconciliationSnapshot {
    pub turn: SessionTurn,
    pub events: Vec<SessionEvent>,
    pub last_turn_status: Option<SessionTurnStatus>,
    pub is_working: bool,
}

pub struct GlobalIdRoutingWorkspaceSessionSeed {
    pub name: String,
    pub root_path: PathBuf,
    pub base_commit: String,
    pub provider_id: String,
    pub model_id: String,
}

pub struct GlobalIdRoutingSessionFixture {
    pub session_id: SessionId,
}

pub struct TaskLifecycleWorktreeSeed {
    pub workspace_id: WorkspaceId,
    pub owner_task_id: TaskId,
    pub worktree_id: WorktreeId,
    pub root_path: PathBuf,
    pub base_commit: String,
    pub git_branch: String,
    pub make_primary: bool,
}

pub struct TaskLifecycleSandboxBindingSeed {
    pub worktree_id: WorktreeId,
    pub workspace_id: WorkspaceId,
    pub substrate: SandboxSubstrate,
    pub live_workspace_root: String,
    pub live_worktree_root: String,
    pub execution_settings_json: Option<String>,
    pub container_name: Option<String>,
    pub host_materialization_root: Option<PathBuf>,
}

pub struct TaskLifecycleSessionSeed {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub execution_environment: ExecutionEnvironment,
    pub title: String,
    pub parent_session_id: Option<SessionId>,
    pub role: Option<String>,
}

pub struct TaskLifecycleSnapshot {
    pub task: Option<Task>,
    pub worktree: Option<Worktree>,
    pub worktree_index_workspace_id: Option<WorkspaceId>,
    pub sandbox_binding: Option<SandboxBinding>,
}

struct CtxUiTurnSeed {
    index: i64,
    run_id: String,
    turn_id: String,
    started_at: String,
    start_seq: i64,
    end_seq: Option<i64>,
    status: &'static str,
    tool_total: i64,
}

const CTX_UI_SIZED_SEED_BATCH: i64 = 500;

fn fixed_test_utc(offset_seconds: i64) -> chrono::DateTime<chrono::Utc> {
    let base = chrono::DateTime::from_timestamp(1735689600, 0)
        .expect("fixed test timestamp should be valid");
    base + chrono::Duration::seconds(offset_seconds)
}

fn build_ctx_ui_sized_turns(seed: &CtxUiSizedHeadSeedSpec) -> Vec<CtxUiTurnSeed> {
    let started_at = chrono::Utc::now();
    let tool_turn_start = (seed.turn_count - 60).max(0);
    let tools_per_turn = seed.tool_count / 60;
    let tool_remainder = seed.tool_count % 60;

    (0..seed.turn_count)
        .map(|index| {
            let start_seq = 1 + (index * seed.event_count / seed.turn_count);
            let tool_total = if index < tool_turn_start {
                0
            } else {
                let offset = index - tool_turn_start;
                tools_per_turn + if offset < tool_remainder { 1 } else { 0 }
            };
            CtxUiTurnSeed {
                index,
                run_id: RunId::new().0.to_string(),
                turn_id: TurnId::new().0.to_string(),
                started_at: (started_at + chrono::Duration::milliseconds(index)).to_rfc3339(),
                start_seq,
                end_seq: if index + 1 == seed.turn_count {
                    None
                } else {
                    Some(start_seq + 1)
                },
                status: if index + 1 == seed.turn_count {
                    "running"
                } else {
                    "completed"
                },
                tool_total,
            }
        })
        .collect()
}

async fn insert_ctx_ui_sized_turns(
    store: &Store,
    session_id: &str,
    turns: &[CtxUiTurnSeed],
) -> anyhow::Result<()> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO session_turns (
            turn_id, session_id, run_id, user_message_id, status, start_seq, end_seq,
            started_at, updated_at, assistant_partial, thought_partial, metrics_json,
            tool_total, tool_pending, tool_running, tool_completed, tool_failed
        ) "#,
    );
    builder.push_values(turns, |mut values, row| {
        values
            .push_bind(&row.turn_id)
            .push_bind(session_id)
            .push_bind(&row.run_id)
            .push_bind(Option::<String>::None)
            .push_bind(row.status)
            .push_bind(row.start_seq)
            .push_bind(row.end_seq)
            .push_bind(&row.started_at)
            .push_bind(&row.started_at)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(row.tool_total)
            .push_bind(0_i64)
            .push_bind(if row.status == "running" {
                1_i64
            } else {
                0_i64
            })
            .push_bind(if row.status == "running" {
                0_i64
            } else {
                row.tool_total
            })
            .push_bind(0_i64);
    });
    builder.build().execute(store.pool()).await?;
    Ok(())
}

async fn seed_ctx_ui_sized_events(
    store: &Store,
    session_id: &str,
    seed: &CtxUiSizedHeadSeedSpec,
    turns: &[CtxUiTurnSeed],
) -> anyhow::Result<()> {
    let mut event_seq = 1_i64;
    while event_seq <= seed.event_count {
        let end = (event_seq + CTX_UI_SIZED_SEED_BATCH - 1).min(seed.event_count);
        let rows = (event_seq..=end).collect::<Vec<_>>();
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_events (
                seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, seq| {
            let turn_index =
                ((*seq - 1) * seed.turn_count / seed.event_count).clamp(0, seed.turn_count - 1);
            let turn = &turns[turn_index as usize];
            let event_type = match *seq % 11 {
                0 => "tool_call",
                1 => "tool_result",
                2 => "assistant_message_inserted",
                3 => "assistant_complete",
                _ => "notice",
            };
            let payload = ctx_ui_sized_event_payload(seed, turn, *seq, event_type);
            values
                .push_bind(*seq)
                .push_bind(uuid::Uuid::new_v4().to_string())
                .push_bind(session_id)
                .push_bind(&turn.run_id)
                .push_bind(&turn.turn_id)
                .push_bind(event_type)
                .push_bind(payload.to_string())
                .push_bind(if *seq % 17 == 0 { 1_i64 } else { 0_i64 })
                .push_bind(&turn.started_at);
        });
        builder.build().execute(store.pool()).await?;
        event_seq = end + 1;
    }
    Ok(())
}

fn ctx_ui_sized_event_payload(
    seed: &CtxUiSizedHeadSeedSpec,
    turn: &CtxUiTurnSeed,
    seq: i64,
    event_type: &str,
) -> serde_json::Value {
    if turn.index + 1 == seed.turn_count && matches!(event_type, "tool_call" | "tool_result") {
        return serde_json::json!({
            "kind": "ctx_ui_sized_fixture",
            "seq": seq,
            "turn_index": turn.index,
            "tool_call_id": format!("ctx-ui-live-tool-{seq}"),
            "order_seq": seed.tool_count + seq,
            "title": format!("Live fixture command {seq}"),
            "status": if event_type == "tool_result" { "completed" } else { "pending" },
            "rawInput": {
                "cmd": "printf ctx-ui-live-fixture",
                "seq": seq,
            },
            "output_text": format!("live fixture output {seq}"),
        });
    }

    serde_json::json!({
        "kind": "ctx_ui_sized_fixture",
        "seq": seq,
        "turn_index": turn.index,
    })
}

async fn seed_ctx_ui_sized_messages(
    store: &Store,
    session_id: &str,
    task_id: &str,
    seed: &CtxUiSizedHeadSeedSpec,
    turns: &[CtxUiTurnSeed],
) -> anyhow::Result<()> {
    let mut message_index = 0_i64;
    while message_index < seed.message_count {
        let end = (message_index + CTX_UI_SIZED_SEED_BATCH).min(seed.message_count);
        let rows = (message_index..end).collect::<Vec<_>>();
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO messages (
                id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content,
                attachments_json, delivery, delivered_at, created_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, index| {
            let turn_index =
                (*index * seed.turn_count / seed.message_count).clamp(0, seed.turn_count - 1);
            let turn = &turns[turn_index as usize];
            values
                .push_bind(MessageId::new().0.to_string())
                .push_bind(session_id)
                .push_bind(task_id)
                .push_bind(&turn.run_id)
                .push_bind(&turn.turn_id)
                .push_bind(*index)
                .push_bind(*index)
                .push_bind("assistant")
                .push_bind(format!("ctx-ui fixture answer {index}"))
                .push_bind("[]")
                .push_bind("immediate")
                .push_bind(Option::<String>::None)
                .push_bind(&turn.started_at);
        });
        builder.build().execute(store.pool()).await?;
        message_index = end;
    }
    Ok(())
}

async fn seed_ctx_ui_sized_tools(
    store: &Store,
    session_id: &str,
    seed: &CtxUiSizedHeadSeedSpec,
    turns: &[CtxUiTurnSeed],
) -> anyhow::Result<()> {
    let output_text = "x".repeat(seed.tool_output_bytes);
    let input_json = serde_json::json!({
        "cmd": "printf ctx-ui-sized-fixture",
        "env": {"CTX_FIXTURE": "long-tail"},
        "payload": "y".repeat(256),
    })
    .to_string();
    let tool_turns = turns
        .iter()
        .filter(|turn| turn.tool_total > 0)
        .collect::<Vec<_>>();

    let mut tool_index = 0_i64;
    while tool_index < seed.tool_count {
        let end = (tool_index + CTX_UI_SIZED_SEED_BATCH).min(seed.tool_count);
        let rows = (tool_index..end).collect::<Vec<_>>();
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_turn_tools (
                session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle,
                status, input_json, output_text, order_seq, first_event_seq, input_truncated,
                input_original_bytes, output_truncated, output_original_bytes, created_at, updated_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, index| {
            let mut turn = tool_turns[(*index as usize) % tool_turns.len()];
            if turn.index + 1 == seed.turn_count && tool_turns.len() > 1 {
                turn = tool_turns[tool_turns.len() - 2];
            }
            values
                .push_bind(session_id)
                .push_bind(format!("ctx-ui-tool-{index}"))
                .push_bind(&turn.turn_id)
                .push_bind("exec")
                .push_bind("exec_command")
                .push_bind(format!("Fixture command {index}"))
                .push_bind(format!("turn {}", turn.index))
                .push_bind("completed")
                .push_bind(&input_json)
                .push_bind(&output_text)
                .push_bind(*index)
                .push_bind(turn.start_seq)
                .push_bind(0_i64)
                .push_bind(input_json.len() as i64)
                .push_bind(0_i64)
                .push_bind(output_text.len() as i64)
                .push_bind(&turn.started_at)
                .push_bind(&turn.started_at);
        });
        builder.build().execute(store.pool()).await?;
        tool_index = end;
    }
    Ok(())
}

async fn seed_ctx_ui_sized_session(
    store: &Store,
    session_id: SessionId,
    task_id: TaskId,
    seed: &CtxUiSizedHeadSeedSpec,
) -> anyhow::Result<()> {
    let session_id = session_id.0.to_string();
    let task_id = task_id.0.to_string();
    let turns = build_ctx_ui_sized_turns(seed);

    insert_ctx_ui_sized_turns(store, &session_id, &turns).await?;
    seed_ctx_ui_sized_events(store, &session_id, seed, &turns).await?;
    seed_ctx_ui_sized_messages(store, &session_id, &task_id, seed, &turns).await?;
    seed_ctx_ui_sized_tools(store, &session_id, seed, &turns).await?;
    Ok(())
}

async fn latest_ctx_ui_sized_turn_id(
    store: &Store,
    session_id: SessionId,
) -> anyhow::Result<TurnId> {
    let value: String = sqlx::query_scalar(
        r#"SELECT turn_id
           FROM session_turns
           WHERE session_id = ?
           ORDER BY start_seq DESC
           LIMIT 1"#,
    )
    .bind(session_id.0.to_string())
    .fetch_one(store.pool())
    .await?;
    let parsed = uuid::Uuid::parse_str(&value)?;
    Ok(TurnId(parsed))
}

async fn tail_ctx_ui_sized_turn_ids(
    store: &Store,
    session_id: SessionId,
    limit: i64,
) -> anyhow::Result<Vec<TurnId>> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"SELECT turn_id
           FROM session_turns
           WHERE session_id = ?
           ORDER BY start_seq DESC
           LIMIT ?"#,
    )
    .bind(session_id.0.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    rows.into_iter()
        .map(|value| {
            uuid::Uuid::parse_str(&value)
                .map(TurnId)
                .map_err(Into::into)
        })
        .collect()
}

impl TestDaemon {
    pub fn new(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        Self::new_with_public_base_url(data_root, stores, providers, daemon_url, None, auth_token)
    }

    pub async fn new_for_test(data_root: PathBuf, daemon_url: String) -> anyhow::Result<Self> {
        let stores = StoreManager::open(&data_root).await?;
        Ok(Self::new(
            data_root,
            stores,
            HashMap::new(),
            daemon_url,
            None,
        ))
    }

    pub fn new_with_public_base_url(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        public_base_url: Option<String>,
        auth_token: Option<String>,
    ) -> Self {
        Self::from_state(Arc::new(DaemonState::new_with_public_base_url(
            data_root,
            stores,
            providers,
            daemon_url,
            public_base_url,
            auth_token,
        )))
    }

    pub fn new_with_runtime_flags(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        public_base_url: Option<String>,
        auth_token: Option<String>,
        runtime_flags: AppRuntimeFlags,
    ) -> Self {
        Self::from_state(Arc::new(DaemonState::new_with_runtime_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            public_base_url,
            auth_token,
            runtime_flags,
        )))
    }

    pub fn from_state(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn handle(&self) -> DaemonHandle {
        DaemonHandle::new(Arc::clone(&self.state))
    }

    pub fn data_root(&self) -> &Path {
        &self.state.core.data_root
    }

    pub fn tool_output_spool_dir(&self) -> &Path {
        self.state.test_tool_output_spool_dir()
    }

    pub fn daemon_url(&self) -> &str {
        &self.state.core.daemon_url
    }

    pub fn global_store(&self) -> &Store {
        self.state.global_store()
    }

    pub fn stores(&self) -> &StoreManager {
        &self.state.core.stores
    }

    pub fn request_shutdown(&self) {
        let _ = self.state.core.shutdown_tx.send(());
    }

    pub async fn set_session_running(&self, session_id: SessionId, running: bool) {
        self.state.set_running(session_id, running).await;
    }

    pub async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.state.is_session_running(session_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> anyhow::Result<Store> {
        self.state.store_for_session(session_id).await
    }

    pub async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> anyhow::Result<Store> {
        self.state.store_for_workspace(workspace_id).await
    }

    pub async fn uncached_store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.state
            .core
            .stores
            .workspace_uncached(workspace_id)
            .await
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> anyhow::Result<Store> {
        self.state.store_for_task(task_id).await
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<tokio::sync::Mutex<()>> {
        self.state.task_session_creation_lock(task_id).await
    }

    pub async fn seed_task_default_workspace_for_test(
        &self,
        name: &str,
        root_path: &Path,
        vcs_kind: VcsKind,
    ) -> anyhow::Result<Workspace> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                name.to_string(),
                root_path.to_string_lossy().to_string(),
                vcs_kind,
            )
            .await?;
        let _ = self.state.store_for_workspace(workspace.id).await?;
        Ok(workspace)
    }

    pub async fn task_default_session_snapshot_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> anyhow::Result<TaskDefaultSessionSnapshot> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        let task = store.get_task(task_id).await?;
        let sessions = store.list_sessions_for_task(task_id).await?;
        let task_count = store.list_tasks(workspace_id).await?.len();
        let worktree_count = store.list_worktrees(workspace_id).await?.len();
        Ok(TaskDefaultSessionSnapshot {
            task,
            sessions,
            task_count,
            worktree_count,
        })
    }

    pub async fn task_default_workspace_counts_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<(usize, usize)> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        Ok((
            store.list_tasks(workspace_id).await?.len(),
            store.list_worktrees(workspace_id).await?.len(),
        ))
    }

    pub async fn seed_task_default_session_task_for_test(
        &self,
        workspace_id: WorkspaceId,
        title: &str,
    ) -> anyhow::Result<Task> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        let task = store
            .create_task(workspace_id, title.to_string(), None)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace_id)
            .await?;
        Ok(task)
    }

    pub async fn hold_task_session_creation_lock_for_test(
        &self,
        task_id: TaskId,
    ) -> TaskSessionCreationLockGuardForTest {
        let lock = self.state.task_session_creation_lock(task_id).await;
        let guard = lock.clone().lock_owned().await;
        TaskSessionCreationLockGuardForTest {
            _lock: lock,
            _guard: guard,
        }
    }

    pub async fn wait_for_task_persisted_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if store.get_task(task_id).await?.is_some() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("task {task_id:?} was not persisted before timeout");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn simulate_missing_workspace_task_index_for_test(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .delete_workspace_task_index(task_id)
            .await
            .map_err(Into::into)
    }

    pub async fn seed_task_lifecycle_workspace_for_test(
        &self,
        name: &str,
        root_path: &Path,
        vcs_kind: VcsKind,
    ) -> anyhow::Result<Workspace> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                name.to_string(),
                root_path.to_string_lossy().to_string(),
                vcs_kind,
            )
            .await?;
        let _ = self.state.store_for_workspace(workspace.id).await?;
        Ok(workspace)
    }

    pub async fn seed_task_lifecycle_task_for_test(
        &self,
        workspace_id: WorkspaceId,
        title: &str,
    ) -> anyhow::Result<Task> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        let task = store
            .create_task(workspace_id, title.to_string(), None)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace_id)
            .await?;
        Ok(task)
    }

    pub async fn seed_task_lifecycle_stale_task_index_for_test(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .upsert_workspace_task_index(task_id, workspace_id)
            .await?;
        Ok(())
    }

    pub async fn seed_task_lifecycle_worktree_for_test(
        &self,
        seed: TaskLifecycleWorktreeSeed,
    ) -> anyhow::Result<Worktree> {
        let store = self.state.store_for_workspace(seed.workspace_id).await?;
        let worktree = store
            .insert_worktree(Worktree {
                id: seed.worktree_id,
                workspace_id: seed.workspace_id,
                root_path: seed.root_path.to_string_lossy().to_string(),
                base_commit_sha: seed.base_commit.clone(),
                git_branch: Some(seed.git_branch),
                vcs_kind: Some(VcsKind::Git),
                base_revision: Some(seed.base_commit),
                vcs_ref: Some(String::new()),
                created_at: chrono::Utc::now(),
                bootstrap_status: None,
                bootstrap_started_at: None,
                bootstrap_finished_at: None,
                bootstrap_exit_code: None,
                bootstrap_timeout_sec: None,
                bootstrap_error: None,
                bootstrap_log_path: None,
                bootstrap_log_truncated: None,
                bootstrap_command: None,
                bootstrap_script_path: None,
            })
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, seed.workspace_id)
            .await?;
        if seed.make_primary {
            store
                .set_task_primary_worktree(seed.owner_task_id, worktree.id)
                .await?;
        }
        Ok(worktree)
    }

    pub async fn seed_task_lifecycle_sandbox_binding_for_test(
        &self,
        seed: TaskLifecycleSandboxBindingSeed,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_workspace(seed.workspace_id).await?;
        store
            .upsert_sandbox_binding(SandboxBinding {
                worktree_id: seed.worktree_id,
                workspace_id: seed.workspace_id,
                sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(
                    seed.workspace_id,
                ),
                substrate: seed.substrate,
                guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
                profile: SandboxProfile::Standard,
                live_workspace_root: seed.live_workspace_root,
                live_worktree_root: seed.live_worktree_root,
                execution_settings_json: seed.execution_settings_json,
                container_name: seed.container_name,
                host_materialization_root: seed
                    .host_materialization_root
                    .map(|path| path.to_string_lossy().to_string()),
                created_at: chrono::Utc::now(),
            })
            .await?;
        Ok(())
    }

    pub async fn save_task_lifecycle_execution_settings_for_test(
        &self,
        execution: ExecutionSettings,
    ) -> anyhow::Result<()> {
        let settings = Settings {
            execution: Some(execution),
            ..Default::default()
        };
        ctx_settings_service::save_settings(self.state.global_store(), &settings).await?;
        Ok(())
    }

    pub async fn task_lifecycle_effective_execution_settings_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<ExecutionSettings> {
        self.handle()
            .workspaces()
            .effective_execution_settings(workspace_id)
            .await
            .map_err(Into::into)
    }

    pub async fn seed_task_lifecycle_session_for_test(
        &self,
        seed: TaskLifecycleSessionSeed,
    ) -> anyhow::Result<Session> {
        let store = self.state.store_for_workspace(seed.workspace_id).await?;
        let session = store
            .create_session(
                seed.task_id,
                seed.workspace_id,
                seed.worktree_id,
                seed.execution_environment,
                "fake".to_string(),
                "model".to_string(),
                seed.title,
                seed.parent_session_id,
                seed.role,
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, seed.workspace_id)
            .await?;
        Ok(session)
    }

    pub async fn archive_task_lifecycle_row_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> anyhow::Result<bool> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        store.archive_task(task_id).await.map_err(Into::into)
    }

    pub async fn archive_task_lifecycle_subagent_session_for_test(
        &self,
        workspace_id: WorkspaceId,
        parent_session_id: SessionId,
        child_session_id: SessionId,
    ) -> anyhow::Result<bool> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        store
            .archive_subagent_session(parent_session_id, child_session_id)
            .await
            .map_err(Into::into)
    }

    pub async fn task_lifecycle_snapshot_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<TaskLifecycleSnapshot> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        Ok(TaskLifecycleSnapshot {
            task: store.get_task(task_id).await?,
            worktree: store.get_worktree(worktree_id).await?,
            worktree_index_workspace_id: self
                .state
                .global_store()
                .get_workspace_id_for_worktree(worktree_id)
                .await?,
            sandbox_binding: store.get_sandbox_binding(worktree_id).await?,
        })
    }

    pub async fn seed_global_id_routing_workspace_session_for_test(
        &self,
        seed: GlobalIdRoutingWorkspaceSessionSeed,
    ) -> anyhow::Result<GlobalIdRoutingSessionFixture> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                seed.name.clone(),
                seed.root_path.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await?;
        let store = self.state.store_for_workspace(workspace.id).await?;
        let worktree = store
            .create_worktree(
                workspace.id,
                seed.root_path.to_string_lossy().to_string(),
                seed.base_commit,
                None,
            )
            .await?;
        let task = store.create_task(workspace.id, seed.name, None).await?;
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ExecutionEnvironment::Host,
                seed.provider_id,
                seed.model_id,
                "assistant".to_string(),
                None,
                None,
                None,
            )
            .await?;

        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace.id)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await?;

        Ok(GlobalIdRoutingSessionFixture {
            session_id: session.id,
        })
    }

    pub async fn seed_global_id_routing_queued_message_for_test(
        &self,
        session_id: SessionId,
        content: &str,
    ) -> anyhow::Result<MessageId> {
        let store = self.state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session {session_id:?} not found"))?;
        let message = store
            .insert_message(Message {
                id: MessageId::new(),
                session_id: session.id,
                task_id: session.task_id,
                run_id: None,
                turn_id: None,
                turn_sequence: None,
                order_seq: None,
                role: MessageRole::User,
                content: content.to_string(),
                attachments: Vec::new(),
                delivery: MessageDelivery::Queued,
                delivered_at: None,
                created_at: chrono::Utc::now(),
            })
            .await?;
        Ok(message.id)
    }

    pub async fn global_id_routing_message_exists_for_test(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> anyhow::Result<bool> {
        let store = self.state.store_for_session(session_id).await?;
        Ok(store
            .get_message(message_id)
            .await?
            .is_some_and(|message| message.session_id == session_id))
    }

    pub async fn seed_shutdown_running_turn_for_test(
        &self,
        root_path: &Path,
        provider_id: &str,
        model_id: &str,
    ) -> anyhow::Result<ShutdownRunningTurnFixture> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                root_path.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await?;
        let store = self.state.store_for_workspace(workspace.id).await?;
        let worktree = store
            .create_worktree(
                workspace.id,
                root_path.to_string_lossy().to_string(),
                "deadbeef".to_string(),
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await?;
        let task = store
            .create_task(workspace.id, "task".to_string(), None)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace.id)
            .await?;
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ExecutionEnvironment::Host,
                provider_id.to_string(),
                model_id.to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await?;

        let turn_id = TurnId::new();
        let now = chrono::Utc::now();
        store
            .insert_session_turn(SessionTurn {
                turn_id,
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
                failure: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            })
            .await?;

        Ok(ShutdownRunningTurnFixture {
            workspace_id: workspace.id,
            session_id: session.id,
            turn_id,
        })
    }

    pub async fn session_turn_status_for_test(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> anyhow::Result<Option<SessionTurnStatus>> {
        Ok(self
            .state
            .store_for_session(session_id)
            .await?
            .get_session_turn(session_id, turn_id)
            .await?
            .map(|turn| turn.status))
    }

    pub async fn seed_invalid_workspace_runtime_settings_document_for_test(
        &self,
        workspace_id: WorkspaceId,
        contents: &str,
    ) -> anyhow::Result<()> {
        self.state
            .store_for_workspace(workspace_id)
            .await?
            .upsert_runtime_settings_document(1, contents)
            .await?;
        Ok(())
    }

    pub async fn seed_workspace_runtime_settings_without_target_branch_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        self.state
            .store_for_workspace(workspace_id)
            .await?
            .upsert_runtime_settings_document(1, "{}")
            .await?;
        Ok(())
    }

    pub async fn seed_org_visible_run_archive_fixture_for_test(
        &self,
        workspace_id: WorkspaceId,
        worktree_root: &Path,
    ) -> anyhow::Result<RunArchiveRouteFixture> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        let task = store
            .create_task(workspace_id, "team archive".to_string(), None)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace_id)
            .await?;
        let worktree = store
            .create_worktree(
                workspace_id,
                worktree_root.to_string_lossy().into_owned(),
                "deadbeef".to_string(),
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace_id)
            .await?;
        let session = store
            .create_session(
                task.id,
                workspace_id,
                worktree.id,
                ExecutionEnvironment::Host,
                "fake".to_string(),
                "fake-model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace_id)
            .await?;

        let now = chrono::Utc::now();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        store
            .upsert_run(RunRecord {
                id: run_id,
                session_id: session.id,
                task_id: task.id,
                workspace_id,
                worktree_id: session.worktree_id,
                parent_run_id: None,
                account_id: Some(ctx_core::ids::AccountId::new()),
                org_id: Some(ctx_core::ids::OrgId::new()),
                run_grant_id: None,
                status: RunStatus::Completed,
                archive_state: RunArchiveState::Archived,
                archive_visibility: ArchiveVisibility::OrgEvidence,
                retention_policy: Some(RetentionPolicyRef {
                    policy_key: "team-default".into(),
                    legal_hold_key: None,
                }),
                created_at: now,
                started_at: Some(now),
                completed_at: Some(now),
                archived_at: Some(now),
                updated_at: now,
            })
            .await?;
        store
            .insert_message(Message {
                id: MessageId::new(),
                session_id: session.id,
                task_id: task.id,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                turn_sequence: Some(1),
                order_seq: None,
                role: MessageRole::Assistant,
                content: "reviewed /Users/example-user/project/.env".into(),
                attachments: Vec::new(),
                delivery: MessageDelivery::Immediate,
                delivered_at: None,
                created_at: now,
            })
            .await?;
        store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::Notice,
                serde_json::json!({
                    "kind": "archive_evidence",
                    "api_key": "sk-test-placeholder-1234567890"
                }),
            )
            .await?;

        Ok(RunArchiveRouteFixture {
            workspace_id,
            run_id,
        })
    }

    pub async fn seed_title_generation_session_for_test(
        &self,
        root_path: &Path,
    ) -> anyhow::Result<Session> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                root_path.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await?;
        let store = self.state.store_for_workspace(workspace.id).await?;
        let worktree = store
            .create_worktree(
                workspace.id,
                root_path.to_string_lossy().to_string(),
                "base".to_string(),
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await?;
        let task = store
            .create_task(
                workspace.id,
                ctx_session_service::title_generation::DEFAULT_SESSION_TITLE.to_string(),
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace.id)
            .await?;
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ExecutionEnvironment::Host,
                "fake".to_string(),
                "fake-model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await?;
        Ok(session)
    }

    pub async fn schedule_fallback_title_generation_for_test(
        &self,
        session_id: SessionId,
        prompt: &str,
        force: bool,
    ) -> anyhow::Result<bool> {
        let session = self
            .state
            .store_for_session(session_id)
            .await?
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session {session_id:?} not found"))?;
        Ok(
            daemon::sessions::title_generation::schedule_session_title_generation(
                Arc::clone(&self.state),
                session,
                prompt.to_string(),
                force,
            )
            .await,
        )
    }

    pub async fn session_title_for_test(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<String>> {
        Ok(self
            .state
            .store_for_session(session_id)
            .await?
            .get_session(session_id)
            .await?
            .map(|session| session.title))
    }

    pub async fn seed_sandbox_bound_worktree_for_test(
        &self,
        workspace_name: &str,
        workspace_root: &Path,
        host_worktree_root: &Path,
        live_workspace_root: &str,
    ) -> anyhow::Result<Worktree> {
        let workspace = self
            .state
            .global_store()
            .create_workspace(
                workspace_name.to_string(),
                workspace_root.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await?;
        let store = self.state.store_for_workspace(workspace.id).await?;
        let worktree = store
            .insert_worktree(Worktree {
                id: WorktreeId::new(),
                workspace_id: workspace.id,
                root_path: host_worktree_root.to_string_lossy().to_string(),
                base_commit_sha: "abc123".to_string(),
                git_branch: Some("ctx/test".to_string()),
                vcs_kind: Some(VcsKind::Git),
                base_revision: Some("abc123".to_string()),
                vcs_ref: Some("ctx/test".to_string()),
                created_at: chrono::Utc::now(),
                bootstrap_status: None,
                bootstrap_started_at: None,
                bootstrap_finished_at: None,
                bootstrap_exit_code: None,
                bootstrap_timeout_sec: None,
                bootstrap_error: None,
                bootstrap_log_path: None,
                bootstrap_log_truncated: None,
                bootstrap_command: None,
                bootstrap_script_path: None,
            })
            .await?;
        store
            .upsert_sandbox_binding(SandboxBinding {
                worktree_id: worktree.id,
                workspace_id: workspace.id,
                sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(
                    workspace.id,
                ),
                substrate: SandboxSubstrate::SharedVmContainer,
                guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
                profile: SandboxProfile::Standard,
                live_workspace_root: live_workspace_root.to_string(),
                live_worktree_root: format!(
                    "{}/worktrees/{}",
                    live_workspace_root.trim_end_matches('/'),
                    worktree.id.0
                ),
                execution_settings_json: None,
                container_name: Some("ctx-test".to_string()),
                host_materialization_root: None,
                created_at: chrono::Utc::now(),
            })
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await?;
        Ok(worktree)
    }

    pub fn spawn_merge_queue_runner(&self) {
        daemon::merge_queue::spawn_merge_queue_runner(Arc::clone(&self.state));
    }

    pub async fn seed_workspace_merge_queue_queued_entry_for_test(
        &self,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> anyhow::Result<MergeQueueEntry> {
        let now = chrono::Utc::now();
        let entry = MergeQueueEntry {
            id: MergeQueueEntryId::new(),
            workspace_id,
            worktree_id: None,
            session_id: None,
            target_branch: "main".to_string(),
            message: Some(name.to_string()),
            patch_source: MergeQueuePatchSource::Generated,
            base_commit_sha: Some(format!("{name}-base")),
            head_commit_sha: Some(format!("{name}-head")),
            patch_path: format!("/tmp/{name}.patch"),
            patch_size: 1,
            status: MergeQueueEntryStatus::Queued,
            result_commit_sha: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        let store = self.uncached_store_for_workspace(workspace_id).await?;
        store.create_merge_queue_entry(&entry).await?;
        store.close().await;
        Ok(entry)
    }

    pub async fn load_workspace_merge_queue_entry_for_test(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> anyhow::Result<MergeQueueEntry> {
        let store = self.uncached_store_for_workspace(workspace_id).await?;
        let entry = store
            .get_merge_queue_entry(entry_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("merge queue entry {entry_id:?} should exist"))?;
        store.close().await;
        Ok(entry)
    }

    pub async fn wait_for_workspace_merge_queue_entry_to_leave_queued_for_test(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
        timeout: Duration,
    ) -> anyhow::Result<MergeQueueEntry> {
        tokio::time::timeout(timeout, async {
            loop {
                let entry = self
                    .load_workspace_merge_queue_entry_for_test(workspace_id, entry_id)
                    .await?;
                if entry.status != MergeQueueEntryStatus::Queued {
                    break Ok(entry);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for merge queue entry to resume"))?
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), daemon::workspaces::WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
    }

    pub async fn session_worktree_root_path_for_test(
        &self,
        session: &Session,
    ) -> anyhow::Result<PathBuf> {
        Ok(PathBuf::from(
            self.load_worktree_for_test(session.worktree_id)
                .await?
                .root_path,
        ))
    }

    pub async fn seed_legacy_session_artifact_by_path_for_test(
        &self,
        session: &Session,
        absolute_path: &Path,
        name: &str,
        mime_type: &str,
        bytes: i64,
    ) -> anyhow::Result<Artifact> {
        let artifact = Artifact {
            id: ctx_core::ids::ArtifactId::new(),
            session_id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            worktree_id: session.worktree_id,
            name: Some(name.to_string()),
            absolute_path: absolute_path.to_string_lossy().to_string(),
            mime_type: mime_type.to_string(),
            bytes,
            created_at: chrono::Utc::now(),
            missing: None,
        };
        self.state
            .store_for_session(session.id)
            .await?
            .upsert_session_artifact_by_path(&artifact)
            .await
            .map_err(Into::into)
    }

    pub async fn record_worktree_bootstrap_log_for_test(
        &self,
        session: &Session,
        status: WorktreeBootstrapStatus,
        log_path: &Path,
        error: Option<&str>,
        command: &str,
    ) -> anyhow::Result<WorktreeId> {
        let worktree = self.load_worktree_for_test(session.worktree_id).await?;
        let now = chrono::Utc::now();
        self.state
            .store_for_worktree(worktree.id)
            .await?
            .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
                worktree_id: worktree.id,
                status,
                started_at: now,
                finished_at: now,
                exit_code: Some(if error.is_some() { 1 } else { 0 }),
                timeout_sec: Some(60),
                error: error.map(str::to_string),
                log_path: Some(log_path.to_string_lossy().to_string()),
                log_truncated: Some(false),
                command: Some(command.to_string()),
                script_path: None,
            })
            .await?;
        Ok(worktree.id)
    }

    pub async fn seed_failed_merge_queue_log_run_for_test(
        &self,
        workspace_id: WorkspaceId,
        message: &str,
        log_path: &Path,
        error_message: &str,
    ) -> anyhow::Result<MergeQueueEntryId> {
        let now = chrono::Utc::now();
        let entry = MergeQueueEntry {
            id: MergeQueueEntryId::new(),
            workspace_id,
            worktree_id: None,
            session_id: None,
            target_branch: "main".to_string(),
            message: Some(message.to_string()),
            patch_source: MergeQueuePatchSource::Generated,
            base_commit_sha: Some("base".to_string()),
            head_commit_sha: Some("head".to_string()),
            patch_path: "/tmp/log-path-boundary.patch".to_string(),
            patch_size: 1,
            status: MergeQueueEntryStatus::Failed,
            result_commit_sha: None,
            error_message: Some("failed".to_string()),
            created_at: now,
            updated_at: now,
        };
        let run = MergeQueueRun {
            id: MergeQueueRunId::new(),
            entry_id: entry.id,
            status: MergeQueueRunStatus::Failed,
            started_at: now,
            finished_at: Some(now),
            exit_code: Some(1),
            log_path: Some(log_path.to_string_lossy().to_string()),
            error_message: Some(error_message.to_string()),
            result_commit_sha: None,
        };
        let store = self.state.store_for_workspace(workspace_id).await?;
        store.create_merge_queue_entry(&entry).await?;
        store.create_merge_queue_run(&run).await?;
        Ok(entry.id)
    }

    pub async fn session_has_no_persisted_messages_for_test(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<bool> {
        Ok(self
            .state
            .store_for_session(session_id)
            .await?
            .list_messages_for_session(session_id)
            .await?
            .is_empty())
    }

    pub async fn wait_for_assistant_message_for_test(
        &self,
        session_id: SessionId,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_session(session_id).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let messages = store.list_messages_for_session(session_id).await?;
            if messages
                .iter()
                .any(|message| matches!(message.role, MessageRole::Assistant))
            {
                return Ok(());
            }
            let turns = store
                .list_session_turns_page_by_seq(session_id, None, Some(10))
                .await?;
            if turns.iter().any(|turn| {
                matches!(
                    turn.status,
                    SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
                )
            }) {
                anyhow::bail!("turn failed before assistant message was produced: {turns:#?}");
            }
            if tokio::time::Instant::now() >= deadline {
                let events = store.list_session_events(session_id).await?;
                anyhow::bail!(
                    "assistant message not produced; messages={messages:#?}; events={events:#?}; turns={turns:#?}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn wait_for_session_turn_failed_for_test(
        &self,
        session_id: SessionId,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_session(session_id).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let turns = store
                .list_session_turns_page_by_seq(session_id, None, Some(10))
                .await?;
            if turns
                .last()
                .is_some_and(|turn| turn.status == SessionTurnStatus::Failed)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("turn did not fail before timeout; turns={turns:#?}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn wait_for_scheduler_runtime_events_for_test<F>(
        &self,
        session_id: SessionId,
        timeout: Duration,
        label: &str,
        mut predicate: F,
    ) -> anyhow::Result<Vec<SessionEvent>>
    where
        F: FnMut(&[SessionEvent]) -> anyhow::Result<bool>,
    {
        let store = self.state.store_for_session(session_id).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let events = store.list_session_events(session_id).await?;
            if predicate(&events)? {
                return Ok(events);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for {label}: {events:#?}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub async fn wait_for_session_done_event_count_for_test(
        &self,
        session_id: SessionId,
        expected_done_events: usize,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.wait_for_scheduler_runtime_events_for_test(
            session_id,
            timeout,
            &format!("{expected_done_events} done events"),
            |events| {
                if events
                    .iter()
                    .any(|event| matches!(event.event_type, SessionEventType::Error))
                {
                    anyhow::bail!(
                        "unexpected session error while waiting for done events: {events:#?}"
                    );
                }
                let done_count = events
                    .iter()
                    .filter(|event| matches!(event.event_type, SessionEventType::Done))
                    .count();
                Ok(done_count >= expected_done_events)
            },
        )
        .await
        .map(|_| ())
    }

    pub async fn assistant_chunk_stream_snapshot_for_test(
        &self,
        session_id: SessionId,
        timeout: Duration,
    ) -> anyhow::Result<AssistantChunkStreamSnapshot> {
        let events = self
            .wait_for_scheduler_runtime_events_for_test(
                session_id,
                timeout,
                "Done event",
                |events| {
                    Ok(events
                        .iter()
                        .any(|event| matches!(event.event_type, SessionEventType::Done)))
                },
            )
            .await?;
        let turns = self
            .state
            .store_for_session(session_id)
            .await?
            .list_session_turns_page_by_seq(session_id, None, Some(1))
            .await?;
        Ok(AssistantChunkStreamSnapshot { events, turns })
    }

    pub async fn wait_for_terminal_turn_persistence_snapshot_for_test(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        timeout: Duration,
    ) -> anyhow::Result<TerminalTurnPersistenceSnapshot> {
        let store = self.state.store_for_session(session_id).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let turn = store
                .get_session_turn(session_id, turn_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("turn {turn_id:?} not found"))?;
            let events = store
                .list_session_events_for_turn(session_id, turn_id, false)
                .await?;
            let terminal = matches!(
                turn.status,
                SessionTurnStatus::Completed
                    | SessionTurnStatus::Failed
                    | SessionTurnStatus::Interrupted
            );
            let finished = events
                .iter()
                .any(|event| matches!(event.event_type, SessionEventType::TurnFinished));
            if terminal && finished {
                let assistant_messages = store
                    .list_messages_for_session(session_id)
                    .await?
                    .into_iter()
                    .filter(|message| {
                        message.turn_id == Some(turn_id)
                            && matches!(message.role, MessageRole::Assistant)
                    })
                    .collect();
                return Ok(TerminalTurnPersistenceSnapshot {
                    turn,
                    events,
                    assistant_messages,
                });
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for terminal turn: {events:#?}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn seed_running_turn_for_reconciliation_test(
        &self,
        session_id: SessionId,
        run_id: RunId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        self.state
            .store_for_session(session_id)
            .await?
            .insert_session_turn(SessionTurn {
                turn_id,
                session_id,
                run_id: Some(run_id),
                user_message_id: Some(MessageId::new()),
                status: SessionTurnStatus::Running,
                start_seq: Some(1),
                end_seq: None,
                started_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                failure: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            })
            .await?;
        Ok(())
    }

    pub async fn append_turn_finished_event_for_test(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: TurnId,
        status: SessionTurnStatus,
    ) -> anyhow::Result<SessionEvent> {
        let status = match status {
            SessionTurnStatus::Completed => "completed",
            SessionTurnStatus::Failed => "failed",
            SessionTurnStatus::Interrupted => "interrupted",
            other => anyhow::bail!("unsupported terminal status for fixture: {other:?}"),
        };
        self.state
            .store_for_session(session_id)
            .await?
            .append_session_event(
                session_id,
                run_id,
                Some(turn_id),
                SessionEventType::TurnFinished,
                serde_json::json!({ "status": status }),
            )
            .await
            .map_err(Into::into)
    }

    pub async fn turn_reconciliation_snapshot_for_test(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> anyhow::Result<TurnReconciliationSnapshot> {
        let store = self.state.store_for_session(session_id).await?;
        let turn = store
            .get_session_turn(session_id, turn_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("turn {turn_id:?} not found"))?;
        let events = store
            .list_session_events_for_turn(session_id, turn_id, false)
            .await?;
        let summary = store
            .get_session_snapshot(session_id, 50, false)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session snapshot {session_id:?} not found"))?
            .summary;
        Ok(TurnReconciliationSnapshot {
            turn,
            events,
            last_turn_status: summary.activity.last_turn_status,
            is_working: summary.activity.is_working,
        })
    }

    pub async fn seed_invalid_workspace_runtime_settings_for_test(
        &self,
        session_id: SessionId,
        contents: &str,
    ) -> anyhow::Result<()> {
        let _ = self
            .state
            .store_for_session(session_id)
            .await?
            .upsert_runtime_settings_document(1, contents)
            .await?;
        Ok(())
    }

    pub async fn session_has_user_message_event_for_test(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<bool> {
        let events = self
            .state
            .store_for_session(session_id)
            .await?
            .list_session_events(session_id)
            .await?;
        Ok(events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::UserMessage)))
    }

    pub async fn seed_large_session_head_fixture_for_test(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        task_id: TaskId,
        turns: i64,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        struct SeedRow {
            index: i64,
            event_seq: i64,
            event_id: String,
            message_id: String,
            run_id: String,
            turn_id: String,
            created_at: String,
            payload_json: String,
            input_json: String,
        }

        let store = self.state.store_for_session(session_id).await?;
        let session_id_value = session_id.0.to_string();
        let task_id_value = task_id.0.to_string();
        let started_at = chrono::Utc::now();
        // Seed fixture rows directly so this response-size test does not enqueue
        // projection work once per row before the explicit refresh below.
        let rows = (0..turns)
            .map(|index| {
                let created_at = started_at + chrono::Duration::milliseconds(index);
                SeedRow {
                    index,
                    event_seq: index + 1,
                    event_id: uuid::Uuid::new_v4().to_string(),
                    message_id: MessageId::new().0.to_string(),
                    run_id: RunId::new().0.to_string(),
                    turn_id: TurnId::new().0.to_string(),
                    created_at: created_at.to_rfc3339(),
                    payload_json: serde_json::json!({
                        "kind": "large_head_checkpoint",
                        "turn_index": index,
                    })
                    .to_string(),
                    input_json: serde_json::json!({ "cmd": format!("echo {index}") }).to_string(),
                }
            })
            .collect::<Vec<_>>();

        let mut turn_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_turns (
                turn_id, session_id, run_id, user_message_id, status, start_seq, end_seq,
                started_at, updated_at, assistant_partial, thought_partial, metrics_json,
                tool_total, tool_pending, tool_running, tool_completed, tool_failed
            ) "#,
        );
        turn_builder.push_values(&rows, |mut values, row| {
            values
                .push_bind(&row.turn_id)
                .push_bind(&session_id_value)
                .push_bind(&row.run_id)
                .push_bind(Option::<String>::None)
                .push_bind("completed")
                .push_bind(row.index + 1)
                .push_bind(row.index + 1)
                .push_bind(&row.created_at)
                .push_bind(&row.created_at)
                .push_bind(Option::<String>::None)
                .push_bind(Option::<String>::None)
                .push_bind(Option::<String>::None)
                .push_bind(1_i64)
                .push_bind(0_i64)
                .push_bind(0_i64)
                .push_bind(1_i64)
                .push_bind(0_i64);
        });
        turn_builder.build().execute(store.pool()).await?;

        let mut event_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_events (
                seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
            ) "#,
        );
        event_builder.push_values(&rows, |mut values, row| {
            values
                .push_bind(row.event_seq)
                .push_bind(&row.event_id)
                .push_bind(&session_id_value)
                .push_bind(&row.run_id)
                .push_bind(&row.turn_id)
                .push_bind("notice")
                .push_bind(&row.payload_json)
                .push_bind(0_i64)
                .push_bind(&row.created_at);
        });
        event_builder.build().execute(store.pool()).await?;

        let mut message_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO messages (
                id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content,
                attachments_json, delivery, delivered_at, created_at
            ) "#,
        );
        message_builder.push_values(&rows, |mut values, row| {
            values
                .push_bind(&row.message_id)
                .push_bind(&session_id_value)
                .push_bind(&task_id_value)
                .push_bind(&row.run_id)
                .push_bind(&row.turn_id)
                .push_bind(1_i64)
                .push_bind(Option::<i64>::None)
                .push_bind("assistant")
                .push_bind(format!("answer {}", row.index))
                .push_bind("[]")
                .push_bind("immediate")
                .push_bind(Option::<String>::None)
                .push_bind(&row.created_at);
        });
        message_builder.build().execute(store.pool()).await?;

        let mut tool_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_turn_tools (
                session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle,
                status, input_json, output_text, order_seq, first_event_seq, input_truncated,
                input_original_bytes, output_truncated, output_original_bytes, created_at, updated_at
            ) "#,
        );
        tool_builder.push_values(&rows, |mut values, row| {
            values
                .push_bind(&session_id_value)
                .push_bind(format!("tool-{}", row.index))
                .push_bind(&row.turn_id)
                .push_bind("execute")
                .push_bind("Bash")
                .push_bind("Bash")
                .push_bind(format!("turn {}", row.index))
                .push_bind("completed")
                .push_bind(&row.input_json)
                .push_bind(format!("output {}", row.index))
                .push_bind(1_i64)
                .push_bind(row.event_seq)
                .push_bind(0_i64)
                .push_bind(Option::<i64>::None)
                .push_bind(0_i64)
                .push_bind(Option::<i64>::None)
                .push_bind(&row.created_at)
                .push_bind(&row.created_at);
        });
        tool_builder.build().execute(store.pool()).await?;

        tokio::time::timeout(
            timeout,
            store.refresh_active_session_head_projection(session_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out refreshing active session head projection"))?
        .map_err(|err| anyhow::anyhow!("refresh active session head projection: {err}"))?;

        tokio::time::timeout(
            timeout,
            self.state
                .ensure_workspace_active_snapshot_hydrated(workspace_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out hydrating workspace active snapshot"))?
        .map_err(|err| anyhow::anyhow!("hydrate workspace active snapshot: {err:?}"))?;

        Ok(())
    }

    pub async fn seed_ctx_ui_sized_session_head_fixture_for_test(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        task_id: TaskId,
        seed: CtxUiSizedHeadSeedSpec,
        timeout: Duration,
    ) -> anyhow::Result<CtxUiSizedHeadSeedStats> {
        let store = self.state.store_for_session(session_id).await?;
        seed_ctx_ui_sized_session(&store, session_id, task_id, &seed).await?;

        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM session_events WHERE session_id = ?")
                .bind(session_id.0.to_string())
                .fetch_one(store.pool())
                .await?;
        let tool_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM session_turn_tools WHERE session_id = ?")
                .bind(session_id.0.to_string())
                .fetch_one(store.pool())
                .await?;
        let message_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?")
                .bind(session_id.0.to_string())
                .fetch_one(store.pool())
                .await?;

        tokio::time::timeout(
            timeout,
            store.refresh_active_session_head_projection(session_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out refreshing active projection for ctx-ui fixture"))?
        .map_err(|err| anyhow::anyhow!("refresh active ctx-ui projection: {err}"))?;

        tokio::time::timeout(
            timeout,
            self.state
                .ensure_workspace_active_snapshot_hydrated(workspace_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out hydrating workspace active snapshot"))?
        .map_err(|err| anyhow::anyhow!("hydrate workspace active snapshot: {err:?}"))?;

        Ok(CtxUiSizedHeadSeedStats {
            event_count,
            tool_count,
            message_count,
        })
    }

    pub async fn ctx_ui_sized_recent_tool_summary_probe_for_test(
        &self,
        session_id: SessionId,
        head_limit: i64,
        tool_summary_limit: usize,
    ) -> anyhow::Result<CtxUiSizedToolSummaryProbe> {
        let store = self.state.store_for_session(session_id).await?;
        let latest_turn_id = latest_ctx_ui_sized_turn_id(&store, session_id).await?;
        let tail_turn_ids = tail_ctx_ui_sized_turn_ids(&store, session_id, head_limit).await?;
        let bounded_tools = store
            .list_recent_turn_tool_summaries_for_turns(
                session_id,
                &tail_turn_ids,
                tool_summary_limit,
            )
            .await?;
        let oldest_loaded_order_seq = bounded_tools
            .iter()
            .map(|tool| tool.order_seq)
            .min()
            .ok_or_else(|| anyhow::anyhow!("ctx-ui sized tool probe returned no tools"))?;
        Ok(CtxUiSizedToolSummaryProbe {
            latest_turn_id,
            bounded_tool_count: bounded_tools.len(),
            oldest_loaded_order_seq,
        })
    }

    pub async fn seed_hot_endpoint_caches_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
        limit: i64,
        session_head_limit: u32,
        include_events: bool,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_session(session_id).await?;
        let _ = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"warm"}),
            )
            .await
            .map_err(|err| anyhow::anyhow!("append warm session event: {err}"))?;
        let _ = store
            .refresh_active_session_head_projection(session_id)
            .await
            .map_err(|err| anyhow::anyhow!("refresh active session head projection: {err}"))?;

        self.state.emit_workspace_task_upsert(task_id).await?;
        self.state.refresh_session_head_cache(session_id).await;

        let head_snapshot = store
            .get_session_head_snapshot(session_id, session_head_limit, include_events)
            .await
            .map_err(|err| anyhow::anyhow!("load session head snapshot: {err}"))?
            .ok_or_else(|| anyhow::anyhow!("session head snapshot {session_id:?} not found"))?;
        self.state
            .sessions
            .cache_session_head_snapshot(
                session_id,
                session_head_limit,
                include_events,
                head_snapshot,
            )
            .await;

        let deadline = tokio::time::Instant::now() + timeout;
        let mut cached_snapshot = self
            .state
            .workspaces
            .workspace_active_snapshot
            .active_snapshot(workspace_id, limit)
            .await;
        while cached_snapshot.active.tasks.is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cached_snapshot = self
                .state
                .workspaces
                .workspace_active_snapshot
                .active_snapshot(workspace_id, limit)
                .await;
        }
        if cached_snapshot.active.tasks.is_empty() {
            anyhow::bail!("expected active snapshot to be cached for workspace {workspace_id:?}");
        }
        self.state
            .cache_workspace_active_snapshot(cached_snapshot)
            .await;

        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
            .map_err(|err| anyhow::anyhow!("hydrate workspace active snapshot: {err:?}"))?;

        let mut cached_heads = self
            .state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await;
        while cached_heads.heads.is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cached_heads = self
                .state
                .workspaces
                .workspace_active_snapshot
                .active_heads(workspace_id)
                .await;
        }
        if cached_heads.heads.is_empty() {
            anyhow::bail!("expected active heads to be cached for workspace {workspace_id:?}");
        }
        self.state.cache_workspace_active_heads(cached_heads).await;

        Ok(())
    }

    pub async fn append_hot_endpoint_delta_notice_for_test(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<SessionEvent> {
        self.state
            .store_for_session(session_id)
            .await?
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({"msg":"delta only"}),
            )
            .await
            .map_err(|err| anyhow::anyhow!("append delta session event: {err}"))
    }

    pub async fn publish_hot_endpoint_event_and_active_head_seq_for_test(
        &self,
        workspace_id: WorkspaceId,
        event: SessionEvent,
        settle_for: Duration,
    ) -> anyhow::Result<i64> {
        self.state.publish_event(event).await;
        tokio::time::sleep(settle_for).await;
        let active_heads = self
            .state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await;
        let head = active_heads.heads.first().ok_or_else(|| {
            anyhow::anyhow!("expected active head for workspace {workspace_id:?}")
        })?;
        Ok(head.last_event_seq)
    }

    pub async fn seed_workspace_stream_stress_session_head_for_test(
        &self,
        session: &Session,
        turns_per_session: i64,
        message_content: &str,
        head_limit: u32,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_session(session.id).await?;
        for turn_sequence in 0..turns_per_session {
            let turn_id = TurnId::new();
            let at = fixed_test_utc(turn_sequence);
            store
                .insert_session_turn(SessionTurn {
                    turn_id,
                    session_id: session.id,
                    run_id: None,
                    user_message_id: None,
                    status: SessionTurnStatus::Completed,
                    start_seq: Some(turn_sequence),
                    end_seq: Some(turn_sequence),
                    started_at: at,
                    updated_at: at,
                    assistant_partial: None,
                    thought_partial: None,
                    metrics_json: None,
                    failure: None,
                    tool_total: 0,
                    tool_pending: 0,
                    tool_running: 0,
                    tool_completed: 0,
                    tool_failed: 0,
                })
                .await?;

            store
                .insert_message(Message {
                    id: MessageId::new(),
                    session_id: session.id,
                    task_id: session.task_id,
                    run_id: None,
                    turn_id: Some(turn_id),
                    turn_sequence: Some(turn_sequence),
                    order_seq: None,
                    role: MessageRole::User,
                    content: message_content.to_string(),
                    attachments: Vec::new(),
                    delivery: MessageDelivery::Immediate,
                    delivered_at: None,
                    created_at: at,
                })
                .await?;
        }

        let head = store
            .get_session_head_snapshot(session.id, head_limit, true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session head snapshot {:?} not found", session.id))?;
        self.state.test_update_session_head(head).await;
        Ok(())
    }

    pub async fn publish_workspace_stream_stress_delta_for_test(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        seq: i64,
    ) {
        let delta = SessionHeadDelta {
            session_id: session.id,
            last_event_seq: seq,
            projection_rev: seq,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: Some(SessionEvent {
                seq,
                id: SessionEventId::new(),
                session_id: session.id,
                run_id: None,
                turn_id: None,
                event_type: SessionEventType::Done,
                payload_json: serde_json::json!({"ok": true}),
                transient: false,
                created_at: chrono::Utc::now(),
            }),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        self.state
            .test_publish_session_head_delta_for_workspace(workspace_id, session, delta, false)
            .await;
    }

    pub async fn publish_replay_fixture_event_for_test(&self, event: SessionEvent) {
        self.state.publish_event(event).await;
    }

    pub async fn refresh_replay_projection_fixture_for_test(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> anyhow::Result<()> {
        self.state.refresh_session_head_cache(session_id).await;
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
            .map_err(|err| anyhow::anyhow!("hydrate replay projection fixture: {err:?}"))
    }

    pub async fn remove_replay_session_head_for_test(&self, session_id: SessionId) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .remove_session_head(session_id)
            .await;
    }

    pub async fn cache_rehydration_seed_replay_head_cache_for_test(
        &self,
        head: SessionHeadSnapshot,
    ) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .update_session_head(head)
            .await;
    }

    pub async fn cache_rehydration_seed_compact_head_cache_for_test(
        &self,
        head: SessionHeadSnapshot,
    ) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .update_compact_session_head(head)
            .await;
    }

    pub async fn cache_rehydration_replay_session_head_cached_for_test(
        &self,
        session_id: SessionId,
    ) -> Option<SessionHeadSnapshot> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(session_id)
            .await
    }

    pub async fn cache_rehydration_session_head_for_read_cached_for_test(
        &self,
        session_id: SessionId,
    ) -> Option<SessionHeadSnapshot> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .get_cached_session_head_for_read(session_id)
            .await
    }

    pub async fn cache_rehydration_cleanup_session_for_test(&self, session_id: SessionId) {
        self.state.cleanup_session(session_id).await;
    }

    pub async fn cache_rehydration_cleanup_workspace_for_test(&self, workspace_id: WorkspaceId) {
        self.state.cleanup_workspace(workspace_id).await;
    }

    pub async fn cache_rehydration_make_workspace_store_unopenable_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        self.state.core.stores.evict_workspace(workspace_id).await;
        let workspace_store_path = self
            .data_root()
            .join("db")
            .join("workspaces")
            .join(workspace_id.0.to_string());
        match tokio::fs::metadata(&workspace_store_path).await {
            Ok(metadata) if metadata.is_dir() => {
                tokio::fs::remove_dir_all(&workspace_store_path).await?;
            }
            Ok(_) => {
                tokio::fs::remove_file(&workspace_store_path).await?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = workspace_store_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("workspace store path has no parent"))?;
        tokio::fs::create_dir_all(parent).await?;
        tokio::fs::write(&workspace_store_path, b"blocked workspace store").await?;
        Ok(())
    }

    pub async fn cache_rehydration_begin_workspace_delete_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) {
        self.state
            .core
            .stores
            .begin_workspace_delete(workspace_id)
            .await;
    }

    pub async fn cache_rehydration_finish_workspace_delete_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) {
        self.state
            .core
            .stores
            .finish_workspace_delete(workspace_id)
            .await;
    }

    pub async fn cache_rehydration_hydrate_snapshot_for_test(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        archived_rev: i64,
        tasks: Vec<WorkspaceActiveTaskSummary>,
        heads: Vec<SessionHeadSnapshot>,
    ) {
        self.state
            .workspaces
            .workspace_active_snapshot
            .hydrate_snapshot(workspace_id, snapshot_rev, archived_rev, tasks, heads)
            .await;
    }

    pub async fn cache_rehydration_active_task_summary_cached_for_test(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> Option<WorkspaceActiveTaskSummary> {
        self.state
            .workspaces
            .workspace_active_snapshot
            .active_task_summary(workspace_id, task_id)
            .await
    }

    pub async fn cache_rehydration_workspace_needs_hydration_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> bool {
        self.state
            .workspaces
            .workspace_active_snapshot
            .needs_hydration(workspace_id)
            .await
    }

    pub async fn workspace_active_snapshot_make_store_unopenable_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        self.cache_rehydration_make_workspace_store_unopenable_for_test(workspace_id)
            .await
    }

    pub async fn workspace_active_snapshot_append_and_publish_event_for_test(
        &self,
        session: &Session,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> anyhow::Result<SessionEvent> {
        self.state.sessions.remember_session_meta(session).await;
        let event = self
            .state
            .store_for_session(session.id)
            .await?
            .append_session_event(session.id, run_id, turn_id, event_type, payload_json)
            .await?;
        self.state.publish_event(event.clone()).await;
        Ok(event)
    }

    pub async fn workspace_active_snapshot_seed_completed_turn_with_partials_for_test(
        &self,
        session: &Session,
        assistant_partial: &str,
        thought_partial: &str,
    ) -> anyhow::Result<TurnId> {
        let store = self.state.store_for_session(session.id).await?;
        let now = chrono::Utc::now();
        let turn_id = TurnId::new();
        store
            .insert_session_turn(SessionTurn {
                turn_id,
                session_id: session.id,
                run_id: None,
                user_message_id: None,
                status: SessionTurnStatus::Running,
                start_seq: Some(1),
                end_seq: None,
                started_at: now,
                updated_at: now,
                assistant_partial: Some(assistant_partial.to_string()),
                thought_partial: Some(thought_partial.to_string()),
                metrics_json: None,
                failure: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            })
            .await?;
        store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::AssistantComplete,
                serde_json::json!({
                    "full_content": "final answer",
                    "message_id": "provider-msg-1",
                    "order_seq": 2
                }),
            )
            .await?;
        let checkpoint_event = store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::Notice,
                serde_json::json!({ "kind": "test_checkpoint", "message": "stable" }),
            )
            .await?;
        store
            .update_session_turn_status(
                session.id,
                turn_id,
                SessionTurnStatus::Completed,
                Some(checkpoint_event.seq),
                None,
                chrono::Utc::now(),
            )
            .await?;
        Ok(turn_id)
    }

    pub async fn workspace_active_snapshot_task_contains_sessions_for_test(
        &self,
        task_id: TaskId,
        expected_sessions: &[SessionId],
    ) -> anyhow::Result<bool> {
        let store = self.state.store_for_task(task_id).await?;
        let sessions = store.list_sessions_for_task(task_id).await?;
        Ok(expected_sessions
            .iter()
            .all(|session_id| sessions.iter().any(|stored| stored.id == *session_id)))
    }

    pub async fn workspace_active_snapshot_load_session_worktree_for_test(
        &self,
        session: &Session,
    ) -> anyhow::Result<Worktree> {
        self.load_worktree_for_test(session.worktree_id).await
    }

    pub async fn workspace_active_snapshot_mark_vcs_pane_open_for_test(
        &self,
        worktree_id: WorktreeId,
    ) {
        let mut next_open = std::collections::HashSet::new();
        next_open.insert(worktree_id);
        self.state
            .update_worktree_vcs_open_panes(&std::collections::HashSet::new(), &next_open)
            .await;
    }

    pub async fn workspace_active_snapshot_mark_vcs_pane_closed_for_test(
        &self,
        worktree_id: WorktreeId,
    ) {
        let mut previous_open = std::collections::HashSet::new();
        previous_open.insert(worktree_id);
        self.state
            .update_worktree_vcs_open_panes(&previous_open, &std::collections::HashSet::new())
            .await;
    }

    pub async fn workspace_active_snapshot_worktree_has_vcs_watcher_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> bool {
        self.state
            .test_worktree_has_git_status_watcher(worktree_id)
            .await
    }

    pub async fn workspace_active_snapshot_hold_vcs_refresh_lock_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let refresh_lock = self.state.worktree_vcs_refresh_lock(worktree_id).await;
        refresh_lock.lock_owned().await
    }

    pub async fn workspace_active_snapshot_vcs_refresh_lock_token_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> usize {
        let refresh_lock = self.state.worktree_vcs_refresh_lock(worktree_id).await;
        Arc::as_ptr(&refresh_lock) as *const () as usize
    }

    pub async fn workspace_active_snapshot_verify_vcs_refresh_lock_eviction_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<()> {
        self.mark_worktree_vcs_active_for_test(worktree_id).await;
        let initial_lock = self.state.worktree_vcs_refresh_lock(worktree_id).await;
        self.mark_worktree_vcs_inactive_for_test(worktree_id).await;

        let next_lock = self.state.worktree_vcs_refresh_lock(worktree_id).await;
        if !Arc::ptr_eq(&initial_lock, &next_lock) {
            anyhow::bail!("worktree VCS reactivation should reuse an in-flight refresh lock");
        }

        let old_lock = Arc::downgrade(&initial_lock);
        drop(next_lock);
        drop(initial_lock);

        let replacement_lock = self.state.worktree_vcs_refresh_lock(worktree_id).await;
        if old_lock.upgrade().is_some() {
            anyhow::bail!("evicted refresh lock should be released once no refreshes are using it");
        }
        if Arc::strong_count(&replacement_lock) != 1 {
            anyhow::bail!(
                "replacement refresh lock should have one strong reference, got {}",
                Arc::strong_count(&replacement_lock)
            );
        }
        Ok(())
    }

    pub async fn workspace_active_snapshot_seed_ready_vcs_summary_for_test(
        &self,
        worktree: Worktree,
    ) -> anyhow::Result<WorktreeVcsSnapshot> {
        self.mark_worktree_vcs_active_for_test(worktree.id).await;
        self.refresh_worktree_vcs_summary_for_test(worktree.clone())
            .await?;
        self.worktree_vcs_snapshot(worktree.id)
            .await
            .ok_or_else(|| anyhow::anyhow!("expected VCS snapshot for worktree {:?}", worktree.id))
    }

    pub async fn reconcile_turn_terminal_state_for_test(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: TurnId,
        fallback_reason: &str,
    ) -> anyhow::Result<()> {
        daemon::scheduler::reconcile_turn_terminal_state(
            &self.state,
            session_id,
            run_id,
            turn_id,
            fallback_reason,
        )
        .await
    }

    pub async fn reconcile_turn_failed_on_provider_exit_for_test(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: TurnId,
        fallback_reason: &str,
    ) -> anyhow::Result<()> {
        daemon::scheduler::reconcile_turn_failed_on_provider_exit(
            &self.state,
            session_id,
            run_id,
            turn_id,
            fallback_reason,
        )
        .await
    }

    pub async fn mark_worktree_vcs_active_for_test(&self, worktree_id: WorktreeId) {
        let mut next_active = std::collections::HashSet::new();
        next_active.insert(worktree_id);
        self.state
            .test_update_worktree_vcs_activity(&std::collections::HashSet::new(), &next_active)
            .await;
    }

    pub async fn mark_worktree_vcs_inactive_for_test(&self, worktree_id: WorktreeId) {
        let mut previous_active = std::collections::HashSet::new();
        previous_active.insert(worktree_id);
        self.state
            .test_update_worktree_vcs_activity(&previous_active, &std::collections::HashSet::new())
            .await;
    }

    pub fn worktree_vcs_enabled_for_test(&self) -> bool {
        self.state.worktree_vcs_enabled()
    }

    pub async fn is_worktree_vcs_active_for_test(&self, worktree_id: WorktreeId) -> bool {
        self.state.is_worktree_vcs_active(worktree_id).await
    }

    pub async fn emit_worktree_vcs_snapshot_for_worktree(
        &self,
        worktree: &Worktree,
        include_commit_info: bool,
    ) -> anyhow::Result<()> {
        daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
            &self.state,
            worktree,
            include_commit_info,
        )
        .await
    }

    pub async fn request_worktree_vcs_refresh_for_test(
        &self,
        worktree: &Worktree,
        summary: bool,
        touched_files: bool,
    ) -> anyhow::Result<()> {
        daemon::git_status::request_worktree_vcs_refresh(
            &self.state,
            worktree,
            summary,
            touched_files,
        )
        .await
    }

    pub async fn refresh_worktree_vcs_summary_for_test(
        &self,
        worktree: Worktree,
    ) -> anyhow::Result<()> {
        daemon::git_status::refresh_worktree_vcs_summary(Arc::clone(&self.state), worktree).await
    }

    pub async fn run_git_status_watcher_for_test(&self, worktree: Worktree) -> anyhow::Result<()> {
        daemon::git_status::run_git_status_watcher(Arc::clone(&self.state), worktree).await
    }

    pub async fn load_worktree_for_test(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Worktree> {
        self.state
            .store_for_worktree(worktree_id)
            .await?
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree {worktree_id:?} not found"))
    }

    pub async fn workspace_primary_branch_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<String>> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        ctx_workspace_config::load_primary_branch(&store).await
    }

    pub async fn set_workspace_attachment_status_for_test(
        &self,
        workspace_id: WorkspaceId,
        attachment_id: WorkspaceAttachmentId,
        status: WorkspaceAttachmentStatus,
    ) -> anyhow::Result<()> {
        self.state
            .store_for_workspace(workspace_id)
            .await?
            .update_workspace_attachment_status(
                attachment_id,
                status,
                None,
                None,
                chrono::Utc::now(),
            )
            .await
    }

    pub async fn worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        self.state.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub async fn terminal_output_snapshot(&self, terminal_id: TerminalId) -> Option<Vec<u8>> {
        self.state
            .test_terminal_handle(terminal_id)
            .await
            .map(|handle| handle.output_snapshot())
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.state.sessions.remember_session_meta(session).await;
    }

    pub async fn publish_session_head_delta(
        &self,
        session: &Session,
        delta: SessionHeadDelta,
        bump_snapshot: bool,
    ) {
        self.state
            .test_publish_session_head_delta(session, delta, bump_snapshot)
            .await;
    }

    pub async fn set_provider_inactivity_timeout(&self, timeout: Duration) {
        self.state
            .test_set_provider_inactivity_timeout(timeout)
            .await;
    }

    pub async fn apply_provider_monitoring_settings_for_test(
        &self,
        settings: &Settings,
    ) -> anyhow::Result<()> {
        daemon::provider_guard::apply_settings(self.state.as_ref(), settings).await?;
        daemon::provider_restart::apply_settings(self.state.as_ref(), settings).await?;
        Ok(())
    }

    pub fn spawn_provider_monitoring_for_test(&self) {
        daemon::resource_telemetry::spawn_resource_telemetry(Arc::clone(&self.state));
        daemon::provider_guard::spawn_provider_guard(Arc::clone(&self.state));
        daemon::provider_restart::spawn_provider_restart(Arc::clone(&self.state));
    }

    pub async fn prepare_workspace_harness_for_test(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution_settings: &ExecutionSettings,
    ) -> anyhow::Result<()> {
        self.state
            .test_prepare_harness(workspace, worktree, execution_settings)
            .await
            .map(|_| ())
    }

    pub async fn workspace_harness_egress_guard_for_test(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<bool>> {
        Ok(self
            .state
            .test_harness_container_status(workspace_id)
            .await?
            .and_then(|status| status.egress_guard))
    }

    pub async fn materialize_workspace_attachments_for_test(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        configs: impl IntoIterator<Item = ctx_workspace_attachments::AttachmentConfig>,
    ) -> anyhow::Result<Vec<WorktreeAttachmentMount>> {
        for config in configs {
            ctx_workspace_services::workspace_attachments::upsert_workspace_attachment(
                self.state.as_ref(),
                workspace.id,
                config,
            )
            .await?;
        }

        let sync = ctx_workspace_services::workspace_attachments::sync_workspace_attachments(
            self.state.as_ref(),
            workspace,
            false,
        )
        .await?;
        for plan in sync.plans {
            ctx_workspace_services::workspace_attachments::run_attachment_materialization(
                self.state.as_ref(),
                workspace,
                plan.id,
                plan.refresh,
            )
            .await?;
        }

        daemon::workspaces::ensure_worktree_attachment_mounts_if_materialized(
            self.state.as_ref(),
            workspace,
            worktree,
        )
        .await
    }

    pub async fn replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        self.state
            .providers
            .replace_provider_statuses(statuses)
            .await;
    }

    pub async fn refresh_provider_statuses(&self) -> anyhow::Result<()> {
        ctx_managed_installs::refresh_provider_statuses(self.state.as_ref()).await
    }

    pub async fn upsert_provider_status(&self, provider_id: String, status: ProviderStatus) {
        self.state
            .providers
            .upsert_provider_status(provider_id, status)
            .await;
    }

    pub fn publish_storage_guard(&self, status: StorageGuardStatus) {
        self.state.test_publish_storage_guard(status);
    }

    pub async fn stop_mobile_tunnel(&self) {
        self.state.test_stop_mobile_tunnel().await;
    }

    pub fn mobile_access_for_test(&self) -> TestMobileAccessForTest<'_> {
        TestMobileAccessForTest { state: &self.state }
    }

    pub async fn issue_provider_session_mcp_token(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    ) -> String {
        daemon::issue_provider_session_mcp_token(&self.state, session_id, workspace_id, worktree_id)
            .await
    }

    pub async fn issue_provider_session_mcp_token_with_capabilities(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        capabilities: ctx_mcp_auth::McpAuthCapabilities,
    ) -> String {
        daemon::issue_provider_session_mcp_token_with_capabilities(
            &self.state,
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        )
        .await
    }

    pub async fn revoke_provider_session_mcp_token(&self, token: &str) -> bool {
        daemon::revoke_provider_session_mcp_token(&self.state, token).await
    }

    pub async fn test_with_provider_usage_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, provider_usage::ProviderUsageSnapshot>) -> R,
    ) -> R {
        self.state.test_with_provider_usage_cache(f).await
    }

    pub async fn test_with_provider_options_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderOptions>) -> R,
    ) -> R {
        self.state.test_with_provider_options_cache(f).await
    }

    pub async fn test_with_provider_verify_cache<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, CachedProviderVerify>) -> R,
    ) -> R {
        self.state.test_with_provider_verify_cache(f).await
    }

    pub async fn seed_provider_options_probe_cache_for_test(
        &self,
        key: &str,
        provider_id: &str,
        probe_ok: bool,
    ) {
        self.state
            .test_with_provider_options_cache(|cache| {
                cache.insert(
                    key.to_string(),
                    CachedProviderOptions {
                        cached_at: std::time::Instant::now(),
                        value: serde_json::json!({
                            "provider_id": provider_id,
                            "probe_ok": probe_ok,
                        }),
                    },
                );
            })
            .await;
    }

    pub async fn provider_options_probe_cache_contains_for_test(&self, key: &str) -> bool {
        self.state
            .test_with_provider_options_cache(|cache| cache.contains_key(key))
            .await
    }

    pub async fn seed_provider_verify_cache_status_for_test(&self, key: &str, status: &str) {
        self.state
            .test_with_provider_verify_cache(|cache| {
                cache.insert(
                    key.to_string(),
                    CachedProviderVerify {
                        cached_at: std::time::Instant::now(),
                        value: serde_json::json!({ "status": status }),
                    },
                );
            })
            .await;
    }

    pub async fn provider_verify_cache_contains_for_test(&self, key: &str) -> bool {
        self.state
            .test_with_provider_verify_cache(|cache| cache.contains_key(key))
            .await
    }

    pub async fn seed_provider_usage_success_for_test(
        &self,
        provider_id: &str,
        source: &str,
        payload: serde_json::Value,
    ) {
        self.state
            .test_with_provider_usage_cache(|cache| {
                cache.insert(
                    provider_id.to_string(),
                    provider_usage::ProviderUsageSnapshot {
                        provider_id: provider_id.to_string(),
                        source: source.to_string(),
                        fetched_at: chrono::Utc::now(),
                        payload: Some(payload),
                        error: None,
                    },
                );
            })
            .await;
    }

    pub async fn seed_mcp_parent_session_for_test(
        &self,
        repo_path: &Path,
        base_commit: String,
        provider_id: &str,
        model_id: &str,
    ) -> anyhow::Result<Session> {
        let workspace: Workspace = self
            .state
            .global_store()
            .create_workspace(
                "test".into(),
                repo_path.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await?;
        let store = self.state.store_for_workspace(workspace.id).await?;
        let worktree: Worktree = store
            .create_worktree(
                workspace.id,
                repo_path.to_string_lossy().to_string(),
                base_commit,
                None,
            )
            .await?;
        let task: Task = store.create_task(workspace.id, "task".into(), None).await?;
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ExecutionEnvironment::Host,
                provider_id.into(),
                model_id.into(),
                "assistant".into(),
                None,
                None,
                None,
            )
            .await?;
        self.state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await?;
        self.state
            .global_store()
            .upsert_workspace_task_index(task.id, workspace.id)
            .await?;
        Ok(session)
    }

    pub async fn mcp_parent_session_events_for_test(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Vec<SessionEvent>> {
        self.state
            .store_for_session(session_id)
            .await?
            .list_session_events(session_id)
            .await
    }

    pub async fn mcp_subagent_sessions_for_test(
        &self,
        parent_session_id: SessionId,
    ) -> anyhow::Result<Vec<SessionSummary>> {
        self.state
            .store_for_session(parent_session_id)
            .await?
            .list_subagent_sessions(parent_session_id)
            .await
    }

    pub async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        self.state.start_install(provider_id, target).await
    }

    pub async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        self.state.find_running_install(provider_id, target).await
    }

    pub async fn install_provider_with_progress(
        &self,
        install_id: InstallId,
        provider_id: String,
        target: InstallTarget,
    ) -> anyhow::Result<()> {
        let state: Arc<ctx_managed_installs::AppState> = self.state.clone();
        ctx_managed_installs::install_provider_with_progress(state, install_id, provider_id, target)
            .await
    }

    pub async fn install_title_generation_local_with_progress(
        &self,
        install_id: InstallId,
    ) -> anyhow::Result<()> {
        let state: Arc<ctx_managed_installs::AppState> = self.state.clone();
        ctx_managed_installs::install_title_generation_local_with_progress(state, install_id).await
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        self.state.emit_install_event(install_id, event).await;
    }

    pub async fn get_install_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        self.state.get_install_info(install_id).await
    }

    pub async fn tracked_install_ids(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Vec<InstallId> {
        self.state
            .test_tracked_install_ids(provider_id, target)
            .await
    }

    pub async fn has_target_provider_adapter(&self, cache_key: &str) -> bool {
        self.state.test_has_target_provider_adapter(cache_key).await
    }

    pub async fn target_provider_adapter_cache_keys(&self) -> Vec<String> {
        self.state
            .test_target_provider_adapter_entries()
            .await
            .into_iter()
            .map(|(cache_key, _)| cache_key)
            .collect()
    }

    pub async fn get_install_polling_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        self.state.get_install_polling_info(install_id).await
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        self.state.get_install_events(install_id).await
    }

    pub async fn provider_login_session_caches_empty(&self) -> bool {
        let gemini = self
            .state
            .test_with_gemini_login_sessions(|map| map.is_empty())
            .await;
        let qwen = self
            .state
            .test_with_qwen_login_sessions(|map| map.is_empty())
            .await;
        let amp = self
            .state
            .test_with_amp_login_sessions(|map| map.is_empty())
            .await;
        let mistral = self
            .state
            .test_with_mistral_login_sessions(|map| map.is_empty())
            .await;
        let kimi = self
            .state
            .test_with_kimi_login_sessions(|map| map.is_empty())
            .await;
        let claude = self
            .state
            .test_with_claude_login_sessions(|map| map.is_empty())
            .await;
        let codex = self
            .state
            .test_with_codex_login_sessions(|map| map.is_empty())
            .await;
        let cursor = self
            .state
            .test_with_cursor_login_sessions(|map| map.is_empty())
            .await;
        gemini && qwen && amp && mistral && kimi && claude && codex && cursor
    }
}

impl TestMobileAccessForTest<'_> {
    fn token_hash(token: &str) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn token_prefix(token: &str) -> String {
        token.chars().take(8).collect()
    }

    async fn create_profile_with_token_hash(
        &self,
        label: &str,
        base_url: &str,
        token_hash: String,
        token_prefix: String,
        scopes: &[&str],
    ) -> anyhow::Result<MobileConnectionProfile> {
        self.state
            .global_store()
            .create_mobile_connection_profile(
                label.to_string(),
                base_url.to_string(),
                token_hash,
                token_prefix,
                scopes.iter().map(|scope| (*scope).to_string()).collect(),
            )
            .await
    }

    pub async fn seed_mobile_api_profile_for_test(
        &self,
        token: &str,
        scopes: &[&str],
    ) -> anyhow::Result<MobileConnectionProfile> {
        self.create_profile_with_token_hash(
            "mobile",
            "https://example.com",
            Self::token_hash(token),
            Self::token_prefix(token),
            scopes,
        )
        .await
    }

    pub async fn seed_empty_managed_mobile_access_profile_for_test(
        &self,
    ) -> anyhow::Result<MobileConnectionProfile> {
        self.create_profile_with_token_hash(
            "Managed Mobile Access",
            "https://legacy.example.com",
            "legacy-token-hash".to_string(),
            "legacy-m".to_string(),
            &[],
        )
        .await
    }

    pub async fn mobile_profile_for_test(
        &self,
        profile_id: ConnectionProfileId,
    ) -> anyhow::Result<Option<MobileConnectionProfile>> {
        self.state
            .global_store()
            .get_mobile_connection_profile(profile_id)
            .await
    }

    pub async fn seed_default_mobile_access_config_for_test(
        &self,
        profile_id: ConnectionProfileId,
        enabled: bool,
        daemon_public_key: String,
        daemon_private_key: String,
    ) -> anyhow::Result<MobileAccessConfig> {
        self.seed_mobile_access_config_for_test(
            profile_id,
            "tunnel-1",
            "https://example.com",
            "https://relay.example.com",
            "secret",
            daemon_public_key,
            daemon_private_key,
            enabled,
        )
        .await
    }

    pub async fn seed_legacy_mobile_access_config_for_test(
        &self,
        profile_id: ConnectionProfileId,
        enabled: bool,
        daemon_public_key: String,
        daemon_private_key: String,
    ) -> anyhow::Result<MobileAccessConfig> {
        self.seed_mobile_access_config_for_test(
            profile_id,
            "legacy-tunnel",
            "https://legacy.example.com",
            "https://legacy-relay.example.com",
            "legacy-secret",
            daemon_public_key,
            daemon_private_key,
            enabled,
        )
        .await
    }

    async fn seed_mobile_access_config_for_test(
        &self,
        profile_id: ConnectionProfileId,
        tunnel_id: &str,
        public_base_url: &str,
        relay_base_url: &str,
        tunnel_secret: &str,
        daemon_public_key: String,
        daemon_private_key: String,
        enabled: bool,
    ) -> anyhow::Result<MobileAccessConfig> {
        self.state
            .global_store()
            .upsert_mobile_access_config(MobileAccessConfig {
                id: "default".to_string(),
                profile_id,
                tunnel_id: tunnel_id.to_string(),
                public_base_url: public_base_url.to_string(),
                relay_base_url: relay_base_url.to_string(),
                tunnel_secret: tunnel_secret.to_string(),
                daemon_public_key,
                daemon_private_key,
                enabled,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
    }

    pub async fn mobile_access_config_for_test(
        &self,
    ) -> anyhow::Result<Option<MobileAccessConfig>> {
        self.state.global_store().get_mobile_access_config().await
    }

    pub async fn seed_mobile_device_for_test(
        &self,
        device_id: MobileDeviceId,
        profile_id: ConnectionProfileId,
        public_key: String,
        device_label: &str,
    ) -> anyhow::Result<MobileDeviceRegistration> {
        self.state
            .global_store()
            .upsert_mobile_device(
                device_id,
                profile_id,
                MobileDeviceUpsert {
                    device_label: Some(device_label.to_string()),
                    platform: Some("ios".to_string()),
                    push_token: None,
                    push_provider: None,
                    public_key: Some(public_key),
                    app_version: Some("1.0.0".to_string()),
                },
            )
            .await
    }

    pub async fn mobile_device_for_test(
        &self,
        device_id: MobileDeviceId,
    ) -> anyhow::Result<Option<MobileDeviceRegistration>> {
        self.state.global_store().get_mobile_device(device_id).await
    }

    pub async fn seed_mobile_pairing_token_for_test(
        &self,
        id: &str,
        token: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<String> {
        let token_hash = Self::token_hash(token);
        self.state
            .global_store()
            .insert_mobile_pairing_token(id, &token_hash, expires_at)
            .await?;
        Ok(token_hash)
    }

    pub async fn consume_mobile_pairing_token_hash_for_test(
        &self,
        token_hash: &str,
    ) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .consume_mobile_pairing_token(token_hash)
            .await
    }
}

/// Workspace-runtime tests historically used a sandbox-specific name for the
/// shared sandbox-runtime lock. Keep that lock separate from the broader
/// process-env lock so long-lived runtime jobs are not queued behind unrelated
/// bundle/env tests.
pub fn sandbox_cli_env_test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

#[cfg(unix)]
pub fn write_running_container_sandbox_cli_shim(
    dir: &Path,
    log_path: &Path,
    container_name: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("sandbox-cli-running-container-test.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  suffix=${{2#ctx-harness-}}\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"ctx-ws-%s\",\"Destination\":\"/ctx/ws\"}}]}}]\\n' \"$suffix\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  shift\n  while [ \"$#\" -gt 0 ]; do\n    case \"$1\" in\n      --interactive)\n        shift\n        ;;\n      --user|--workdir|--env)\n        shift 2\n        ;;\n      *)\n        break\n        ;;\n    esac\n  done\n  container_name=\"$1\"\n  shift\n  command=\"$1\"\n  shift\n  if [ \"$container_name\" != \"{container}\" ]; then\n    echo \"unexpected container: $container_name\" >&2\n    exit 1\n  fi\n  if [ \"$command\" = \"tar\" ] && [ \"$1\" = \"-xf\" ] && [ \"$2\" = \"-\" ]; then\n    cat >/dev/null\n    exit 0\n  fi\n  if [ \"$command\" = \"git\" ] && [ \"$1\" = \"checkout\" ]; then\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-u\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-g\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"df\" ] && [ \"$1\" = \"-Pk\" ]; then\n    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n'\n    printf 'overlay 10485760 1024 7340032 1%% /ctx/ws\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"sh\" ] && [ \"$1\" = \"-lc\" ]; then\n    case \"$2\" in\n      *\"git rev-parse --is-inside-work-tree\"*)\n        printf 'true\\n'\n        exit 0\n        ;;\n      *)\n        exit 0\n        ;;\n    esac\n  fi\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write running-container sandbox CLI shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod running-container sandbox CLI shim");
    path
}

pub fn avf_linux_runtime_manager_test_sandbox_cli_path(dir: &Path) -> PathBuf {
    dir.join("ctx-avf-linux-sandbox-cli-runtime-manager-test.sh")
}
