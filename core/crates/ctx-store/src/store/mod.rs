use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use conversions::*;
use conversions_tools::*;
use ctx_core::ids::*;
use ctx_core::models::*;
use metrics_and_runtime::*;
use serde::Serialize;
use serde_json::Value;
use sqlx::sqlite::{SqliteArguments, SqlitePoolOptions, SqliteRow};
use sqlx::{Pool, Row, Sqlite};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::info;

use head_kind::*;
pub(crate) use head_projection::*;
pub(crate) use lease::StoreLeaseGuard;

mod head_kind;
mod head_projection;
mod lease;
#[cfg(test)]
mod tests_runtime_shutdown;
mod turn_projection;
#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
    event_log: Arc<EventLogRuntime>,
    active_head_projection: Arc<ActiveHeadProjectionRuntime>,
    write_gate: Arc<Mutex<()>>,
    _lease_guard: Option<Arc<dyn StoreLeaseGuard>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoreStats {
    pub pool_size: usize,
    pub pool_idle: usize,
}

pub struct SessionRetentionPruneStats {
    pub tool_summaries_deleted: u64,
    pub turn_thoughts_cleared: u64,
}
pub fn is_unique_constraint_violation(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        let Some(sqlx::Error::Database(db_err)) = cause.downcast_ref::<sqlx::Error>() else {
            continue;
        };
        if let Some(code) = db_err.code() {
            if matches!(code.as_ref(), "1555" | "2067") {
                return true;
            }
        }
        let message = db_err.message();
        if message.contains("UNIQUE constraint failed") || message.contains("PRIMARY KEY") {
            return true;
        }
    }
    false
}

const SESSION_HEAD_MAX_TURNS: u32 = 200;
const SESSION_HEAD_MESSAGE_LIMIT: usize = 200;
const SESSION_HEAD_EVENT_LIMIT: usize = 200;
const SESSION_HEAD_BYTE_LIMIT: usize = 1_500_000;
const ACTIVE_SNAPSHOT_HEAD_LIMIT: u32 = 5;
const SESSION_HEAD_ARCHIVED_TURN_LIMIT: u32 = 50;
const SESSION_REASONING_EFFORT_MIGRATION_VERSION: i64 = 46;
const TOOL_DISPLAY_FIELDS_MIGRATION_VERSION: i64 = 47;
const TOOL_DISPLAY_FIELDS_MIGRATION_DESCRIPTION: &str = "tool display fields";
const WORKSPACE_MESSAGE_INDEX_MIGRATION_VERSION: i64 = 51;
const WORKSPACE_MESSAGE_INDEX_MIGRATION_DESCRIPTION: &str = "drop workspace message index";
// Keep stream-only seq values within JS safe integer range.
const STREAM_ONLY_EVENT_SEQ_START: i64 = -(1_i64 << 52);
static STREAM_ONLY_EVENT_SEQ: AtomicI64 = AtomicI64::new(STREAM_ONLY_EVENT_SEQ_START);
static STORE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

