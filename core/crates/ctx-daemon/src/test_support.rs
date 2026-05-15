use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ctx_core::ids::{
    ConnectionProfileId, MessageId, MobileDeviceId, RunId, SessionEventId, SessionId, TaskId,
    TerminalId, TurnId, WorkspaceAttachmentId, WorkspaceId, WorktreeId,
};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, MobileConnectionProfile, MobileDeviceRegistration,
    Session, SessionEvent, SessionEventType, SessionHeadDelta, SessionHeadSnapshot, SessionTurn,
    SessionTurnStatus, Workspace, WorkspaceActiveTaskSummary, WorkspaceAttachmentStatus, Worktree,
    WorktreeAttachmentMount, WorktreeVcsSnapshot,
};
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::{provider_usage, CachedProviderOptions, CachedProviderVerify};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_settings_model::{ExecutionSettings, Settings};
use ctx_storage_admission::StorageGuardStatus;
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};
use ctx_store::{Store, StoreManager};
use sha2::Digest;
use tokio::sync::Mutex as AsyncMutex;

use crate::daemon::{self, AppRuntimeFlags, DaemonHandle, DaemonState};

#[derive(Clone)]
pub struct TestDaemon {
    state: Arc<DaemonState>,
}

pub struct TestMobileAccessForTest<'a> {
    state: &'a Arc<DaemonState>,
}

fn fixed_test_utc(offset_seconds: i64) -> chrono::DateTime<chrono::Utc> {
    let base = chrono::DateTime::from_timestamp(1735689600, 0)
        .expect("fixed test timestamp should be valid");
    base + chrono::Duration::seconds(offset_seconds)
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

    pub fn spawn_merge_queue_runner(&self) {
        daemon::merge_queue::spawn_merge_queue_runner(Arc::clone(&self.state));
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), daemon::workspaces::WorkspaceHydrationError> {
        self.state
            .ensure_workspace_active_snapshot_hydrated(workspace_id)
            .await
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