fn next_stream_only_event_seq() -> i64 {
    STREAM_ONLY_EVENT_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn retain_messages_for_turns(messages: &mut Vec<Message>, turns: &[SessionTurn]) {
    if turns.is_empty() {
        messages.clear();
        return;
    }
    let mut allowed = std::collections::HashSet::new();
    for turn in turns {
        allowed.insert(turn.turn_id);
    }
    messages.retain(|msg| match msg.turn_id {
        Some(turn_id) => allowed.contains(&turn_id),
        None => true,
    });
}

fn retain_tool_summaries_for_turns(
    tool_summaries: &mut Vec<SessionTurnToolSummary>,
    turns: &[SessionTurn],
) {
    if turns.is_empty() {
        tool_summaries.clear();
        return;
    }
    let mut allowed = std::collections::HashSet::new();
    for turn in turns {
        allowed.insert(turn.turn_id);
    }
    tool_summaries.retain(|tool| allowed.contains(&tool.turn_id));
}

fn strip_snapshot_partials(turns: &mut [SessionTurn], events: &mut Vec<SessionEvent>) {
    for turn in turns.iter_mut() {
        turn.assistant_partial = None;
        turn.thought_partial = None;
    }
    if events.is_empty() {
        return;
    }
    events.retain(|event| {
        !matches!(
            event.event_type,
            SessionEventType::AssistantChunk
                | SessionEventType::AssistantComplete
                | SessionEventType::ThoughtChunk
        )
    });
}

#[allow(clippy::too_many_arguments)]
fn trim_session_head_window(
    turns: &mut Vec<SessionTurn>,
    messages: &mut Vec<Message>,
    tool_summaries: &mut Vec<SessionTurnToolSummary>,
    events: &mut Vec<SessionEvent>,
    has_more_turns: &mut bool,
    turn_limit: usize,
    message_limit: usize,
    event_limit: usize,
    byte_limit: usize,
) -> SessionHeadWindow {
    let mut truncated = false;

    while turns.len() > turn_limit {
        turns.remove(0);
        truncated = true;
        *has_more_turns = true;
    }
    retain_messages_for_turns(messages, turns);
    retain_tool_summaries_for_turns(tool_summaries, turns);

    // Session heads are turn-atomic under the current history contract. Once only
    // one turn remains, preserve it even if its message count exceeds the soft cap.
    while messages.len() > message_limit && turns.len() > 1 {
        turns.remove(0);
        truncated = true;
        *has_more_turns = true;
        retain_messages_for_turns(messages, turns);
        retain_tool_summaries_for_turns(tool_summaries, turns);
    }

    if events.len() > event_limit {
        let drop = events.len() - event_limit;
        events.drain(0..drop);
        truncated = true;
    }

    loop {
        let bytes = head_window_bytes(turns, tool_summaries, events, messages);
        if bytes <= byte_limit || (turns.is_empty() && events.is_empty()) {
            break;
        }
        if turns.len() > 1 {
            turns.remove(0);
            truncated = true;
            *has_more_turns = true;
            retain_messages_for_turns(messages, turns);
            retain_tool_summaries_for_turns(tool_summaries, turns);
            continue;
        }
        if !events.is_empty() {
            events.remove(0);
            truncated = true;
            continue;
        }
        // Preserve the newest remaining turn even if it exceeds soft byte limits.
        break;
    }

    let bytes = head_window_bytes(turns, tool_summaries, events, messages);
    SessionHeadWindow {
        turn_limit: turn_limit as i64,
        message_limit: message_limit as i64,
        event_limit: event_limit as i64,
        byte_limit: byte_limit as i64,
        turn_count: turns.len() as i64,
        message_count: messages.len() as i64,
        event_count: events.len() as i64,
        bytes: bytes as i64,
        truncated,
    }
}

fn session_head_kind_to_str(kind: SessionHeadKind) -> &'static str {
    match kind {
        SessionHeadKind::Active => "active",
        SessionHeadKind::Archived => "archived",
    }
}

fn session_head_limits(kind: SessionHeadKind, turn_limit: u32) -> SessionHeadLimits {
    let max_turns = match kind {
        SessionHeadKind::Active => SESSION_HEAD_MAX_TURNS,
        SessionHeadKind::Archived => SESSION_HEAD_ARCHIVED_TURN_LIMIT,
    };
    let turn_limit = turn_limit.clamp(1, max_turns) as usize;
    SessionHeadLimits {
        turn_limit,
        message_limit: SESSION_HEAD_MESSAGE_LIMIT,
        event_limit: SESSION_HEAD_EVENT_LIMIT,
        byte_limit: SESSION_HEAD_BYTE_LIMIT,
    }
}

fn apply_session_head_limits(
    mut head: SessionHead,
    limits: SessionHeadLimits,
    include_events: bool,
) -> SessionHead {
    if !include_events {
        head.events.clear();
    }
    strip_snapshot_partials(&mut head.turns, &mut head.events);
    let mut has_more_turns = head.has_more_turns;
    let head_window = trim_session_head_window(
        &mut head.turns,
        &mut head.messages,
        &mut head.tool_summaries,
        &mut head.events,
        &mut has_more_turns,
        limits.turn_limit,
        limits.message_limit,
        limits.event_limit,
        limits.byte_limit,
    );
    head.has_more_turns = has_more_turns;
    head.head_window = head_window;
    head
}

fn serialize_bootstrap_status(status: &WorktreeBootstrapStatus) -> &'static str {
    match status {
        WorktreeBootstrapStatus::Success => "success",
        WorktreeBootstrapStatus::Failed => "failed",
        WorktreeBootstrapStatus::Timeout => "timeout",
    }
}

fn parse_bootstrap_status(raw: Option<String>) -> Option<WorktreeBootstrapStatus> {
    match raw.as_deref() {
        Some("success") => Some(WorktreeBootstrapStatus::Success),
        Some("failed") => Some(WorktreeBootstrapStatus::Failed),
        Some("timeout") => Some(WorktreeBootstrapStatus::Timeout),
        _ => None,
    }
}

fn vcs_kind_to_str(kind: &VcsKind) -> &'static str {
    match kind {
        VcsKind::Git => "git",
        VcsKind::Jj => "jj",
        VcsKind::Hg => "hg",
        VcsKind::Svn => "svn",
        VcsKind::P4 => "p4",
        VcsKind::Other => "other",
    }
}

fn parse_vcs_kind(raw: Option<String>) -> Option<VcsKind> {
    match raw.as_deref() {
        Some("git") => Some(VcsKind::Git),
        Some("jj") => Some(VcsKind::Jj),
        Some("hg") => Some(VcsKind::Hg),
        Some("svn") => Some(VcsKind::Svn),
        Some("p4") => Some(VcsKind::P4),
        Some("other") => Some(VcsKind::Other),
        Some(_) => Some(VcsKind::Other),
        None => None,
    }
}

fn parse_optional_session_id(raw: Option<String>) -> Option<SessionId> {
    raw.and_then(|value| uuid::Uuid::parse_str(&value).ok())
        .map(SessionId)
}

pub struct WorktreeBootstrapResultUpdate {
    pub worktree_id: WorktreeId,
    pub status: WorktreeBootstrapStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub exit_code: Option<i64>,
    pub timeout_sec: Option<i64>,
    pub error: Option<String>,
    pub log_path: Option<String>,
    pub log_truncated: Option<bool>,
    pub command: Option<String>,
    pub script_path: Option<String>,
}

pub struct SessionTurnToolCountDeltas {
    pub total: i64,
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}

pub struct MobileDeviceUpsert {
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub push_token: Option<String>,
    pub push_provider: Option<String>,
    pub public_key: Option<String>,
    pub app_version: Option<String>,
}

pub struct MobileAccessConfig {
    pub id: String,
    pub profile_id: ConnectionProfileId,
    pub tunnel_id: String,
    pub public_base_url: String,
    pub relay_base_url: String,
    pub tunnel_secret: String,
    pub daemon_public_key: String,
    pub daemon_private_key: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct RuntimeSettingsDocument {
    pub id: String,
    pub schema_version: i64,
    pub settings_json: String,
    pub updated_at: DateTime<Utc>,
}

impl Store {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_sqlite(path, None).await
    }

    pub async fn open_sqlite(path: impl AsRef<Path>, max_connections: Option<u32>) -> Result<Self> {
        let path = path.as_ref();
        let path_str = path.to_string_lossy();
        if path_str != ":memory:" {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if !path.exists() {
                let _ = tokio::fs::File::create(path).await?;
            }
        }
        let sqlite_url = format!("sqlite://{}", path.to_string_lossy());
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections.unwrap_or(5))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA synchronous = NORMAL")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&sqlite_url)
            .await?;
        repair_historical_tool_order_seq_migration_version(&pool).await?;
        ensure_sqlite_journal_mode_wal(&pool).await?;
        repair_duplicate_tool_display_migration_version(&pool).await?;
        repair_workspace_message_index_migration_versions(&pool).await?;
        STORE_MIGRATOR.run(&pool).await?;
        let event_log = Arc::new(EventLogRuntime::load(&pool).await?);
        let active_head_projection = Arc::new(ActiveHeadProjectionRuntime::new());
        let store = Self {
            pool,
            event_log,
            active_head_projection,
            write_gate: Arc::new(Mutex::new(())),
            _lease_guard: None,
        };
        store.event_log.start_persister(store.clone())?;
        store
            .active_head_projection
            .start_projector(store.clone())?;
        Ok(store)
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    pub fn stats(&self) -> StoreStats {
        StoreStats {
            pool_size: self.pool.size() as usize,
            pool_idle: self.pool.num_idle(),
        }
    }

    pub async fn close(&self) {
        if self._lease_guard.is_some() {
            tracing::debug!("ignoring close() on lease-backed store handle");
            return;
        }
        if let Err(err) = self.event_log.shutdown().await {
            tracing::warn!("event log shutdown failed during close: {err:#}");
        }
        if let Err(err) = self.active_head_projection.shutdown().await {
            tracing::warn!("active head projection shutdown failed during close: {err:#}");
        }
        self.pool.close().await;
    }

    pub fn close_blocking(&self) {
        if self._lease_guard.is_some() {
            tracing::debug!("ignoring close_blocking() on lease-backed store handle");
            return;
        }
        if let Err(err) = self.event_log.shutdown_blocking() {
            tracing::warn!("event log shutdown failed during blocking close: {err:#}");
        }
        if let Err(err) = self.active_head_projection.shutdown_blocking() {
            tracing::warn!("active head projection shutdown failed during blocking close: {err:#}");
        }
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(async {
                self.pool.close().await;
            }),
            Err(err) => {
                tracing::warn!("failed to build runtime for blocking pool close: {err:#}");
                futures::executor::block_on(async {
                    self.pool.close().await;
                });
            }
        }
    }

    fn sql(&self, sql: &'static str) -> &'static str {
        sql
    }

    fn query<'q>(
        &'q self,
        sql: &'static str,
    ) -> sqlx::query::Query<'q, Sqlite, SqliteArguments<'q>> {
        let sql: &'q str = self.sql(sql);
        sqlx::query(sql)
    }

    fn query_scalar<'q, T>(
        &'q self,
        sql: &'static str,
    ) -> sqlx::query::QueryScalar<'q, Sqlite, T, SqliteArguments<'q>>
    where
        for<'r> T: sqlx::Decode<'r, Sqlite> + sqlx::Type<Sqlite> + Send,
    {
        let sql: &'q str = self.sql(sql);
        sqlx::query_scalar(sql)
    }

    fn rewrite_sql<'a>(&self, sql: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(sql)
    }

    pub async fn prune_session_data_older_than_days(
        &self,
        retention_days: u64,
    ) -> Result<SessionRetentionPruneStats> {
        if retention_days == 0 {
            return Ok(SessionRetentionPruneStats {
                tool_summaries_deleted: 0,
                turn_thoughts_cleared: 0,
            });
        }
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let tool_summaries_deleted = if disable_tool_summary_persistence() {
            0
        } else {
            self.query(
                r#"DELETE FROM session_turn_tools
               WHERE session_id IN (
                   SELECT s.id
                   FROM sessions s
                   JOIN tasks t ON t.id = s.task_id
                   WHERE t.archived_at IS NOT NULL
                     AND t.archived_at < ?
               )"#,
            )
            .bind(&cutoff_str)
            .execute(&self.pool)
            .await?
            .rows_affected()
        };

        // Keep the row (turn metadata is still useful), but remove old final thoughts.
        let turn_thoughts_cleared = self
            .query(
                r#"UPDATE session_turns
               SET thought_partial = NULL
               WHERE thought_partial IS NOT NULL
                 AND session_id IN (
                     SELECT s.id
                     FROM sessions s
                     JOIN tasks t ON t.id = s.task_id
                     WHERE t.archived_at IS NOT NULL
                       AND t.archived_at < ?
                 )"#,
            )
            .bind(&cutoff_str)
            .execute(&self.pool)
            .await?
            .rows_affected();
        record_write(WriteMetricTable::SessionTurns, turn_thoughts_cleared, 0);

        Ok(SessionRetentionPruneStats {
            tool_summaries_deleted,
            turn_thoughts_cleared,
        })
    }
}

async fn repair_duplicate_tool_display_migration_version(pool: &Pool<Sqlite>) -> Result<()> {
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if migrations_table_exists == 0 {
        return Ok(());
    }

    let tool_display_version = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE description = ? LIMIT 1",
    )
    .bind(TOOL_DISPLAY_FIELDS_MIGRATION_DESCRIPTION)
    .fetch_optional(pool)
    .await?;

    if tool_display_version != Some(SESSION_REASONING_EFFORT_MIGRATION_VERSION) {
        return Ok(());
    }

    let renamed_version_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
    )
    .bind(TOOL_DISPLAY_FIELDS_MIGRATION_VERSION)
    .fetch_one(pool)
    .await?;

    if renamed_version_exists == 0 {
        sqlx::query(
            "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND description = ?",
        )
        .bind(TOOL_DISPLAY_FIELDS_MIGRATION_VERSION)
        .bind(SESSION_REASONING_EFFORT_MIGRATION_VERSION)
        .bind(TOOL_DISPLAY_FIELDS_MIGRATION_DESCRIPTION)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ? AND description = ?")
            .bind(SESSION_REASONING_EFFORT_MIGRATION_VERSION)
            .bind(TOOL_DISPLAY_FIELDS_MIGRATION_DESCRIPTION)
            .execute(pool)
            .await?;
    }

    Ok(())
}

async fn ensure_sqlite_journal_mode_wal(pool: &Pool<Sqlite>) -> Result<()> {
    let current_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await?;
    if current_mode.eq_ignore_ascii_case("wal") {
        return Ok(());
    }

    let _: String = sqlx::query_scalar("PRAGMA journal_mode = WAL")
        .fetch_one(pool)
        .await?;
    Ok(())
}

async fn repair_workspace_message_index_migration_versions(pool: &Pool<Sqlite>) -> Result<()> {
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if migrations_table_exists == 0 {
        return Ok(());
    }

    let rows =
        sqlx::query("SELECT version FROM _sqlx_migrations WHERE description = ? ORDER BY version")
            .bind(WORKSPACE_MESSAGE_INDEX_MIGRATION_DESCRIPTION)
            .fetch_all(pool)
            .await?;

    for row in rows {
        let version: i64 = row.try_get("version")?;
        if version == WORKSPACE_MESSAGE_INDEX_MIGRATION_VERSION {
            continue;
        }

        let repaired_version_exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
        )
        .bind(WORKSPACE_MESSAGE_INDEX_MIGRATION_VERSION)
        .fetch_one(pool)
        .await?;

        if repaired_version_exists == 0 {
            sqlx::query(
                "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND description = ?",
            )
            .bind(WORKSPACE_MESSAGE_INDEX_MIGRATION_VERSION)
            .bind(version)
            .bind(WORKSPACE_MESSAGE_INDEX_MIGRATION_DESCRIPTION)
            .execute(pool)
            .await?;
        } else {
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ? AND description = ?")
                .bind(version)
                .bind(WORKSPACE_MESSAGE_INDEX_MIGRATION_DESCRIPTION)
                .execute(pool)
                .await?;
        }
    }

    Ok(())
}

mod artifacts_blobs;
mod attachments;
mod conversions;
mod conversions_tools;
mod events;
mod messages;
mod messages_snapshots;
mod messages_workspace_active;
mod messages_workspace_index;
mod metrics_and_runtime;
mod migration_repairs;
mod mobile;
mod sandbox_bindings;
mod sessions;
mod tasks;
mod turns;
mod turns_session_heads;
mod workspace;
mod worktrees;

use migration_repairs::repair_historical_tool_order_seq_migration_version;

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    pub(crate) async fn setup_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();
        (dir, store)
    }

    pub(crate) async fn create_session_with_turn(
        store: &Store,
        assistant_partial: Option<String>,
    ) -> (Session, TurnId) {
        let ws = store
            .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
            .await
            .unwrap();
        let task = store.create_task(ws.id, "task".into(), None).await.unwrap();
        let worktree = store
            .create_worktree(ws.id, "/tmp/ws".into(), "abc123".into(), None)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                ctx_core::models::ExecutionEnvironment::Host,
                "fake".into(),
                "fake".into(),
                "implementer".into(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .set_task_primary_session(task.id, session.id, worktree.id)
            .await
            .unwrap();

        let turn_id = TurnId::new();
        let now = Utc::now();
        let turn = SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: None,
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        };
        store.insert_session_turn(turn).await.unwrap();

        (session, turn_id)
    }

    #[tokio::test]
    async fn session_head_snapshot_excludes_assistant_partials() {
        let (_dir, store) = setup_store().await;
        let (session, turn_id) =
            create_session_with_turn(&store, Some("partial".to_string())).await;

        let _ = store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::AssistantChunk,
                json!({"content_fragment":"hi"}),
            )
            .await
            .unwrap();
        let _ = store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::Notice,
                json!({"msg":"done"}),
            )
            .await
            .unwrap();

        let events = store.list_session_events(session.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events
            .iter()
            .all(|event| !matches!(event.event_type, SessionEventType::AssistantChunk)));

        let head = store
            .get_session_head_snapshot(session.id, 10, true)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(head.turns.len(), 1);
        assert!(head.turns[0].assistant_partial.is_none());
        assert!(head
            .events
            .iter()
            .all(|event| !matches!(event.event_type, SessionEventType::AssistantChunk)));
        assert!(head
            .events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::Notice)));
    }

    #[tokio::test]
    async fn active_snapshot_head_strips_assistant_partials() {
        let (_dir, store) = setup_store().await;
        let (session, _turn_id) =
            create_session_with_turn(&store, Some("partial".to_string())).await;
        store
            .flush_active_snapshot_head_projection_queue()
            .await
            .unwrap();

        let row = sqlx::query(
            r#"SELECT head_rev, turns_json
               FROM session_active_snapshot_heads
               WHERE session_id = ?"#,
        )
        .bind(session.id.0.to_string())
        .fetch_optional(&store.pool)
        .await
        .unwrap();
        let row = row.expect("expected durable active snapshot head projection");
        let head_rev: i64 = row.try_get("head_rev").unwrap();
        assert_eq!(
            head_rev,
            store.get_session_projection_rev(session.id).await.unwrap()
        );
        let turns_json: String = row.try_get("turns_json").unwrap();
        let turns: Vec<SessionTurn> = serde_json::from_str(&turns_json).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0].assistant_partial.is_none());
    }

    #[tokio::test]
    async fn terminal_event_flush_updates_turn_and_summary_in_one_persist_path() {
        let (_dir, store) = setup_store().await;
        let (session, turn_id) = create_session_with_turn(&store, None).await;

        let _done = store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::Done,
                json!({"context_window": {"total_tokens": 42}}),
            )
            .await
            .unwrap();
        let finished = store
            .append_session_event(
                session.id,
                None,
                Some(turn_id),
                SessionEventType::TurnFinished,
                json!({"status": "completed"}),
            )
            .await
            .unwrap();

        let events = store
            .list_session_events_for_turn(session.id, turn_id, false)
            .await
            .unwrap();
        assert_eq!(events.len(), 2);

        let turn = store
            .get_session_turn(session.id, turn_id)
            .await
            .unwrap()
            .expect("turn exists");
        assert_eq!(turn.status, SessionTurnStatus::Completed);
        assert_eq!(turn.end_seq, Some(finished.seq));
        assert_eq!(turn.metrics_json, Some(json!({"total_tokens": 42})));

        let snapshot = store
            .get_session_snapshot(session.id, 10, false)
            .await
            .unwrap()
            .expect("snapshot");
        assert_eq!(snapshot.summary.last_event_seq, Some(finished.seq));
        assert_eq!(
            snapshot.summary.activity.last_turn_status,
            Some(SessionTurnStatus::Completed)
        );
        assert!(!snapshot.summary.activity.is_working);
    }

    #[tokio::test]
    async fn raw_provider_terminal_events_do_not_complete_turn_until_turn_finished_persists() {
        let (_dir, store) = setup_store().await;
        let cases = [
            (
                SessionEventType::Done,
                json!({"context_window": {"total_tokens": 7}}),
                json!({"status": "completed"}),
                SessionTurnStatus::Completed,
                Some(json!({"total_tokens": 7})),
            ),
            (
                SessionEventType::Error,
                json!({"message": "provider error"}),
                json!({"status": "failed", "message": "provider error"}),
                SessionTurnStatus::Failed,
                None,
            ),
            (
                SessionEventType::TurnInterrupted,
                json!({"reason": "cancelled", "provider_cancelled": true}),
                json!({"status": "interrupted", "reason": "cancelled", "provider_cancelled": true}),
                SessionTurnStatus::Interrupted,
                None,
            ),
        ];

        for (event_type, event_payload, finished_payload, expected_status, expected_metrics) in
            cases
        {
            let (session, turn_id) = create_session_with_turn(&store, None).await;

            let terminal = store
                .append_session_event(session.id, None, Some(turn_id), event_type, event_payload)
                .await
                .unwrap();
            store.flush_session_event_log().await.unwrap();

            let running_turn = store
                .get_session_turn(session.id, turn_id)
                .await
                .unwrap()
                .expect("turn exists");
            assert_eq!(running_turn.status, SessionTurnStatus::Running);
            assert_eq!(running_turn.end_seq, None);

            let persisted = store
                .persist_turn_terminal_events(
                    session.id,
                    None,
                    turn_id,
                    vec![(SessionEventType::TurnFinished, finished_payload)],
                )
                .await
                .unwrap();

            assert_eq!(persisted.len(), 1);
            assert!(persisted[0].seq > terminal.seq);

            let completed_turn = store
                .get_session_turn(session.id, turn_id)
                .await
                .unwrap()
                .expect("completed turn");
            assert_eq!(completed_turn.status, expected_status);
            assert_eq!(completed_turn.end_seq, Some(persisted[0].seq));
            assert_eq!(completed_turn.metrics_json, expected_metrics);
        }
    }

    #[tokio::test]
    async fn get_terminal_event_for_run_flushes_buffered_events_before_reading() {
        let (_dir, store) = setup_store().await;
        let (session, turn_id) = create_session_with_turn(&store, None).await;
        let run_id = RunId::new();
        let write_guard = store.write_gate.lock().await;
        let queued_event = SessionEvent {
            seq: store.event_log.next_seq(),
            id: SessionEventId::new(),
            session_id: session.id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            event_type: SessionEventType::TurnFinished,
            payload_json: json!({ "status": "completed" }),
            transient: false,
            created_at: Utc::now(),
        };
        store
            .event_log
            .enqueue(queued_event.clone())
            .await
            .expect("buffer terminal event");

        let store_for_read = store.clone();
        let read_task = tokio::spawn(async move {
            store_for_read
                .get_terminal_event_for_run(session.id, run_id)
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !read_task.is_finished(),
            "terminal-event read should wait for buffered event-log persistence"
        );

        drop(write_guard);

        let terminal = read_task
            .await
            .expect("join read task")
            .expect("read terminal event")
            .expect("terminal event exists");
        assert_eq!(terminal.id, queued_event.id);
    }

    #[tokio::test]
    async fn subagent_label_is_unique_per_task() {
        let (_dir, store) = setup_store().await;
        let ws = store
            .create_workspace("test".into(), "/tmp/test".into(), VcsKind::Git)
            .await
            .unwrap();
        let task = store.create_task(ws.id, "task".into(), None).await.unwrap();
        let worktree = store
            .create_worktree(ws.id, "/tmp/ws".into(), "abc123".into(), None)
            .await
            .unwrap();
        let parent = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                ctx_core::models::ExecutionEnvironment::Host,
                "fake".into(),
                "fake".into(),
                "assistant".into(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let child_one = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                ctx_core::models::ExecutionEnvironment::Host,
                "fake".into(),
                "fake".into(),
                "subagent".into(),
                Some(parent.id),
                Some("sub_agent".into()),
                None,
            )
            .await
            .unwrap();
        let child_two = store
            .create_session(
                task.id,
                ws.id,
                worktree.id,
                ctx_core::models::ExecutionEnvironment::Host,
                "fake".into(),
                "fake".into(),
                "subagent".into(),
                Some(parent.id),
                Some("sub_agent".into()),
                None,
            )
            .await
            .unwrap();

        store
            .update_session_title(child_one.id, "Dup".into())
            .await
            .unwrap();

        let err = store
            .update_session_title(child_two.id, "Dup".into())
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unique"));
    }
}
