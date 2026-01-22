use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ctx_core::ids::*;
use ctx_core::models::*;
use serde::Serialize;
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, Pool, QueryBuilder, Row, Sqlite};

#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
}

#[derive(Debug, Clone)]
pub struct EventLogStats {
    pub published_seq: i64,
    pub durable_seq: i64,
    pub queue_depth: u64,
}

pub struct SessionRetentionPruneStats {
    pub tool_summaries_deleted: u64,
    pub turn_thoughts_cleared: u64,
}

#[derive(Debug, Clone)]
pub struct SessionCompactionSeed {
    pub session_id: SessionId,
    pub seed_text: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

const SESSION_HEAD_MAX_TURNS: u32 = 200;
const SESSION_HEAD_MESSAGE_LIMIT: usize = 200;
const SESSION_HEAD_EVENT_LIMIT: usize = 200;
const SESSION_HEAD_BYTE_LIMIT: usize = 1_500_000;
const ACTIVE_SNAPSHOT_HEAD_LIMIT: u32 = 60;
const SESSION_HEAD_ARCHIVED_TURN_LIMIT: u32 = 50;

#[derive(Clone, Copy, Debug)]
enum SessionHeadKind {
    Active,
    Archived,
}

#[derive(Clone, Copy, Debug)]
struct SessionHeadLimits {
    turn_limit: usize,
    message_limit: usize,
    event_limit: usize,
    byte_limit: usize,
}

#[derive(Debug, Clone)]
struct SessionHeadMaterialization {
    last_event_seq: i64,
    turns: Vec<SessionTurn>,
    tool_summaries: Vec<SessionTurnToolSummary>,
    events: Vec<SessionEvent>,
    messages: Vec<Message>,
    has_more_turns: bool,
    head_window: SessionHeadWindow,
}

impl SessionHeadMaterialization {
    fn from_head(head: &SessionHead) -> Self {
        Self {
            last_event_seq: head.last_event_seq,
            turns: head.turns.clone(),
            tool_summaries: head.tool_summaries.clone(),
            events: head.events.clone(),
            messages: head.messages.clone(),
            has_more_turns: head.has_more_turns,
            head_window: head.head_window.clone(),
        }
    }

    fn into_session_head(
        self,
        session: Session,
        summary_checkpoint: Option<SessionSummaryCheckpoint>,
    ) -> SessionHead {
        let last_status = self.turns.last().map(|t| t.status.clone());
        let has_running_turn = self
            .turns
            .iter()
            .any(|turn| matches!(turn.status, SessionTurnStatus::Running));
        let activity = derive_activity_from_status(last_status, has_running_turn);
        SessionHead {
            session,
            turns: self.turns,
            tool_summaries: self.tool_summaries,
            events: self.events,
            messages: self.messages,
            last_event_seq: self.last_event_seq,
            activity,
            has_more_turns: self.has_more_turns,
            summary_checkpoint,
            head_window: self.head_window,
        }
    }
}

#[derive(Serialize)]
struct SessionHeadWindowPayload<'a> {
    turns: &'a [SessionTurn],
    tool_summaries: &'a [SessionTurnToolSummary],
    events: &'a [SessionEvent],
    messages: &'a [Message],
}

fn head_window_bytes(
    turns: &[SessionTurn],
    tool_summaries: &[SessionTurnToolSummary],
    events: &[SessionEvent],
    messages: &[Message],
) -> usize {
    let payload = SessionHeadWindowPayload {
        turns,
        tool_summaries,
        events,
        messages,
    };
    serde_json::to_vec(&payload)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
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

    while messages.len() > message_limit && !turns.is_empty() {
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
        if !turns.is_empty() {
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
    pub config_path: Option<String>,
    pub config_key: Option<String>,
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

impl Store {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    pub fn event_log_stats(&self) -> EventLogStats {
        EventLogStats {
            published_seq: 0,
            durable_seq: 0,
            queue_depth: 0,
        }
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn migrate_workspace_from_path(
        &self,
        legacy_path: &Path,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        let workspace_id = workspace_id.0.to_string();
        let legacy_path = legacy_path.to_string_lossy().to_string();
        let mut conn = self.pool.acquire().await?;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await?;
        sqlx::query("ATTACH DATABASE ? AS legacy")
            .bind(&legacy_path)
            .execute(&mut *conn)
            .await?;

        let migrate = async {
            sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

            sqlx::query(
                r#"INSERT OR REPLACE INTO workspaces
                   SELECT * FROM legacy.workspaces WHERE id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO tasks
                   SELECT * FROM legacy.tasks WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO worktrees
                   SELECT * FROM legacy.worktrees WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO sessions
                   SELECT * FROM legacy.sessions WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO messages
                   SELECT * FROM legacy.messages
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_events
                   SELECT * FROM legacy.session_events
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_turns
                   SELECT * FROM legacy.session_turns
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_turn_tools
                   SELECT * FROM legacy.session_turn_tools
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO artifacts
                   SELECT * FROM legacy.artifacts WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO workspace_attachments
                   SELECT * FROM legacy.workspace_attachments WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO worktree_attachment_mounts
                   SELECT * FROM legacy.worktree_attachment_mounts
                   WHERE worktree_id IN (
                     SELECT id FROM legacy.worktrees WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO subagent_invocations
                   SELECT * FROM legacy.subagent_invocations
                   WHERE parent_session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO subagent_invocation_children
                   SELECT * FROM legacy.subagent_invocation_children
                   WHERE child_session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO merge_queue_entries
                   SELECT * FROM legacy.merge_queue_entries WHERE workspace_id = ?"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO merge_queue_runs
                   SELECT * FROM legacy.merge_queue_runs
                   WHERE entry_id IN (
                     SELECT id FROM legacy.merge_queue_entries WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_summary_checkpoints
                   SELECT * FROM legacy.session_summary_checkpoints
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_head_materializations
                   SELECT * FROM legacy.session_head_materializations
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_snapshot_summaries
                   SELECT * FROM legacy.session_snapshot_summaries
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query(
                r#"INSERT INTO session_git_status_snapshots
                   SELECT * FROM legacy.session_git_status_snapshots
                   WHERE session_id IN (
                     SELECT id FROM legacy.sessions WHERE workspace_id = ?
                   )"#,
            )
            .bind(&workspace_id)
            .execute(&mut *conn)
            .await?;

            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = migrate {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            let _ = sqlx::query("DETACH DATABASE legacy")
                .execute(&mut *conn)
                .await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *conn)
                .await;
            return Err(err);
        }

        sqlx::query("DETACH DATABASE legacy")
            .execute(&mut *conn)
            .await?;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await?;
        Ok(())
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

        let tool_summaries_deleted = sqlx::query(
            r#"DELETE FROM session_turn_tools
               WHERE updated_at < ?"#,
        )
        .bind(&cutoff_str)
        .execute(&self.pool)
        .await?
        .rows_affected();

        // Keep the row (turn metadata is still useful), but remove old final thoughts.
        let turn_thoughts_cleared = sqlx::query(
            r#"UPDATE session_turns
               SET thought_partial = NULL
               WHERE updated_at < ?
                 AND thought_partial IS NOT NULL"#,
        )
        .bind(&cutoff_str)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(SessionRetentionPruneStats {
            tool_summaries_deleted,
            turn_thoughts_cleared,
        })
    }

    // Workspace APIs
    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let rows = sqlx::query(
            r#"SELECT id, name, root_path, created_at FROM workspaces ORDER BY created_at ASC"#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let name: String = r.try_get("name")?;
            let root_path: String = r.try_get("root_path")?;
            let created_at: String = r.try_get("created_at")?;
            out.push(Workspace {
                id: WorkspaceId(uuid::Uuid::parse_str(&id)?),
                name,
                root_path,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        let row =
            sqlx::query(r#"SELECT id, name, root_path, created_at FROM workspaces WHERE id = ?"#)
                .bind(id.0.to_string())
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            Some(Workspace {
                id: WorkspaceId(uuid::Uuid::parse_str(&id).ok()?),
                name: r.try_get("name").ok()?,
                root_path: r.try_get("root_path").ok()?,
                created_at: parse_dt(r.try_get::<String, _>("created_at").ok()?.as_str()).ok()?,
            })
        }))
    }

    pub async fn create_workspace(&self, name: String, root_path: String) -> Result<Workspace> {
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name,
            root_path,
            created_at: Utc::now(),
        };
        sqlx::query(
            r#"INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)"#,
        )
        .bind(workspace.id.0.to_string())
        .bind(&workspace.name)
        .bind(&workspace.root_path)
        .bind(workspace.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(workspace)
    }

    pub async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        sqlx::query(r#"DELETE FROM workspaces WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_workspace(&self, workspace: &Workspace) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workspaces (id, name, root_path, created_at)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 root_path = excluded.root_path"#,
        )
        .bind(workspace.id.0.to_string())
        .bind(&workspace.name)
        .bind(&workspace.root_path)
        .bind(workspace.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_task_index(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workspace_task_index (task_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(task_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(task_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_session_index(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workspace_session_index (session_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(session_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(session_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_worktree_index(
        &self,
        worktree_id: WorktreeId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workspace_worktree_index (worktree_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(worktree_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(worktree_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_workspace_task_index(&self, task_id: TaskId) -> Result<()> {
        sqlx::query(r#"DELETE FROM workspace_task_index WHERE task_id = ?"#)
            .bind(task_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_session_index(&self, session_id: SessionId) -> Result<()> {
        sqlx::query(r#"DELETE FROM workspace_session_index WHERE session_id = ?"#)
            .bind(session_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_worktree_index(&self, worktree_id: WorktreeId) -> Result<()> {
        sqlx::query(r#"DELETE FROM workspace_worktree_index WHERE worktree_id = ?"#)
            .bind(worktree_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_workspace_id_for_task(&self, task_id: TaskId) -> Result<Option<WorkspaceId>> {
        let row = sqlx::query(r#"SELECT workspace_id FROM workspace_task_index WHERE task_id = ?"#)
            .bind(task_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn get_workspace_id_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceId>> {
        let row =
            sqlx::query(r#"SELECT workspace_id FROM workspace_session_index WHERE session_id = ?"#)
                .bind(session_id.0.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn get_workspace_id_for_worktree(
        &self,
        worktree_id: WorktreeId,
    ) -> Result<Option<WorkspaceId>> {
        let row = sqlx::query(
            r#"SELECT workspace_id FROM workspace_worktree_index WHERE worktree_id = ?"#,
        )
        .bind(worktree_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn refresh_workspace_indexes(&self, workspace_id: WorkspaceId) -> Result<()> {
        let workspace_id = workspace_id.0.to_string();
        sqlx::query(
            r#"INSERT OR REPLACE INTO workspace_task_index (task_id, workspace_id)
               SELECT id, workspace_id FROM tasks WHERE workspace_id = ?"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"INSERT OR REPLACE INTO workspace_session_index (session_id, workspace_id)
               SELECT id, workspace_id FROM sessions WHERE workspace_id = ?"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"INSERT OR REPLACE INTO workspace_worktree_index (worktree_id, workspace_id)
               SELECT id, workspace_id FROM worktrees WHERE workspace_id = ?"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_workspace_indexes(&self, workspace_id: WorkspaceId) -> Result<()> {
        let workspace_id = workspace_id.0.to_string();
        sqlx::query(r#"DELETE FROM workspace_task_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(r#"DELETE FROM workspace_session_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(r#"DELETE FROM workspace_worktree_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Task APIs
    pub async fn list_tasks(&self, workspace_id: WorkspaceId) -> Result<Vec<Task>> {
        let rows = sqlx::query(
            r#"
            SELECT
              t.id, t.workspace_id, t.title, t.description, t.status, t.exec_plan_id,
              t.primary_session_id, t.primary_worktree_id,
              t.created_at, t.updated_at, t.archived_at, t.assistant_seen_at,
              t.last_activity_at,
              t.last_assistant_message_at,
              EXISTS(
                SELECT 1
                FROM sessions s
                WHERE s.task_id = t.id AND s.status = 'active'
              ) AS has_active_session
            FROM tasks t
            WHERE t.workspace_id = ?
            ORDER BY COALESCE(last_activity_at, t.updated_at, t.created_at) DESC
            "#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let archived_at: Option<String> = r.try_get("archived_at")?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at")?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id")?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id")?;
            let last_activity_at: Option<String> = r.try_get("last_activity_at")?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at")?;
            let has_active_session: i64 = r.try_get("has_active_session")?;
            out.push(Task {
                id: TaskId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                title: r.try_get("title")?,
                description: r.try_get("description")?,
                status: parse_task_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
                exec_plan_id: r.try_get("exec_plan_id")?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose()?,
                last_activity_at: last_activity_at.as_deref().map(parse_dt).transpose()?,
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                has_active_session: has_active_session != 0,
            });
        }
        Ok(out)
    }

    pub async fn create_task(
        &self,
        workspace_id: WorkspaceId,
        title: String,
        description: Option<String>,
    ) -> Result<Task> {
        let now = Utc::now();
        let task = Task {
            id: TaskId::new(),
            workspace_id,
            title,
            description,
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            exec_plan_id: None,
            primary_session_id: None,
            primary_worktree_id: None,
            archived_at: None,
            assistant_seen_at: None,
            last_activity_at: None,
            last_assistant_message_at: None,
            has_active_session: false,
        };
        sqlx::query(
            r#"INSERT INTO tasks (id, workspace_id, title, description, status, exec_plan_id, primary_session_id, primary_worktree_id, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(task.id.0.to_string())
        .bind(task.workspace_id.0.to_string())
        .bind(&task.title)
        .bind(&task.description)
        .bind(task_status_to_str(&task.status))
        .bind(&task.exec_plan_id)
        .bind(task.primary_session_id.map(|id| id.0.to_string()))
        .bind(task.primary_worktree_id.map(|id| id.0.to_string()))
        .bind(task.created_at.to_rfc3339())
        .bind(task.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(task)
    }

    pub async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, title, description, status, exec_plan_id,
                      primary_session_id, primary_worktree_id,
                      created_at, updated_at, archived_at, assistant_seen_at,
                      last_activity_at, last_assistant_message_at
               FROM tasks WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let updated_at: String = r.try_get("updated_at").ok()?;
            let archived_at: Option<String> = r.try_get("archived_at").ok()?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at").ok()?;
            let last_activity_at: Option<String> = r.try_get("last_activity_at").ok()?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at").ok()?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id").ok()?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id").ok()?;
            Some(Task {
                id: TaskId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                title: r.try_get("title").ok()?,
                description: r.try_get("description").ok()?,
                status: parse_task_status(r.try_get::<String, _>("status").ok()?.as_str()),
                created_at: parse_dt(&created_at).ok()?,
                updated_at: parse_dt(&updated_at).ok()?,
                exec_plan_id: r.try_get("exec_plan_id").ok()?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose().ok()?,
                assistant_seen_at: assistant_seen_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                last_activity_at: last_activity_at.as_deref().map(parse_dt).transpose().ok()?,
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                has_active_session: false,
            })
        }))
    }

    pub async fn archive_task(&self, id: TaskId) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET archived_at = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(&now)
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        if res.rows_affected() > 0 {
            self.materialize_archived_heads_for_task(id).await?;
            self.delete_session_head_materializations_for_task(id, SessionHeadKind::Active)
                .await?;
        }
        Ok(res.rows_affected() > 0)
    }

    pub async fn unarchive_task(&self, id: TaskId) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET archived_at = NULL, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        if res.rows_affected() > 0 {
            self.delete_session_head_materializations_for_task(id, SessionHeadKind::Archived)
                .await?;
        }
        Ok(res.rows_affected() > 0)
    }

    pub async fn update_task_title(&self, id: TaskId, title: String) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET title = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(title)
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn set_task_primary_session(
        &self,
        id: TaskId,
        session_id: SessionId,
        worktree_id: WorktreeId,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET primary_session_id = ?, primary_worktree_id = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(session_id.0.to_string())
        .bind(worktree_id.0.to_string())
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn set_task_primary_worktree(
        &self,
        id: TaskId,
        worktree_id: WorktreeId,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET primary_worktree_id = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(worktree_id.0.to_string())
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn mark_task_read(&self, id: TaskId) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE tasks
               SET assistant_seen_at = ?
               WHERE id = ?"#,
        )
        .bind(&now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn mark_task_unread(&self, id: TaskId) -> Result<bool> {
        let res = sqlx::query(
            r#"UPDATE tasks
               SET assistant_seen_at = NULL
               WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn delete_task(&self, id: TaskId) -> Result<bool> {
        let res = sqlx::query(r#"DELETE FROM tasks WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_task_with_activity(&self, id: TaskId) -> Result<Option<Task>> {
        let row = sqlx::query(
            r#"
            SELECT
              t.id, t.workspace_id, t.title, t.description, t.status, t.exec_plan_id,
              t.primary_session_id, t.primary_worktree_id,
              t.created_at, t.updated_at, t.archived_at, t.assistant_seen_at,
              t.last_activity_at,
              t.last_assistant_message_at,
              EXISTS(
                SELECT 1
                FROM sessions s
                WHERE s.task_id = t.id AND s.status = 'active'
              ) AS has_active_session
            FROM tasks t
            WHERE t.id = ?
            "#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let updated_at: String = r.try_get("updated_at").ok()?;
            let archived_at: Option<String> = r.try_get("archived_at").ok()?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at").ok()?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id").ok()?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id").ok()?;
            let last_activity_at: Option<String> = r.try_get("last_activity_at").ok()?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at").ok()?;
            let has_active_session: i64 = r.try_get("has_active_session").ok()?;
            Some(Task {
                id: TaskId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                title: r.try_get("title").ok()?,
                description: r.try_get("description").ok()?,
                status: parse_task_status(r.try_get::<String, _>("status").ok()?.as_str()),
                created_at: parse_dt(&created_at).ok()?,
                updated_at: parse_dt(&updated_at).ok()?,
                exec_plan_id: r.try_get("exec_plan_id").ok()?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose().ok()?,
                assistant_seen_at: assistant_seen_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                last_activity_at: last_activity_at.as_deref().map(parse_dt).transpose().ok()?,
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                has_active_session: has_active_session != 0,
            })
        }))
    }

    // Worktree APIs
    pub async fn insert_worktree(&self, worktree: Worktree) -> Result<Worktree> {
        sqlx::query(
            r#"INSERT INTO worktrees (id, workspace_id, root_path, base_commit_sha, git_branch, created_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(worktree.id.0.to_string())
        .bind(worktree.workspace_id.0.to_string())
        .bind(&worktree.root_path)
        .bind(&worktree.base_commit_sha)
        .bind(&worktree.git_branch)
        .bind(worktree.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(worktree)
    }

    pub async fn create_worktree(
        &self,
        workspace_id: WorkspaceId,
        root_path: String,
        base_commit_sha: String,
        git_branch: Option<String>,
    ) -> Result<Worktree> {
        let worktree = Worktree {
            id: WorktreeId::new(),
            workspace_id,
            root_path,
            base_commit_sha,
            git_branch,
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_config_path: None,
            bootstrap_config_key: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        };
        sqlx::query(
            r#"INSERT INTO worktrees (id, workspace_id, root_path, base_commit_sha, git_branch, created_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(worktree.id.0.to_string())
        .bind(worktree.workspace_id.0.to_string())
        .bind(&worktree.root_path)
        .bind(&worktree.base_commit_sha)
        .bind(&worktree.git_branch)
        .bind(worktree.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(worktree)
    }

    pub async fn get_worktree(&self, id: WorktreeId) -> Result<Option<Worktree>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at,
                      bootstrap_status, bootstrap_started_at, bootstrap_finished_at, bootstrap_exit_code,
                      bootstrap_timeout_sec, bootstrap_error, bootstrap_log_path, bootstrap_log_truncated,
                      bootstrap_config_path, bootstrap_config_key, bootstrap_command, bootstrap_script_path
               FROM worktrees WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let bootstrap_status: Option<String> = r.try_get("bootstrap_status").ok()?;
            let bootstrap_started_at: Option<String> = r.try_get("bootstrap_started_at").ok()?;
            let bootstrap_finished_at: Option<String> = r.try_get("bootstrap_finished_at").ok()?;
            let bootstrap_exit_code: Option<i64> = r.try_get("bootstrap_exit_code").ok()?;
            let bootstrap_timeout_sec: Option<i64> = r.try_get("bootstrap_timeout_sec").ok()?;
            let bootstrap_error: Option<String> = r.try_get("bootstrap_error").ok()?;
            let bootstrap_log_path: Option<String> = r.try_get("bootstrap_log_path").ok()?;
            let bootstrap_log_truncated: Option<i64> = r.try_get("bootstrap_log_truncated").ok()?;
            let bootstrap_config_path: Option<String> = r.try_get("bootstrap_config_path").ok()?;
            let bootstrap_config_key: Option<String> = r.try_get("bootstrap_config_key").ok()?;
            let bootstrap_command: Option<String> = r.try_get("bootstrap_command").ok()?;
            let bootstrap_script_path: Option<String> = r.try_get("bootstrap_script_path").ok()?;
            Some(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                root_path: r.try_get("root_path").ok()?,
                base_commit_sha: r.try_get("base_commit_sha").ok()?,
                git_branch: r.try_get("git_branch").ok()?,
                created_at: parse_dt(&created_at).ok()?,
                bootstrap_status: parse_bootstrap_status(bootstrap_status),
                bootstrap_started_at: bootstrap_started_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                bootstrap_finished_at: bootstrap_finished_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                bootstrap_exit_code,
                bootstrap_timeout_sec,
                bootstrap_error,
                bootstrap_log_path,
                bootstrap_log_truncated: bootstrap_log_truncated.map(|v| v != 0),
                bootstrap_config_path,
                bootstrap_config_key,
                bootstrap_command,
                bootstrap_script_path,
            })
        }))
    }

    pub async fn get_local_worktree_for_root(
        &self,
        workspace_id: WorkspaceId,
        root_path: &str,
    ) -> Result<Option<Worktree>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at,
                      bootstrap_status, bootstrap_started_at, bootstrap_finished_at, bootstrap_exit_code,
                      bootstrap_timeout_sec, bootstrap_error, bootstrap_log_path, bootstrap_log_truncated,
                      bootstrap_config_path, bootstrap_config_key, bootstrap_command, bootstrap_script_path
               FROM worktrees
               WHERE workspace_id = ? AND root_path = ? AND git_branch IS NULL
               ORDER BY created_at DESC
               LIMIT 1"#,
        )
        .bind(workspace_id.0.to_string())
        .bind(root_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let bootstrap_status: Option<String> = r.try_get("bootstrap_status").ok()?;
            let bootstrap_started_at: Option<String> = r.try_get("bootstrap_started_at").ok()?;
            let bootstrap_finished_at: Option<String> = r.try_get("bootstrap_finished_at").ok()?;
            let bootstrap_exit_code: Option<i64> = r.try_get("bootstrap_exit_code").ok()?;
            let bootstrap_timeout_sec: Option<i64> = r.try_get("bootstrap_timeout_sec").ok()?;
            let bootstrap_error: Option<String> = r.try_get("bootstrap_error").ok()?;
            let bootstrap_log_path: Option<String> = r.try_get("bootstrap_log_path").ok()?;
            let bootstrap_log_truncated: Option<i64> = r.try_get("bootstrap_log_truncated").ok()?;
            let bootstrap_config_path: Option<String> = r.try_get("bootstrap_config_path").ok()?;
            let bootstrap_config_key: Option<String> = r.try_get("bootstrap_config_key").ok()?;
            let bootstrap_command: Option<String> = r.try_get("bootstrap_command").ok()?;
            let bootstrap_script_path: Option<String> = r.try_get("bootstrap_script_path").ok()?;
            Some(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                root_path: r.try_get("root_path").ok()?,
                base_commit_sha: r.try_get("base_commit_sha").ok()?,
                git_branch: r.try_get("git_branch").ok()?,
                created_at: parse_dt(&created_at).ok()?,
                bootstrap_status: parse_bootstrap_status(bootstrap_status),
                bootstrap_started_at: bootstrap_started_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                bootstrap_finished_at: bootstrap_finished_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                bootstrap_exit_code,
                bootstrap_timeout_sec,
                bootstrap_error,
                bootstrap_log_path,
                bootstrap_log_truncated: bootstrap_log_truncated.map(|v| v != 0),
                bootstrap_config_path,
                bootstrap_config_key,
                bootstrap_command,
                bootstrap_script_path,
            })
        }))
    }

    pub async fn list_worktrees(&self, workspace_id: WorkspaceId) -> Result<Vec<Worktree>> {
        let rows = sqlx::query(
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at,
                      bootstrap_status, bootstrap_started_at, bootstrap_finished_at, bootstrap_exit_code,
                      bootstrap_timeout_sec, bootstrap_error, bootstrap_log_path, bootstrap_log_truncated,
                      bootstrap_config_path, bootstrap_config_key, bootstrap_command, bootstrap_script_path
               FROM worktrees WHERE workspace_id = ? ORDER BY created_at ASC"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let bootstrap_status: Option<String> = r.try_get("bootstrap_status")?;
            let bootstrap_started_at: Option<String> = r.try_get("bootstrap_started_at")?;
            let bootstrap_finished_at: Option<String> = r.try_get("bootstrap_finished_at")?;
            let bootstrap_exit_code: Option<i64> = r.try_get("bootstrap_exit_code")?;
            let bootstrap_timeout_sec: Option<i64> = r.try_get("bootstrap_timeout_sec")?;
            let bootstrap_error: Option<String> = r.try_get("bootstrap_error")?;
            let bootstrap_log_path: Option<String> = r.try_get("bootstrap_log_path")?;
            let bootstrap_log_truncated: Option<i64> = r.try_get("bootstrap_log_truncated")?;
            let bootstrap_config_path: Option<String> = r.try_get("bootstrap_config_path")?;
            let bootstrap_config_key: Option<String> = r.try_get("bootstrap_config_key")?;
            let bootstrap_command: Option<String> = r.try_get("bootstrap_command")?;
            let bootstrap_script_path: Option<String> = r.try_get("bootstrap_script_path")?;
            out.push(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                root_path: r.try_get("root_path")?,
                base_commit_sha: r.try_get("base_commit_sha")?,
                git_branch: r.try_get("git_branch")?,
                created_at: parse_dt(&created_at)?,
                bootstrap_status: parse_bootstrap_status(bootstrap_status),
                bootstrap_started_at: bootstrap_started_at.as_deref().map(parse_dt).transpose()?,
                bootstrap_finished_at: bootstrap_finished_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                bootstrap_exit_code,
                bootstrap_timeout_sec,
                bootstrap_error,
                bootstrap_log_path,
                bootstrap_log_truncated: bootstrap_log_truncated.map(|v| v != 0),
                bootstrap_config_path,
                bootstrap_config_key,
                bootstrap_command,
                bootstrap_script_path,
            });
        }
        Ok(out)
    }

    pub async fn update_worktree_base_commit(
        &self,
        worktree_id: WorktreeId,
        base_commit_sha: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE worktrees
               SET base_commit_sha = ?
               WHERE id = ?"#,
        )
        .bind(base_commit_sha)
        .bind(worktree_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_worktree_bootstrap_result(
        &self,
        update: WorktreeBootstrapResultUpdate,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE worktrees
               SET bootstrap_status = ?,
                   bootstrap_started_at = ?,
                   bootstrap_finished_at = ?,
                   bootstrap_exit_code = ?,
                   bootstrap_timeout_sec = ?,
                   bootstrap_error = ?,
                   bootstrap_log_path = ?,
                   bootstrap_log_truncated = ?,
                   bootstrap_config_path = ?,
                   bootstrap_config_key = ?,
                   bootstrap_command = ?,
                   bootstrap_script_path = ?
               WHERE id = ?"#,
        )
        .bind(serialize_bootstrap_status(&update.status))
        .bind(update.started_at.to_rfc3339())
        .bind(update.finished_at.to_rfc3339())
        .bind(update.exit_code)
        .bind(update.timeout_sec)
        .bind(update.error)
        .bind(update.log_path)
        .bind(update.log_truncated.map(|v| if v { 1 } else { 0 }))
        .bind(update.config_path)
        .bind(update.config_key)
        .bind(update.command)
        .bind(update.script_path)
        .bind(update.worktree_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // Attachment APIs
    pub async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let rows = sqlx::query(
            r#"SELECT id, workspace_id, kind, name, source, revision, subpath, mount_relpath, mode,
                      update_policy, created_at, updated_at
               FROM workspace_attachments
               WHERE workspace_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let kind: String = r.try_get("kind")?;
            let mode: String = r.try_get("mode")?;
            let update_policy: String = r.try_get("update_policy")?;
            out.push(WorkspaceAttachment {
                id: WorkspaceAttachmentId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                kind: parse_attachment_kind(&kind),
                name: r.try_get("name")?,
                source: r.try_get("source")?,
                revision: r.try_get("revision")?,
                subpath: r.try_get("subpath")?,
                mount_relpath: r.try_get("mount_relpath")?,
                mode: parse_attachment_mode(&mode),
                update_policy: parse_attachment_update_policy(&update_policy),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn upsert_workspace_attachment(
        &self,
        attachment: &WorkspaceAttachment,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workspace_attachments
               (id, workspace_id, kind, name, source, revision, subpath, mount_relpath, mode, update_policy, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 name = excluded.name,
                 source = excluded.source,
                 revision = excluded.revision,
                 subpath = excluded.subpath,
                 mount_relpath = excluded.mount_relpath,
                 mode = excluded.mode,
                 update_policy = excluded.update_policy,
                 updated_at = excluded.updated_at"#,
        )
        .bind(attachment.id.0.to_string())
        .bind(attachment.workspace_id.0.to_string())
        .bind(attachment_kind_to_str(&attachment.kind))
        .bind(&attachment.name)
        .bind(&attachment.source)
        .bind(&attachment.revision)
        .bind(&attachment.subpath)
        .bind(&attachment.mount_relpath)
        .bind(attachment_mode_to_str(&attachment.mode))
        .bind(attachment_update_policy_to_str(&attachment.update_policy))
        .bind(attachment.created_at.to_rfc3339())
        .bind(attachment.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_workspace_attachment(&self, id: WorkspaceAttachmentId) -> Result<()> {
        sqlx::query(r#"DELETE FROM workspace_attachments WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_worktree_attachment_mounts(
        &self,
        worktree_id: WorktreeId,
    ) -> Result<Vec<WorktreeAttachmentMount>> {
        let rows = sqlx::query(
            r#"SELECT worktree_id, attachment_id, mount_abs_path, materialized_id, status,
                      last_sync_at, error_message, created_at, updated_at
               FROM worktree_attachment_mounts
               WHERE worktree_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(worktree_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let worktree_id: String = r.try_get("worktree_id")?;
            let attachment_id: String = r.try_get("attachment_id")?;
            let status: String = r.try_get("status")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(WorktreeAttachmentMount {
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
                attachment_id: WorkspaceAttachmentId(uuid::Uuid::parse_str(&attachment_id)?),
                mount_abs_path: r.try_get("mount_abs_path")?,
                materialized_id: r.try_get("materialized_id")?,
                status: parse_worktree_attachment_status(&status),
                last_sync_at: r
                    .try_get::<Option<String>, _>("last_sync_at")?
                    .and_then(|v| parse_dt(&v).ok()),
                error_message: r.try_get("error_message")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_worktree_attachment_mounts_for_attachment(
        &self,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<Vec<WorktreeAttachmentMount>> {
        let rows = sqlx::query(
            r#"SELECT worktree_id, attachment_id, mount_abs_path, materialized_id, status,
                      last_sync_at, error_message, created_at, updated_at
               FROM worktree_attachment_mounts
               WHERE attachment_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(attachment_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let worktree_id: String = r.try_get("worktree_id")?;
            let attachment_id: String = r.try_get("attachment_id")?;
            let status: String = r.try_get("status")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(WorktreeAttachmentMount {
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
                attachment_id: WorkspaceAttachmentId(uuid::Uuid::parse_str(&attachment_id)?),
                mount_abs_path: r.try_get("mount_abs_path")?,
                materialized_id: r.try_get("materialized_id")?,
                status: parse_worktree_attachment_status(&status),
                last_sync_at: r
                    .try_get::<Option<String>, _>("last_sync_at")?
                    .and_then(|v| parse_dt(&v).ok()),
                error_message: r.try_get("error_message")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn upsert_worktree_attachment_mount(
        &self,
        mount: &WorktreeAttachmentMount,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO worktree_attachment_mounts
               (worktree_id, attachment_id, mount_abs_path, materialized_id, status, last_sync_at, error_message, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(worktree_id, attachment_id) DO UPDATE SET
                 mount_abs_path = excluded.mount_abs_path,
                 materialized_id = excluded.materialized_id,
                 status = excluded.status,
                 last_sync_at = excluded.last_sync_at,
                 error_message = excluded.error_message,
                 updated_at = excluded.updated_at"#,
        )
        .bind(mount.worktree_id.0.to_string())
        .bind(mount.attachment_id.0.to_string())
        .bind(&mount.mount_abs_path)
        .bind(&mount.materialized_id)
        .bind(worktree_attachment_status_to_str(&mount.status))
        .bind(mount.last_sync_at.map(|v| v.to_rfc3339()))
        .bind(&mount.error_message)
        .bind(mount.created_at.to_rfc3339())
        .bind(mount.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_worktree_attachment_mounts_for_attachment(
        &self,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<()> {
        sqlx::query(r#"DELETE FROM worktree_attachment_mounts WHERE attachment_id = ?"#)
            .bind(attachment_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_merge_queue_entry(&self, entry: &MergeQueueEntry) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO merge_queue_entries (
                   id, workspace_id, worktree_id, session_id, target_branch, message, patch_source,
                   base_commit_sha, head_commit_sha, patch_path, patch_size, status,
                   result_commit_sha, error_message, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(entry.id.0.to_string())
        .bind(entry.workspace_id.0.to_string())
        .bind(entry.worktree_id.map(|id| id.0.to_string()))
        .bind(entry.session_id.map(|id| id.0.to_string()))
        .bind(&entry.target_branch)
        .bind(entry.message.as_deref())
        .bind(merge_queue_patch_source_to_str(&entry.patch_source))
        .bind(entry.base_commit_sha.as_deref())
        .bind(entry.head_commit_sha.as_deref())
        .bind(&entry.patch_path)
        .bind(entry.patch_size)
        .bind(merge_queue_entry_status_to_str(&entry.status))
        .bind(entry.result_commit_sha.as_deref())
        .bind(entry.error_message.as_deref())
        .bind(entry.created_at.to_rfc3339())
        .bind(entry.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_merge_queue_entry(&self, entry: &MergeQueueEntry) -> Result<()> {
        sqlx::query(
            r#"UPDATE merge_queue_entries
               SET worktree_id = ?,
                   session_id = ?,
                   target_branch = ?,
                   message = ?,
                   patch_source = ?,
                   base_commit_sha = ?,
                   head_commit_sha = ?,
                   patch_path = ?,
                   patch_size = ?,
                   status = ?,
                   result_commit_sha = ?,
                   error_message = ?,
                   updated_at = ?
               WHERE id = ?"#,
        )
        .bind(entry.worktree_id.map(|id| id.0.to_string()))
        .bind(entry.session_id.map(|id| id.0.to_string()))
        .bind(&entry.target_branch)
        .bind(entry.message.as_deref())
        .bind(merge_queue_patch_source_to_str(&entry.patch_source))
        .bind(entry.base_commit_sha.as_deref())
        .bind(entry.head_commit_sha.as_deref())
        .bind(&entry.patch_path)
        .bind(entry.patch_size)
        .bind(merge_queue_entry_status_to_str(&entry.status))
        .bind(entry.result_commit_sha.as_deref())
        .bind(entry.error_message.as_deref())
        .bind(entry.updated_at.to_rfc3339())
        .bind(entry.id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_merge_queue_entry(
        &self,
        id: MergeQueueEntryId,
    ) -> Result<Option<MergeQueueEntry>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, worktree_id, session_id, target_branch, message,
                      patch_source, base_commit_sha, head_commit_sha, patch_path, patch_size,
                      status, result_commit_sha, error_message, created_at, updated_at
               FROM merge_queue_entries WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(map_merge_queue_entry))
    }

    pub async fn list_merge_queue_entries(
        &self,
        workspace_id: WorkspaceId,
        limit: Option<i64>,
    ) -> Result<Vec<MergeQueueEntry>> {
        let mut builder = QueryBuilder::new(
            r#"SELECT id, workspace_id, worktree_id, session_id, target_branch, message,
                      patch_source, base_commit_sha, head_commit_sha, patch_path, patch_size,
                      status, result_commit_sha, error_message, created_at, updated_at
               FROM merge_queue_entries WHERE workspace_id = "#,
        );
        builder.push_bind(workspace_id.0.to_string());
        builder.push(" ORDER BY created_at DESC");
        if let Some(limit) = limit {
            builder.push(" LIMIT ");
            builder.push_bind(limit);
        }
        let rows = builder.build().fetch_all(&self.pool).await?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(entry) = map_merge_queue_entry(row) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    pub async fn list_queued_merge_queue_entries(&self) -> Result<Vec<MergeQueueEntry>> {
        let rows = sqlx::query(
            r#"SELECT id, workspace_id, worktree_id, session_id, target_branch, message,
                      patch_source, base_commit_sha, head_commit_sha, patch_path, patch_size,
                      status, result_commit_sha, error_message, created_at, updated_at
               FROM merge_queue_entries
               WHERE status = ?
               ORDER BY created_at ASC"#,
        )
        .bind("queued")
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(entry) = map_merge_queue_entry(row) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    pub async fn claim_merge_queue_entry(
        &self,
        entry_id: MergeQueueEntryId,
        updated_at: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE merge_queue_entries
               SET status = ?, updated_at = ?
               WHERE id = ? AND status = ?"#,
        )
        .bind("running")
        .bind(updated_at.to_rfc3339())
        .bind(entry_id.0.to_string())
        .bind("queued")
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn has_merge_queue_blocking_failure(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let row = sqlx::query(
            r#"SELECT 1 FROM merge_queue_entries
               WHERE workspace_id = ? AND status IN ("failed", "conflict")
               LIMIT 1"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn create_merge_queue_run(&self, run: &MergeQueueRun) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO merge_queue_runs (
                   id, entry_id, status, started_at, finished_at, exit_code,
                   log_path, error_message, result_commit_sha
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(run.id.0.to_string())
        .bind(run.entry_id.0.to_string())
        .bind(merge_queue_run_status_to_str(&run.status))
        .bind(run.started_at.to_rfc3339())
        .bind(run.finished_at.map(|dt| dt.to_rfc3339()))
        .bind(run.exit_code)
        .bind(run.log_path.as_deref())
        .bind(run.error_message.as_deref())
        .bind(run.result_commit_sha.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_merge_queue_run(&self, run: &MergeQueueRun) -> Result<()> {
        sqlx::query(
            r#"UPDATE merge_queue_runs
               SET status = ?,
                   finished_at = ?,
                   exit_code = ?,
                   log_path = ?,
                   error_message = ?,
                   result_commit_sha = ?
               WHERE id = ?"#,
        )
        .bind(merge_queue_run_status_to_str(&run.status))
        .bind(run.finished_at.map(|dt| dt.to_rfc3339()))
        .bind(run.exit_code)
        .bind(run.log_path.as_deref())
        .bind(run.error_message.as_deref())
        .bind(run.result_commit_sha.as_deref())
        .bind(run.id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_merge_queue_runs(
        &self,
        entry_id: MergeQueueEntryId,
    ) -> Result<Vec<MergeQueueRun>> {
        let rows = sqlx::query(
            r#"SELECT id, entry_id, status, started_at, finished_at, exit_code,
                      log_path, error_message, result_commit_sha
               FROM merge_queue_runs WHERE entry_id = ?
               ORDER BY started_at DESC"#,
        )
        .bind(entry_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(run) = map_merge_queue_run(row) {
                out.push(run);
            }
        }
        Ok(out)
    }

    pub async fn get_latest_merge_queue_run(
        &self,
        entry_id: MergeQueueEntryId,
    ) -> Result<Option<MergeQueueRun>> {
        let row = sqlx::query(
            r#"SELECT id, entry_id, status, started_at, finished_at, exit_code,
                      log_path, error_message, result_commit_sha
               FROM merge_queue_runs WHERE entry_id = ?
               ORDER BY started_at DESC
               LIMIT 1"#,
        )
        .bind(entry_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(map_merge_queue_run))
    }

    // Session APIs
    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        provider_id: String,
        model_id: String,
        agent_role: String,
        parent_session_id: Option<SessionId>,
        relationship: Option<String>,
        provider_session_ref: Option<String>,
    ) -> Result<Session> {
        let relationship = relationship.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        if parent_session_id.is_some() && relationship.is_none() {
            anyhow::bail!("parent_session_id requires relationship");
        }
        if parent_session_id.is_none() && relationship.is_some() {
            anyhow::bail!("relationship requires parent_session_id");
        }
        let now = Utc::now();
        let session = Session {
            id: SessionId::new(),
            task_id,
            workspace_id,
            worktree_id,
            parent_session_id,
            relationship,
            provider_id,
            model_id,
            title: "New Task".to_string(),
            agent_role,
            status: SessionStatus::Active,
            provider_session_ref,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"INSERT INTO sessions (id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, title, agent_role, status, provider_session_ref, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(session.id.0.to_string())
        .bind(session.task_id.0.to_string())
        .bind(session.workspace_id.0.to_string())
        .bind(session.worktree_id.0.to_string())
        .bind(session.parent_session_id.map(|id| id.0.to_string()))
        .bind(&session.relationship)
        .bind(&session.provider_id)
        .bind(&session.model_id)
        .bind(&session.title)
        .bind(&session.agent_role)
        .bind(session_status_to_str(&session.status))
        .bind(&session.provider_session_ref)
        .bind(session.created_at.to_rfc3339())
        .bind(session.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.ensure_session_snapshot_summary(session.id).await?;
        Ok(session)
    }

    pub async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        let row = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let task_id: String = r.try_get("task_id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let wt_id: String = r.try_get("worktree_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let updated_at: String = r.try_get("updated_at").ok()?;
            Some(Session {
                id: SessionId(uuid::Uuid::parse_str(&id).ok()?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id).ok()?),
                parent_session_id: parse_optional_session_id(r.try_get("parent_session_id").ok()?),
                relationship: r.try_get("relationship").ok()?,
                provider_id: r.try_get("provider_id").ok()?,
                model_id: r.try_get("model_id").ok()?,
                title: r.try_get("title").ok()?,
                agent_role: r.try_get("agent_role").ok()?,
                status: parse_session_status(r.try_get::<String, _>("status").ok()?.as_str()),
                provider_session_ref: r.try_get("provider_session_ref").ok()?,
                created_at: parse_dt(&created_at).ok()?,
                updated_at: parse_dt(&updated_at).ok()?,
            })
        }))
    }

    pub async fn update_session_model(&self, id: SessionId, model_id: String) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE sessions
               SET model_id = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(model_id)
        .bind(now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_session_title(&self, id: SessionId, title: String) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            r#"UPDATE sessions
               SET title = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(title)
        .bind(now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn update_session_provider_session_ref(
        &self,
        id: SessionId,
        provider_session_ref: Option<String>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE sessions
               SET provider_session_ref = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(provider_session_ref)
        .bind(now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sessions_for_task(&self, task_id: TaskId) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions WHERE task_id = ? ORDER BY created_at ASC"#,
        )
        .bind(task_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(Session {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                parent_session_id: parse_optional_session_id(r.try_get("parent_session_id")?),
                relationship: r.try_get("relationship")?,
                provider_id: r.try_get("provider_id")?,
                model_id: r.try_get("model_id")?,
                title: r.try_get("title")?,
                agent_role: r.try_get("agent_role")?,
                status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                provider_session_ref: r.try_get("provider_session_ref")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_sessions_for_worktree(
        &self,
        worktree_id: WorktreeId,
    ) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions WHERE worktree_id = ? ORDER BY created_at ASC"#,
        )
        .bind(worktree_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(Session {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                parent_session_id: parse_optional_session_id(r.try_get("parent_session_id")?),
                relationship: r.try_get("relationship")?,
                provider_id: r.try_get("provider_id")?,
                model_id: r.try_get("model_id")?,
                title: r.try_get("title")?,
                agent_role: r.try_get("agent_role")?,
                status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                provider_session_ref: r.try_get("provider_session_ref")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_subagent_sessions(
        &self,
        parent_session_id: SessionId,
    ) -> Result<Vec<SessionSummary>> {
        let rows = sqlx::query(
            r#"SELECT id, task_id, workspace_id, parent_session_id, relationship,
               provider_id, model_id, title, status, created_at, updated_at
               FROM sessions
               WHERE parent_session_id = ? AND relationship = 'sub_agent'
               ORDER BY created_at ASC"#,
        )
        .bind(parent_session_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(SessionSummary {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                parent_session_id: r
                    .try_get::<Option<String>, _>("parent_session_id")?
                    .and_then(|value| uuid::Uuid::parse_str(&value).ok())
                    .map(SessionId),
                relationship: r.try_get("relationship")?,
                provider_id: r.try_get("provider_id")?,
                model_id: r.try_get("model_id")?,
                title: r.try_get("title")?,
                status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_session_summary_checkpoint(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSummaryCheckpoint>> {
        let row = sqlx::query(
            r#"SELECT session_id, checkpoint_id, summary, last_turn_id, last_event_seq, created_at, updated_at
               FROM session_summary_checkpoints
               WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(row) => row,
            None => return Ok(None),
        };
        let session_id: String = row.try_get("session_id")?;
        let last_turn_id: Option<String> = row.try_get("last_turn_id")?;
        let created_at: String = row.try_get("created_at")?;
        let updated_at: String = row.try_get("updated_at")?;

        Ok(Some(SessionSummaryCheckpoint {
            session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
            checkpoint_id: row.try_get("checkpoint_id")?,
            summary: row.try_get("summary")?,
            last_turn_id: last_turn_id
                .and_then(|value| uuid::Uuid::parse_str(&value).ok())
                .map(TurnId),
            last_event_seq: row.try_get("last_event_seq")?,
            created_at: parse_dt(&created_at)?,
            updated_at: parse_dt(&updated_at)?,
        }))
    }

    pub async fn upsert_session_summary_checkpoint(
        &self,
        checkpoint: SessionSummaryCheckpoint,
    ) -> Result<SessionSummaryCheckpoint> {
        sqlx::query(
            r#"INSERT INTO session_summary_checkpoints (
                   session_id, checkpoint_id, summary, last_turn_id, last_event_seq, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id) DO UPDATE SET
                   checkpoint_id = excluded.checkpoint_id,
                   summary = excluded.summary,
                   last_turn_id = excluded.last_turn_id,
                   last_event_seq = excluded.last_event_seq,
                   updated_at = excluded.updated_at"#,
        )
        .bind(checkpoint.session_id.0.to_string())
        .bind(&checkpoint.checkpoint_id)
        .bind(&checkpoint.summary)
        .bind(checkpoint.last_turn_id.map(|id| id.0.to_string()))
        .bind(checkpoint.last_event_seq)
        .bind(checkpoint.created_at.to_rfc3339())
        .bind(checkpoint.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(checkpoint)
    }

    pub async fn upsert_session_compaction_seed(
        &self,
        session_id: SessionId,
        seed_text: String,
        reason: String,
    ) -> Result<SessionCompactionSeed> {
        let created_at = Utc::now();
        sqlx::query(
            r#"INSERT INTO session_compaction_seeds (
                   session_id, seed_text, reason, created_at
               )
               VALUES (?, ?, ?, ?)
               ON CONFLICT(session_id) DO UPDATE SET
                   seed_text = excluded.seed_text,
                   reason = excluded.reason,
                   created_at = excluded.created_at"#,
        )
        .bind(session_id.0.to_string())
        .bind(&seed_text)
        .bind(&reason)
        .bind(created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(SessionCompactionSeed {
            session_id,
            seed_text,
            reason,
            created_at,
        })
    }

    pub async fn take_session_compaction_seed(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionCompactionSeed>> {
        let row = sqlx::query(
            r#"SELECT seed_text, reason, created_at
               FROM session_compaction_seeds
               WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(row) => row,
            None => return Ok(None),
        };

        sqlx::query(r#"DELETE FROM session_compaction_seeds WHERE session_id = ?"#)
            .bind(session_id.0.to_string())
            .execute(&self.pool)
            .await?;

        let created_at: String = row.try_get("created_at")?;
        Ok(Some(SessionCompactionSeed {
            session_id,
            seed_text: row.try_get("seed_text")?,
            reason: row.try_get("reason")?,
            created_at: parse_dt(&created_at)?,
        }))
    }

    pub async fn has_session_compaction_seed(&self, session_id: SessionId) -> Result<bool> {
        let row = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT 1 FROM session_compaction_seeds WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn upsert_subagent_invocation(
        &self,
        invocation: SubagentInvocation,
    ) -> Result<SubagentInvocation> {
        sqlx::query(
            r#"INSERT INTO subagent_invocations (
                   id, tool_call_id, parent_session_id, parent_turn_id,
                   requested_count, request_json, status, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   tool_call_id = excluded.tool_call_id,
                   parent_session_id = excluded.parent_session_id,
                   parent_turn_id = COALESCE(excluded.parent_turn_id, subagent_invocations.parent_turn_id),
                   requested_count = excluded.requested_count,
                   request_json = COALESCE(excluded.request_json, subagent_invocations.request_json),
                   status = excluded.status,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&invocation.id)
        .bind(&invocation.tool_call_id)
        .bind(invocation.parent_session_id.0.to_string())
        .bind(invocation.parent_turn_id.map(|t| t.0.to_string()))
        .bind(invocation.requested_count)
        .bind(invocation.request_json.as_ref().map(serde_json::to_string).transpose()?)
        .bind(&invocation.status)
        .bind(invocation.created_at.to_rfc3339())
        .bind(invocation.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(invocation)
    }

    pub async fn update_subagent_invocation_status(
        &self,
        id: &str,
        status: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE subagent_invocations
               SET status = ?, updated_at = ?
               WHERE id = ?"#,
        )
        .bind(status)
        .bind(updated_at.to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_subagent_invocation_child(
        &self,
        child: SubagentInvocationChild,
    ) -> Result<SubagentInvocationChild> {
        sqlx::query(
            r#"INSERT INTO subagent_invocation_children (
                   invocation_id, child_session_id, run_id, position, status,
                   label, harness, model, reasoning_effort, prompt_length,
                   created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(invocation_id, child_session_id) DO UPDATE SET
                   run_id = COALESCE(excluded.run_id, subagent_invocation_children.run_id),
                   position = excluded.position,
                   status = excluded.status,
                   label = COALESCE(excluded.label, subagent_invocation_children.label),
                   harness = COALESCE(excluded.harness, subagent_invocation_children.harness),
                   model = COALESCE(excluded.model, subagent_invocation_children.model),
                   reasoning_effort = COALESCE(excluded.reasoning_effort, subagent_invocation_children.reasoning_effort),
                   prompt_length = excluded.prompt_length,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&child.invocation_id)
        .bind(child.child_session_id.0.to_string())
        .bind(child.run_id.as_ref().map(|run_id| run_id.0.to_string()))
        .bind(child.position)
        .bind(&child.status)
        .bind(child.label.as_deref())
        .bind(child.harness.as_deref())
        .bind(child.model.as_deref())
        .bind(child.reasoning_effort.as_deref())
        .bind(child.prompt_length)
        .bind(child.created_at.to_rfc3339())
        .bind(child.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(child)
    }

    pub async fn get_subagent_invocation(&self, id: &str) -> Result<Option<SubagentInvocation>> {
        let row = sqlx::query(
            r#"SELECT id, tool_call_id, parent_session_id, parent_turn_id,
                      requested_count, request_json, status, created_at, updated_at
               FROM subagent_invocations
               WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mut invocation = build_subagent_invocation_from_row(row)?;
        let rows = sqlx::query(
            r#"SELECT invocation_id, child_session_id, run_id, position, status,
                      label, harness, model, reasoning_effort, prompt_length,
                      created_at, updated_at
               FROM subagent_invocation_children
               WHERE invocation_id = ?
               ORDER BY position ASC"#,
        )
        .bind(&invocation.id)
        .fetch_all(&self.pool)
        .await?;

        invocation.children = rows
            .into_iter()
            .filter_map(|r| build_subagent_invocation_child_from_row(r).ok())
            .collect();
        Ok(Some(invocation))
    }

    pub async fn list_subagent_invocations_for_session(
        &self,
        parent_session_id: SessionId,
        parent_turn_id: Option<TurnId>,
    ) -> Result<Vec<SubagentInvocation>> {
        let mut qb = QueryBuilder::new(
            r#"SELECT id, tool_call_id, parent_session_id, parent_turn_id,
                      requested_count, request_json, status, created_at, updated_at
               FROM subagent_invocations
               WHERE parent_session_id = "#,
        );
        qb.push_bind(parent_session_id.0.to_string());
        if let Some(turn_id) = parent_turn_id {
            qb.push(" AND parent_turn_id = ");
            qb.push_bind(turn_id.0.to_string());
        }
        qb.push(" ORDER BY created_at ASC");
        let rows = qb.build().fetch_all(&self.pool).await?;

        let mut invocations = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(invocation) = build_subagent_invocation_from_row(r) {
                invocations.push(invocation);
            }
        }
        if invocations.is_empty() {
            return Ok(invocations);
        }

        let mut child_qb = QueryBuilder::new(
            r#"SELECT invocation_id, child_session_id, run_id, position, status,
                      label, harness, model, reasoning_effort, prompt_length,
                      created_at, updated_at
               FROM subagent_invocation_children
               WHERE invocation_id IN ("#,
        );
        let mut separated = child_qb.separated(", ");
        for invocation in &invocations {
            separated.push_bind(invocation.id.clone());
        }
        child_qb.push(") ORDER BY position ASC");
        let child_rows = child_qb.build().fetch_all(&self.pool).await?;

        let mut children_by_id: HashMap<String, Vec<SubagentInvocationChild>> = HashMap::new();
        for r in child_rows {
            if let Ok(child) = build_subagent_invocation_child_from_row(r) {
                children_by_id
                    .entry(child.invocation_id.clone())
                    .or_default()
                    .push(child);
            }
        }
        for invocation in &mut invocations {
            if let Some(children) = children_by_id.remove(&invocation.id) {
                invocation.children = children;
            }
        }

        Ok(invocations)
    }

    // Message APIs
    pub async fn insert_message(&self, mut message: Message) -> Result<Message> {
        if matches!(message.delivery, MessageDelivery::Immediate) && message.delivered_at.is_none()
        {
            message.delivered_at = Some(Utc::now());
        }
        let attachments_json = if message.attachments.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&message.attachments)
                    .context("serializing message attachments")?,
            )
        };
        sqlx::query(
            r#"INSERT INTO messages (id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(message.id.0.to_string())
        .bind(message.session_id.0.to_string())
        .bind(message.task_id.0.to_string())
        .bind(message.run_id.map(|r| r.0.to_string()))
        .bind(message.turn_id.map(|t| t.0.to_string()))
        .bind(message.turn_sequence)
        .bind(message_role_to_str(&message.role))
        .bind(&message.content)
        .bind(attachments_json)
        .bind(message_delivery_to_str(&message.delivery))
        .bind(message.delivered_at.map(|d| d.to_rfc3339()))
        .bind(message.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.update_task_activity_from_message(&message).await?;
        if matches!(message.role, MessageRole::Assistant) {
            self.update_session_snapshot_last_message(&message).await?;
        }
        Ok(message)
    }

    async fn ensure_session_snapshot_summary(&self, session_id: SessionId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO session_snapshot_summaries (
                    session_id, running_turn_count, created_at, updated_at
               )
               VALUES (?, 0, ?, ?)
               ON CONFLICT(session_id) DO NOTHING"#,
        )
        .bind(session_id.0.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_task_activity_from_message(&self, message: &Message) -> Result<()> {
        let created_at = message.created_at.to_rfc3339();
        let is_assistant = matches!(message.role, MessageRole::Assistant);
        sqlx::query(
            r#"UPDATE tasks
               SET last_activity_at = CASE
                     WHEN last_activity_at IS NULL OR last_activity_at < ? THEN ?
                     ELSE last_activity_at
                   END,
                   last_assistant_message_at = CASE
                     WHEN ? = 1 AND (last_assistant_message_at IS NULL OR last_assistant_message_at < ?)
                       THEN ?
                     ELSE last_assistant_message_at
                   END
               WHERE id = ?"#,
        )
        .bind(&created_at)
        .bind(&created_at)
        .bind(if is_assistant { 1 } else { 0 })
        .bind(&created_at)
        .bind(&created_at)
        .bind(message.task_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_session_snapshot_last_message(&self, message: &Message) -> Result<()> {
        self.ensure_session_snapshot_summary(message.session_id)
            .await?;
        let created_at = message.created_at.to_rfc3339();
        let content = message.content.clone();
        sqlx::query(
            r#"UPDATE session_snapshot_summaries
               SET last_message_at = CASE
                     WHEN last_message_at IS NULL OR last_message_at < ? THEN ?
                     ELSE last_message_at
                   END,
                   last_message_preview = CASE
                     WHEN last_message_at IS NULL OR last_message_at < ? THEN ?
                     ELSE last_message_preview
                   END,
                   updated_at = ?
               WHERE session_id = ?"#,
        )
        .bind(&created_at)
        .bind(&created_at)
        .bind(&created_at)
        .bind(&content)
        .bind(&created_at)
        .bind(message.session_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_session_snapshot_last_event_seq(
        &self,
        session_id: SessionId,
        seq: i64,
    ) -> Result<()> {
        self.ensure_session_snapshot_summary(session_id).await?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE session_snapshot_summaries
               SET last_event_seq = ?,
                   updated_at = ?
               WHERE session_id = ?"#,
        )
        .bind(seq)
        .bind(&now)
        .bind(session_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn refresh_session_turn_summary(&self, session_id: SessionId) -> Result<()> {
        self.ensure_session_snapshot_summary(session_id).await?;
        let last_row = sqlx::query(
            r#"SELECT status, start_seq
               FROM session_turns
               WHERE session_id = ?
               ORDER BY COALESCE(start_seq, -1) DESC, started_at DESC, turn_id DESC
               LIMIT 1"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let (last_status, last_seq) = if let Some(row) = last_row {
            let status: String = row.try_get("status")?;
            let start_seq: Option<i64> = row.try_get("start_seq")?;
            (Some(status), start_seq)
        } else {
            (None, None)
        };
        let running_count = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)
               FROM session_turns
               WHERE session_id = ? AND status = 'running'"#,
        )
        .bind(session_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE session_snapshot_summaries
               SET last_turn_status = ?,
                   last_turn_seq = ?,
                   running_turn_count = ?,
                   updated_at = ?
               WHERE session_id = ?"#,
        )
        .bind(last_status)
        .bind(last_seq)
        .bind(running_count)
        .bind(&now)
        .bind(session_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn workspace_task_counts(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        let active = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM tasks WHERE workspace_id = ? AND archived_at IS NULL"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;

        let archived = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM tasks WHERE workspace_id = ? AND archived_at IS NOT NULL"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok((active, archived))
    }

    pub async fn list_workspace_index_page(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceIndexCursor>,
        limit: i64,
        include_archived: bool,
    ) -> Result<(Vec<WorkspaceTaskSummary>, Option<WorkspaceIndexCursor>)> {
        let filter = if include_archived { None } else { Some(false) };
        self.list_workspace_index_page_filtered(workspace_id, cursor, limit, filter)
            .await
    }

    pub async fn list_workspace_archived_page(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceIndexCursor>,
        limit: i64,
    ) -> Result<(Vec<WorkspaceTaskSummary>, Option<WorkspaceIndexCursor>)> {
        self.list_workspace_index_page_filtered(workspace_id, cursor, limit, Some(true))
            .await
    }

    async fn list_workspace_index_page_filtered(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceIndexCursor>,
        limit: i64,
        archived_only: Option<bool>,
    ) -> Result<(Vec<WorkspaceTaskSummary>, Option<WorkspaceIndexCursor>)> {
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        const ACTIVITY_EXPR: &str = "COALESCE(t.last_activity_at, t.updated_at, t.created_at)";
        const SORT_EXPR: &str = "COALESCE(t.archived_at, t.created_at)";

        let mut sql = format!(
            r#"
            SELECT
              t.id,
              t.workspace_id,
              t.title,
              t.description,
              t.status,
              t.exec_plan_id,
              t.primary_session_id,
              t.primary_worktree_id,
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              t.last_assistant_message_at AS last_assistant_message_at,
              EXISTS(
                SELECT 1
                FROM sessions s
                WHERE s.task_id = t.id AND s.status = 'active'
              ) AS has_active_session,
              ({activity_expr}) AS activity_at,
              ({sort_expr}) AS sort_at
            FROM tasks t
            WHERE t.workspace_id = ?
            "#,
            activity_expr = ACTIVITY_EXPR,
            sort_expr = SORT_EXPR,
        );

        if let Some(archived_only) = archived_only {
            if archived_only {
                sql.push_str(" AND t.archived_at IS NOT NULL");
            } else {
                sql.push_str(" AND t.archived_at IS NULL");
            }
        }

        if cursor.is_some() {
            sql.push_str(&format!(
                " AND (({expr}) < ? OR (({expr}) = ? AND t.id < ?))",
                expr = SORT_EXPR
            ));
        }

        sql.push_str(" ORDER BY sort_at DESC, t.id DESC LIMIT ?");

        let mut query = sqlx::query(&sql).bind(workspace_id.0.to_string());

        if let Some(cursor) = &cursor {
            let cursor_ts = cursor.sort_at.to_rfc3339();
            query = query
                .bind(cursor_ts.clone())
                .bind(cursor_ts)
                .bind(cursor.task_id.0.to_string());
        }

        query = query.bind(limit + 1);

        let rows = query.fetch_all(&self.pool).await?;

        let mut task_rows = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let archived_at: Option<String> = r.try_get("archived_at")?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at")?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id")?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id")?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at")?;
            let has_active_session: i64 = r.try_get("has_active_session")?;
            let activity_at: String = r.try_get("activity_at")?;
            let activity_at_dt = parse_dt(&activity_at)?;
            let sort_at: String = r.try_get("sort_at")?;
            let sort_at_dt = parse_dt(&sort_at)?;

            let task = Task {
                id: TaskId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                title: r.try_get("title")?,
                description: r.try_get("description")?,
                status: parse_task_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
                exec_plan_id: r.try_get("exec_plan_id")?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose()?,
                last_activity_at: Some(activity_at_dt),
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                has_active_session: has_active_session != 0,
            };
            task_rows.push((task, sort_at_dt));
        }

        let mut next_cursor: Option<WorkspaceIndexCursor> = None;
        if task_rows.len() as i64 > limit {
            if let Some((task, sort_at)) = task_rows.pop() {
                next_cursor = Some(WorkspaceIndexCursor {
                    sort_at,
                    task_id: task.id,
                });
            }
        }

        if task_rows.is_empty() {
            return Ok((Vec::new(), next_cursor));
        }

        let summaries = self.build_workspace_task_summaries(task_rows).await?;

        Ok((summaries, next_cursor))
    }

    pub async fn list_workspace_active_page(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        const ACTIVITY_EXPR: &str = "COALESCE(t.last_activity_at, t.updated_at, t.created_at)";

        let total_count = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)
               FROM tasks t
               WHERE t.workspace_id = ?
                 AND t.archived_at IS NULL
                 AND EXISTS (SELECT 1 FROM sessions s WHERE s.task_id = t.id)"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;

        let sql = format!(
            r#"
            SELECT
              t.id,
              t.workspace_id,
              t.title,
              t.description,
              t.status,
              t.exec_plan_id,
              t.primary_session_id,
              t.primary_worktree_id,
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              t.last_assistant_message_at AS last_assistant_message_at,
              EXISTS(
                SELECT 1
                FROM sessions s
                WHERE s.task_id = t.id AND s.status = 'active'
              ) AS has_active_session,
              ({activity_expr}) AS activity_at
            FROM tasks t
            WHERE t.workspace_id = ?
              AND t.archived_at IS NULL
              AND EXISTS (SELECT 1 FROM sessions s WHERE s.task_id = t.id)
            ORDER BY t.created_at DESC, t.id DESC
            LIMIT ?
            "#,
            activity_expr = ACTIVITY_EXPR,
        );

        let rows = sqlx::query(&sql)
            .bind(workspace_id.0.to_string())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

        let mut task_rows = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let archived_at: Option<String> = r.try_get("archived_at")?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at")?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id")?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id")?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at")?;
            let has_active_session: i64 = r.try_get("has_active_session")?;
            let activity_at: String = r.try_get("activity_at")?;
            let activity_at_dt = parse_dt(&activity_at)?;

            let task = Task {
                id: TaskId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                title: r.try_get("title")?,
                description: r.try_get("description")?,
                status: parse_task_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
                exec_plan_id: r.try_get("exec_plan_id")?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose()?,
                last_activity_at: Some(activity_at_dt),
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                has_active_session: has_active_session != 0,
            };
            let sort_at = task.created_at;
            task_rows.push((task, sort_at));
        }

        if task_rows.is_empty() {
            return Ok((Vec::new(), total_count));
        }

        let summaries = self
            .build_workspace_active_task_summaries(task_rows)
            .await?;
        Ok((summaries, total_count))
    }

    pub async fn get_workspace_task_summary(
        &self,
        task_id: TaskId,
    ) -> Result<Option<WorkspaceTaskSummary>> {
        const ACTIVITY_EXPR: &str = "COALESCE(t.last_activity_at, t.updated_at, t.created_at)";
        const SORT_EXPR: &str = "COALESCE(t.archived_at, t.created_at)";
        let sql = format!(
            r#"
            SELECT
              t.id,
              t.workspace_id,
              t.title,
              t.description,
              t.status,
              t.exec_plan_id,
              t.primary_session_id,
              t.primary_worktree_id,
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              t.last_assistant_message_at AS last_assistant_message_at,
              EXISTS(
                SELECT 1
                FROM sessions s
                WHERE s.task_id = t.id AND s.status = 'active'
              ) AS has_active_session,
              ({activity_expr}) AS activity_at,
              ({sort_expr}) AS sort_at
            FROM tasks t
            WHERE t.id = ?
            LIMIT 1
            "#,
            activity_expr = ACTIVITY_EXPR,
            sort_expr = SORT_EXPR,
        );

        if let Some(r) = sqlx::query(&sql)
            .bind(task_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?
        {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let archived_at: Option<String> = r.try_get("archived_at")?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at")?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id")?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id")?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at")?;
            let has_active_session: i64 = r.try_get("has_active_session")?;
            let activity_at: String = r.try_get("activity_at")?;
            let activity_at_dt = parse_dt(&activity_at)?;
            let sort_at: String = r.try_get("sort_at")?;
            let sort_at_dt = parse_dt(&sort_at)?;

            let task = Task {
                id: TaskId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                title: r.try_get("title")?,
                description: r.try_get("description")?,
                status: parse_task_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
                exec_plan_id: r.try_get("exec_plan_id")?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose()?,
                last_activity_at: Some(activity_at_dt),
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                has_active_session: has_active_session != 0,
            };

            let summaries = self
                .build_workspace_task_summaries(vec![(task, sort_at_dt)])
                .await?;
            Ok(summaries.into_iter().next())
        } else {
            Ok(None)
        }
    }

    pub async fn get_workspace_active_task_summary(
        &self,
        task_id: TaskId,
    ) -> Result<Option<WorkspaceActiveTaskSummary>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, title, description, status, exec_plan_id,
                      primary_session_id, primary_worktree_id,
                      created_at, updated_at, archived_at, assistant_seen_at,
                      t.last_assistant_message_at AS last_assistant_message_at,
                      EXISTS(
                        SELECT 1
                        FROM sessions s
                        WHERE s.task_id = t.id AND s.status = 'active'
                      ) AS has_active_session,
                      COALESCE(t.last_activity_at, t.updated_at, t.created_at) AS activity_at
               FROM tasks t
               WHERE id = ?
                 AND archived_at IS NULL
                 AND EXISTS (SELECT 1 FROM sessions s WHERE s.task_id = t.id)"#,
        )
        .bind(task_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            let id: String = r.try_get("id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let archived_at: Option<String> = r.try_get("archived_at")?;
            let assistant_seen_at: Option<String> = r.try_get("assistant_seen_at")?;
            let primary_session_id: Option<String> = r.try_get("primary_session_id")?;
            let primary_worktree_id: Option<String> = r.try_get("primary_worktree_id")?;
            let last_assistant_message_at: Option<String> =
                r.try_get("last_assistant_message_at")?;
            let has_active_session: i64 = r.try_get("has_active_session")?;
            let activity_at: String = r.try_get("activity_at")?;
            let activity_at_dt = parse_dt(&activity_at)?;

            let task = Task {
                id: TaskId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                title: r.try_get("title")?,
                description: r.try_get("description")?,
                status: parse_task_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
                exec_plan_id: r.try_get("exec_plan_id")?,
                primary_session_id: primary_session_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(SessionId),
                primary_worktree_id: primary_worktree_id
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(WorktreeId),
                archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose()?,
                last_activity_at: Some(activity_at_dt),
                last_assistant_message_at: last_assistant_message_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()?,
                has_active_session: has_active_session != 0,
            };

            let sort_at = task.created_at;
            let summaries = self
                .build_workspace_active_task_summaries(vec![(task, sort_at)])
                .await?;
            return Ok(summaries.into_iter().next());
        }
        Ok(None)
    }

    async fn build_workspace_active_task_summaries(
        &self,
        rows: Vec<(Task, DateTime<Utc>)>,
    ) -> Result<Vec<WorkspaceActiveTaskSummary>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let mut seeds = Vec::with_capacity(rows.len());
        for (task, sort_at) in rows {
            seeds.push((task, sort_at));
        }

        let task_ids: Vec<TaskId> = seeds.iter().map(|(task, _)| task.id).collect();
        let session_rows = self.list_session_snapshot_rows(&task_ids).await?;
        let mut sessions_by_task: HashMap<TaskId, Vec<SessionSnapshotSummary>> = HashMap::new();
        for row in session_rows {
            let summary = SessionSnapshotSummary {
                session: session_metadata_from_session(&row.session),
                last_message_at: row.last_message_at,
                last_message_preview: row.last_message_preview,
                last_event_seq: row.last_event_seq,
                state_rev: row.last_event_seq.unwrap_or(0),
                activity: row.activity,
                unread: None,
            };
            sessions_by_task
                .entry(summary.session.task_id)
                .or_default()
                .push(summary);
        }

        let mut summaries = Vec::with_capacity(seeds.len());
        for (task, sort_at) in seeds {
            let mut task_sessions = sessions_by_task.remove(&task.id).unwrap_or_default();
            if task_sessions.is_empty() {
                continue;
            }

            let mut primary_idx = None;
            let mut include_children = false;
            if let Some(primary_id) = task.primary_session_id {
                if let Some(idx) = task_sessions
                    .iter()
                    .position(|summary| summary.session.id == primary_id)
                {
                    primary_idx = Some(idx);
                    include_children = true;
                }
            }
            if primary_idx.is_none() {
                primary_idx = task_sessions
                    .iter()
                    .position(|summary| summary.session.parent_session_id.is_none());
            }
            if primary_idx.is_none() {
                primary_idx = Some(0);
            }

            let primary_summary = task_sessions.remove(primary_idx.unwrap());
            let primary_id = primary_summary.session.id;
            let mut sessions = Vec::new();
            for summary in task_sessions {
                let include = if include_children {
                    summary.session.parent_session_id == Some(primary_id)
                } else {
                    summary.session.parent_session_id.is_none()
                };
                if include {
                    sessions.push(summary);
                }
            }

            let head = match self
                .get_session_head(primary_id, ACTIVE_SNAPSHOT_HEAD_LIMIT, false)
                .await?
            {
                Some(head) => head,
                None => continue,
            };
            let primary_session_head = session_head_to_snapshot(head);

            summaries.push(WorkspaceActiveTaskSummary {
                task,
                primary_session: primary_summary,
                primary_session_head,
                sessions,
                sort_at,
            });
        }

        Ok(summaries)
    }

    pub async fn list_messages_for_session(&self, session_id: SessionId) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages
               WHERE session_id = ?
               ORDER BY created_at ASC, turn_sequence ASC"#,
        )
        .bind(session_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let task_id: String = r.try_get("task_id")?;
            let created_at: String = r.try_get("created_at")?;
            let delivered_at: Option<String> = r.try_get("delivered_at")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let turn_sequence: Option<i64> = r.try_get("turn_sequence")?;
            let attachments_json: Option<String> = r.try_get("attachments_json")?;
            let attachments = attachments_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<MessageAttachment>>(s).ok())
                .unwrap_or_default();
            out.push(Message {
                id: MessageId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                turn_sequence,
                role: parse_message_role(r.try_get::<String, _>("role")?.as_str()),
                content: r.try_get("content")?,
                attachments,
                delivery: parse_message_delivery(r.try_get::<String, _>("delivery")?.as_str()),
                delivered_at: delivered_at.as_deref().map(parse_dt).transpose()?,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_last_assistant_message_for_run(
        &self,
        session_id: SessionId,
        run_id: RunId,
    ) -> Result<Option<Message>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages
               WHERE session_id = ? AND run_id = ? AND role = 'assistant'
               ORDER BY created_at DESC, turn_sequence DESC
               LIMIT 1"#,
        )
        .bind(session_id.0.to_string())
        .bind(run_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let session_id: String = r.try_get("session_id").ok()?;
            let task_id: String = r.try_get("task_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let delivered_at: Option<String> = r.try_get("delivered_at").ok()?;
            let run_id: Option<String> = r.try_get("run_id").ok()?;
            let turn_id: Option<String> = r.try_get("turn_id").ok()?;
            let turn_sequence: Option<i64> = r.try_get("turn_sequence").ok()?;
            let attachments_json: Option<String> = r.try_get("attachments_json").ok()?;
            let attachments = attachments_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<MessageAttachment>>(s).ok())
                .unwrap_or_default();
            Some(Message {
                id: MessageId(uuid::Uuid::parse_str(&id).ok()?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id).ok()?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id).ok()?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                turn_sequence,
                role: parse_message_role(r.try_get::<String, _>("role").ok()?.as_str()),
                content: r.try_get("content").ok()?,
                attachments,
                delivery: parse_message_delivery(r.try_get::<String, _>("delivery").ok()?.as_str()),
                delivered_at: delivered_at.as_deref().map(parse_dt).transpose().ok()?,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn count_user_messages_for_session(&self, session_id: SessionId) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM messages WHERE session_id = ? AND role = 'user'"#,
        )
        .bind(session_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn get_first_user_message_content(
        &self,
        session_id: SessionId,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            r#"SELECT content
               FROM messages
               WHERE session_id = ? AND role = 'user'
               ORDER BY created_at ASC, id ASC
               LIMIT 1"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| r.try_get("content").ok()))
    }

    async fn list_messages_for_turns(
        &self,
        session_id: SessionId,
        turn_ids: &[TurnId],
    ) -> Result<Vec<Message>> {
        let mut query = QueryBuilder::new(
            "SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
             FROM messages
             WHERE session_id = ",
        );
        query.push_bind(session_id.0.to_string());
        if turn_ids.is_empty() {
            query.push(" AND delivery = 'queued' AND delivered_at IS NULL");
        } else {
            query.push(" AND (turn_id IN (");
            let mut first = true;
            for turn_id in turn_ids {
                if !first {
                    query.push(", ");
                }
                first = false;
                query.push_bind(turn_id.0.to_string());
            }
            query.push(") OR (delivery = 'queued' AND delivered_at IS NULL))");
        }
        query.push(" ORDER BY created_at ASC, turn_sequence ASC");

        let rows = query.build().fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let task_id: String = r.try_get("task_id")?;
            let created_at: String = r.try_get("created_at")?;
            let delivered_at: Option<String> = r.try_get("delivered_at")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let turn_sequence: Option<i64> = r.try_get("turn_sequence")?;
            let attachments_json: Option<String> = r.try_get("attachments_json")?;
            let attachments = attachments_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<MessageAttachment>>(s).ok())
                .unwrap_or_default();
            out.push(Message {
                id: MessageId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                turn_sequence,
                role: parse_message_role(r.try_get::<String, _>("role")?.as_str()),
                content: r.try_get("content")?,
                attachments,
                delivery: parse_message_delivery(r.try_get::<String, _>("delivery")?.as_str()),
                delivered_at: delivered_at.as_deref().map(parse_dt).transpose()?,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    async fn build_workspace_task_summaries(
        &self,
        rows: Vec<(Task, DateTime<Utc>)>,
    ) -> Result<Vec<WorkspaceTaskSummary>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        const SESSION_LIMIT: i64 = 4;
        let mut summaries = Vec::with_capacity(rows.len());
        let mut index_by_task = HashMap::new();
        for (idx, (task, sort_at)) in rows.into_iter().enumerate() {
            index_by_task.insert(task.id, idx);
            summaries.push(WorkspaceTaskSummary {
                task,
                provider_ids: Vec::new(),
                sessions: Vec::new(),
                sort_at,
            });
        }

        let task_ids: Vec<TaskId> = summaries.iter().map(|s| s.task.id).collect();

        if !task_ids.is_empty() {
            let mut session_query = QueryBuilder::new(
                "
                SELECT id, task_id, workspace_id, parent_session_id, relationship,
                       provider_id, model_id, title, status, created_at, updated_at
                FROM (
                    SELECT
                        s.*,
                        ROW_NUMBER() OVER (
                            PARTITION BY s.task_id
                            ORDER BY
                                CASE
                                    WHEN s.relationship = 'sub_agent' THEN 1
                                    ELSE 0
                                END,
                                CASE s.status
                                    WHEN 'active' THEN 0
                                    ELSE 1
                                END,
                                s.updated_at DESC
                        ) AS rn
                    FROM sessions s
                    WHERE s.task_id IN (",
            );
            let mut first = true;
            for task_id in &task_ids {
                if !first {
                    session_query.push(", ");
                }
                first = false;
                session_query.push_bind(task_id.0.to_string());
            }
            session_query.push(")) WHERE rn <= ");
            session_query.push_bind(SESSION_LIMIT);
            session_query.push(" ORDER BY task_id, rn");

            let session_rows = session_query.build().fetch_all(&self.pool).await?;

            for r in session_rows {
                let id: String = r.try_get("id")?;
                let task_id: String = r.try_get("task_id")?;
                let ws_id: String = r.try_get("workspace_id")?;
                let created_at: String = r.try_get("created_at")?;
                let updated_at: String = r.try_get("updated_at")?;
                let summary = SessionSummary {
                    id: SessionId(uuid::Uuid::parse_str(&id)?),
                    task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                    workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                    parent_session_id: parse_optional_session_id(r.try_get("parent_session_id")?),
                    relationship: r.try_get("relationship")?,
                    provider_id: r.try_get("provider_id")?,
                    model_id: r.try_get("model_id")?,
                    title: r.try_get("title")?,
                    status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                    created_at: parse_dt(&created_at)?,
                    updated_at: parse_dt(&updated_at)?,
                };
                if let Some(task_idx) = index_by_task.get(&summary.task_id) {
                    summaries[*task_idx].sessions.push(summary.clone());
                    let summary_task = &mut summaries[*task_idx];
                    let pid = summary.provider_id.trim().to_string();
                    if !pid.is_empty() && !summary_task.provider_ids.contains(&pid) {
                        summary_task.provider_ids.push(pid);
                        summary_task.provider_ids.sort();
                        if summary_task.provider_ids.len() > 3 {
                            summary_task.provider_ids.truncate(3);
                        }
                    }
                }
            }
        }

        Ok(summaries)
    }
    async fn list_session_snapshot_rows(
        &self,
        task_ids: &[TaskId],
    ) -> Result<Vec<SessionSnapshotRow>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut session_query = QueryBuilder::new(
            r#"
            SELECT
                s.id,
                s.task_id,
                s.workspace_id,
                s.worktree_id,
                s.parent_session_id,
                s.relationship,
                s.provider_id,
                s.model_id,
                s.title,
                s.agent_role,
                s.status,
                s.provider_session_ref,
                s.parent_session_id,
                s.relationship,
                s.created_at,
                s.updated_at,
                ss.last_message_preview AS last_message_content,
                ss.last_message_at AS last_message_at,
                ss.last_event_seq AS last_event_seq,
                ss.last_turn_status AS last_turn_status,
                COALESCE(ss.running_turn_count, 0) AS running_turn_count
            FROM sessions s
            LEFT JOIN session_snapshot_summaries ss
              ON ss.session_id = s.id
            WHERE s.task_id IN ("#,
        );

        let mut first = true;
        for task_id in task_ids {
            if !first {
                session_query.push(", ");
            }
            first = false;
            session_query.push_bind(task_id.0.to_string());
        }
        session_query.push(") ORDER BY s.created_at ASC");

        let session_rows = session_query.build().fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(session_rows.len());

        for r in session_rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let last_message_at: Option<String> = r.try_get("last_message_at")?;
            let last_message_content: Option<String> = r.try_get("last_message_content")?;
            let last_event_seq: Option<i64> = r.try_get("last_event_seq")?;
            let last_turn_status: Option<String> = r.try_get("last_turn_status")?;
            let running_turn_count: i64 = r.try_get("running_turn_count")?;

            let session = Session {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                provider_id: r.try_get("provider_id")?,
                model_id: r.try_get("model_id")?,
                title: r.try_get("title")?,
                agent_role: r.try_get("agent_role")?,
                status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                provider_session_ref: r.try_get("provider_session_ref")?,
                parent_session_id: parse_optional_session_id(r.try_get("parent_session_id")?),
                relationship: r.try_get("relationship")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            };

            let last_message_preview = last_message_content.as_deref().and_then(|content| {
                let preview = derive_message_preview(content);
                if preview.is_empty() {
                    None
                } else {
                    Some(preview)
                }
            });

            let activity = derive_activity_from_status(
                last_turn_status.as_deref().map(parse_session_turn_status),
                running_turn_count > 0,
            );

            let row = SessionSnapshotRow {
                session,
                last_message_at: last_message_at.as_deref().map(parse_dt).transpose()?,
                last_message_preview,
                last_event_seq,
                activity,
            };
            out.push(row);
        }

        Ok(out)
    }

    async fn get_session_snapshot_summary(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSnapshotSummary>> {
        let row = sqlx::query(
            r#"
            SELECT
                s.id,
                s.task_id,
                s.workspace_id,
                s.worktree_id,
                s.parent_session_id,
                s.relationship,
                s.provider_id,
                s.model_id,
                s.title,
                s.agent_role,
                s.status,
                s.provider_session_ref,
                s.parent_session_id,
                s.relationship,
                s.created_at,
                s.updated_at,
                ss.last_message_preview AS last_message_content,
                ss.last_message_at AS last_message_at,
                ss.last_event_seq AS last_event_seq,
                ss.last_turn_status AS last_turn_status,
                COALESCE(ss.running_turn_count, 0) AS running_turn_count
            FROM sessions s
            LEFT JOIN session_snapshot_summaries ss
              ON ss.session_id = s.id
            WHERE s.id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(r) = row else {
            return Ok(None);
        };

        let id: String = r.try_get("id")?;
        let task_id: String = r.try_get("task_id")?;
        let ws_id: String = r.try_get("workspace_id")?;
        let wt_id: String = r.try_get("worktree_id")?;
        let created_at: String = r.try_get("created_at")?;
        let updated_at: String = r.try_get("updated_at")?;
        let last_message_at: Option<String> = r.try_get("last_message_at")?;
        let last_message_content: Option<String> = r.try_get("last_message_content")?;
        let last_event_seq: Option<i64> = r.try_get("last_event_seq")?;
        let last_turn_status: Option<String> = r.try_get("last_turn_status")?;
        let running_turn_count: i64 = r.try_get("running_turn_count")?;

        let session = Session {
            id: SessionId(uuid::Uuid::parse_str(&id)?),
            task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
            workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
            worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
            provider_id: r.try_get("provider_id")?,
            model_id: r.try_get("model_id")?,
            title: r.try_get("title")?,
            agent_role: r.try_get("agent_role")?,
            status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
            provider_session_ref: r.try_get("provider_session_ref")?,
            parent_session_id: parse_optional_session_id(r.try_get("parent_session_id")?),
            relationship: r.try_get("relationship")?,
            created_at: parse_dt(&created_at)?,
            updated_at: parse_dt(&updated_at)?,
        };

        let last_message_preview = last_message_content.as_deref().and_then(|content| {
            let preview = derive_message_preview(content);
            if preview.is_empty() {
                None
            } else {
                Some(preview)
            }
        });

        let activity = derive_activity_from_status(
            last_turn_status.as_deref().map(parse_session_turn_status),
            running_turn_count > 0,
        );

        Ok(Some(SessionSnapshotSummary {
            session: session_metadata_from_session(&session),
            last_message_at: last_message_at.as_deref().map(parse_dt).transpose()?,
            last_message_preview,
            last_event_seq,
            state_rev: last_event_seq.unwrap_or(0),
            activity,
            unread: None,
        }))
    }

    pub async fn list_queued_messages_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages
               WHERE session_id = ? AND delivery = 'queued' AND delivered_at IS NULL
               ORDER BY created_at ASC, turn_sequence ASC"#,
        )
        .bind(session_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let task_id: String = r.try_get("task_id")?;
            let created_at: String = r.try_get("created_at")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let turn_sequence: Option<i64> = r.try_get("turn_sequence")?;
            let attachments_json: Option<String> = r.try_get("attachments_json")?;
            let attachments = attachments_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<MessageAttachment>>(s).ok())
                .unwrap_or_default();
            out.push(Message {
                id: MessageId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                turn_sequence,
                role: parse_message_role(r.try_get::<String, _>("role")?.as_str()),
                content: r.try_get("content")?,
                attachments,
                delivery: MessageDelivery::Queued,
                delivered_at: None,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_message(&self, id: MessageId) -> Result<Option<Message>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let session_id: String = r.try_get("session_id").ok()?;
            let task_id: String = r.try_get("task_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let delivered_at: Option<String> = r.try_get("delivered_at").ok()?;
            let run_id: Option<String> = r.try_get("run_id").ok()?;
            let turn_id: Option<String> = r.try_get("turn_id").ok()?;
            let turn_sequence: Option<i64> = r.try_get("turn_sequence").ok()?;
            let attachments_json: Option<String> = r.try_get("attachments_json").ok()?;
            let attachments = attachments_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<MessageAttachment>>(s).ok())
                .unwrap_or_default();
            Some(Message {
                id: MessageId(uuid::Uuid::parse_str(&id).ok()?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id).ok()?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id).ok()?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                turn_sequence,
                role: parse_message_role(r.try_get::<String, _>("role").ok()?.as_str()),
                content: r.try_get("content").ok()?,
                attachments,
                delivery: parse_message_delivery(r.try_get::<String, _>("delivery").ok()?.as_str()),
                delivered_at: delivered_at.as_deref().map(parse_dt).transpose().ok()?,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn delete_message(&self, id: MessageId) -> Result<()> {
        sqlx::query(r#"DELETE FROM messages WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_message_delivered(&self, id: MessageId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE messages
               SET delivery = 'immediate', delivered_at = ?
               WHERE id = ?"#,
        )
        .bind(now)
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // Session Turn APIs
    pub async fn insert_session_turn(&self, turn: SessionTurn) -> Result<SessionTurn> {
        let metrics_json = turn
            .metrics_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serializing turn metrics")?;
        sqlx::query(
            r#"INSERT INTO session_turns (
                    turn_id,
                    session_id,
                    run_id,
                    user_message_id,
                    status,
                    start_seq,
                    end_seq,
                    started_at,
                    updated_at,
                    assistant_partial,
                    thought_partial,
                    metrics_json,
                    tool_total,
                    tool_pending,
                    tool_running,
                    tool_completed,
                    tool_failed
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(turn.turn_id.0.to_string())
        .bind(turn.session_id.0.to_string())
        .bind(turn.run_id.map(|r| r.0.to_string()))
        .bind(turn.user_message_id.map(|m| m.0.to_string()))
        .bind(session_turn_status_to_str(&turn.status))
        .bind(turn.start_seq)
        .bind(turn.end_seq)
        .bind(turn.started_at.to_rfc3339())
        .bind(turn.updated_at.to_rfc3339())
        .bind(turn.assistant_partial.as_deref())
        .bind(turn.thought_partial.as_deref())
        .bind(metrics_json)
        .bind(turn.tool_total)
        .bind(turn.tool_pending)
        .bind(turn.tool_running)
        .bind(turn.tool_completed)
        .bind(turn.tool_failed)
        .execute(&self.pool)
        .await?;
        self.refresh_session_turn_summary(turn.session_id).await?;
        Ok(turn)
    }

    pub async fn get_session_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<Option<SessionTurn>> {
        let row = sqlx::query(
            r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                      start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                      metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
               FROM session_turns
               WHERE session_id = ? AND turn_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| build_session_turn_from_row(r).ok()))
    }

    pub async fn delete_session_turn(&self, session_id: SessionId, turn_id: TurnId) -> Result<()> {
        sqlx::query(r#"DELETE FROM session_turns WHERE session_id = ? AND turn_id = ?"#)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .execute(&self.pool)
            .await?;
        self.refresh_session_turn_summary(session_id).await?;
        Ok(())
    }

    pub async fn update_session_turn_partial(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        assistant_partial: Option<&str>,
        thought_partial: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        if assistant_partial.is_none() && thought_partial.is_none() {
            return Ok(());
        }
        sqlx::query(
            r#"UPDATE session_turns
               SET assistant_partial = COALESCE(?, assistant_partial),
                   thought_partial = COALESCE(?, thought_partial),
                   updated_at = ?
               WHERE session_id = ? AND turn_id = ?"#,
        )
        .bind(assistant_partial.map(|s| s.to_string()))
        .bind(thought_partial.map(|s| s.to_string()))
        .bind(updated_at.to_rfc3339())
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .execute(&self.pool)
        .await?;
        self.refresh_session_turn_summary(session_id).await?;
        Ok(())
    }

    pub async fn update_session_turn_status(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        status: SessionTurnStatus,
        end_seq: Option<i64>,
        metrics_json: Option<&serde_json::Value>,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        let metrics_json = metrics_json
            .map(serde_json::to_string)
            .transpose()
            .context("serializing turn metrics")?;
        sqlx::query(
            r#"UPDATE session_turns
               SET status = ?,
                   end_seq = COALESCE(?, end_seq),
                   metrics_json = COALESCE(?, metrics_json),
                   updated_at = ?
               WHERE session_id = ? AND turn_id = ?"#,
        )
        .bind(session_turn_status_to_str(&status))
        .bind(end_seq)
        .bind(metrics_json)
        .bind(updated_at.to_rfc3339())
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .execute(&self.pool)
        .await?;
        self.refresh_session_turn_summary(session_id).await?;
        Ok(())
    }

    pub async fn update_session_turn_tool_counts(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        deltas: SessionTurnToolCountDeltas,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE session_turns
               SET tool_total = tool_total + ?,
                   tool_pending = tool_pending + ?,
                   tool_running = tool_running + ?,
                   tool_completed = tool_completed + ?,
                   tool_failed = tool_failed + ?,
                   updated_at = ?
               WHERE session_id = ? AND turn_id = ?"#,
        )
        .bind(deltas.total)
        .bind(deltas.pending)
        .bind(deltas.running)
        .bind(deltas.completed)
        .bind(deltas.failed)
        .bind(updated_at.to_rfc3339())
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_session_turns_page_by_seq(
        &self,
        session_id: SessionId,
        before_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<SessionTurn>> {
        let limit = limit.unwrap_or(50).clamp(1, 500) as i64;
        let rows = if let Some(before_seq) = before_seq {
            sqlx::query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ? AND start_seq < ?
                   ORDER BY start_seq DESC
                   LIMIT ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(before_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ?
                   ORDER BY start_seq DESC
                   LIMIT ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(turn) = build_session_turn_from_row(r) {
                out.push(turn);
            }
        }
        out.reverse();
        Ok(out)
    }

    pub async fn list_session_turns_by_statuses(
        &self,
        statuses: &[SessionTurnStatus],
    ) -> Result<Vec<SessionTurn>> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let mut qb = QueryBuilder::new(
            r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                      start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                      metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
               FROM session_turns
               WHERE status IN ("#,
        );
        let mut separated = qb.separated(", ");
        for status in statuses {
            separated.push_bind(session_turn_status_to_str(status));
        }
        qb.push(") ORDER BY updated_at ASC");
        let rows = qb.build().fetch_all(&self.pool).await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(turn) = build_session_turn_from_row(r) {
                out.push(turn);
            }
        }
        Ok(out)
    }

    async fn load_session_head_materialization(
        &self,
        session_id: SessionId,
        kind: SessionHeadKind,
    ) -> Result<Option<SessionHeadMaterialization>> {
        let row = sqlx::query(
            r#"SELECT last_event_seq, turns_json, tool_summaries_json, events_json,
                      messages_json, has_more_turns, head_window_json
               FROM session_head_materializations
               WHERE session_id = ? AND head_kind = ?"#,
        )
        .bind(session_id.0.to_string())
        .bind(session_head_kind_to_str(kind))
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let turns_json: String = row.try_get("turns_json")?;
        let tool_summaries_json: String = row.try_get("tool_summaries_json")?;
        let events_json: String = row.try_get("events_json")?;
        let messages_json: String = row.try_get("messages_json")?;
        let head_window_json: String = row.try_get("head_window_json")?;

        let turns: Vec<SessionTurn> = match serde_json::from_str(&turns_json) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let tool_summaries: Vec<SessionTurnToolSummary> =
            match serde_json::from_str(&tool_summaries_json) {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
        let events: Vec<SessionEvent> = match serde_json::from_str(&events_json) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let messages: Vec<Message> = match serde_json::from_str(&messages_json) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let head_window: SessionHeadWindow = match serde_json::from_str(&head_window_json) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        let has_more_turns: i64 = row.try_get("has_more_turns")?;

        Ok(Some(SessionHeadMaterialization {
            last_event_seq: row.try_get("last_event_seq")?,
            turns,
            tool_summaries,
            events,
            messages,
            has_more_turns: has_more_turns != 0,
            head_window,
        }))
    }

    async fn upsert_session_head_materialization(
        &self,
        session_id: SessionId,
        kind: SessionHeadKind,
        head: &SessionHeadMaterialization,
    ) -> Result<i64> {
        let turns_json =
            serde_json::to_string(&head.turns).context("serializing session head turns")?;
        let tool_summaries_json = serde_json::to_string(&head.tool_summaries)
            .context("serializing session head tool summaries")?;
        let events_json =
            serde_json::to_string(&head.events).context("serializing session head events")?;
        let messages_json =
            serde_json::to_string(&head.messages).context("serializing session head messages")?;
        let head_window_json =
            serde_json::to_string(&head.head_window).context("serializing session head window")?;
        let now = Utc::now().to_rfc3339();

        let head_rev: i64 = sqlx::query_scalar(
            r#"INSERT INTO session_head_materializations (
                    session_id, head_kind, head_rev, last_event_seq,
                    turns_json, tool_summaries_json, events_json, messages_json,
                    has_more_turns, head_window_json, created_at, updated_at
               )
               VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id, head_kind) DO UPDATE SET
                   head_rev = session_head_materializations.head_rev + 1,
                   last_event_seq = excluded.last_event_seq,
                   turns_json = excluded.turns_json,
                   tool_summaries_json = excluded.tool_summaries_json,
                   events_json = excluded.events_json,
                   messages_json = excluded.messages_json,
                   has_more_turns = excluded.has_more_turns,
                   head_window_json = excluded.head_window_json,
                   updated_at = excluded.updated_at
               RETURNING head_rev"#,
        )
        .bind(session_id.0.to_string())
        .bind(session_head_kind_to_str(kind))
        .bind(head.last_event_seq)
        .bind(turns_json)
        .bind(tool_summaries_json)
        .bind(events_json)
        .bind(messages_json)
        .bind(if head.has_more_turns { 1 } else { 0 })
        .bind(head_window_json)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await?;

        Ok(head_rev)
    }

    async fn session_head_kind_for_task(&self, task_id: TaskId) -> Result<SessionHeadKind> {
        let archived_at: Option<Option<String>> =
            sqlx::query_scalar(r#"SELECT archived_at FROM tasks WHERE id = ?"#)
                .bind(task_id.0.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(if archived_at.flatten().is_some() {
            SessionHeadKind::Archived
        } else {
            SessionHeadKind::Active
        })
    }

    async fn materialize_session_head(
        &self,
        session: &Session,
        kind: SessionHeadKind,
        last_event_seq: i64,
    ) -> Result<SessionHead> {
        let turn_limit = match kind {
            SessionHeadKind::Active => SESSION_HEAD_MAX_TURNS,
            SessionHeadKind::Archived => SESSION_HEAD_ARCHIVED_TURN_LIMIT,
        };
        let limits = session_head_limits(kind, turn_limit);
        let head = self
            .build_session_head(session, limits, true, last_event_seq)
            .await?;
        let materialized = SessionHeadMaterialization::from_head(&head);
        self.upsert_session_head_materialization(session.id, kind, &materialized)
            .await?;
        Ok(head)
    }

    async fn delete_session_head_materializations_for_task(
        &self,
        task_id: TaskId,
        kind: SessionHeadKind,
    ) -> Result<()> {
        sqlx::query(
            r#"DELETE FROM session_head_materializations
               WHERE head_kind = ?
                 AND session_id IN (SELECT id FROM sessions WHERE task_id = ?)"#,
        )
        .bind(session_head_kind_to_str(kind))
        .bind(task_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn materialize_archived_heads_for_task(&self, task_id: TaskId) -> Result<()> {
        let sessions = self.list_sessions_for_task(task_id).await?;
        if sessions.is_empty() {
            return Ok(());
        }
        for session in sessions {
            let last_event_seq = self.session_last_event_seq(session.id).await?;
            self.materialize_session_head(&session, SessionHeadKind::Archived, last_event_seq)
                .await?;
        }
        Ok(())
    }

    async fn build_session_head(
        &self,
        session: &Session,
        limits: SessionHeadLimits,
        include_events: bool,
        last_event_seq: i64,
    ) -> Result<SessionHead> {
        let limit = limits.turn_limit as i64;
        let rows = sqlx::query(
            r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                      start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                      metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
               FROM session_turns
               WHERE session_id = ?
               ORDER BY start_seq DESC
               LIMIT ?"#,
        )
        .bind(session.id.0.to_string())
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;

        let mut has_more_turns = false;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if out.len() as i64 >= limit {
                has_more_turns = true;
                break;
            }
            if let Ok(turn) = build_session_turn_from_row(r) {
                out.push(turn);
            }
        }
        out.reverse();

        let turn_ids: Vec<TurnId> = out.iter().map(|t| t.turn_id).collect();
        let mut messages = self.list_messages_for_turns(session.id, &turn_ids).await?;
        let mut tool_summaries = self
            .list_turn_tool_summaries_for_turns(session.id, &turn_ids)
            .await?;
        if !turn_ids.is_empty() {
            let mut tool_ids: HashMap<String, bool> = HashMap::new();
            for tool in &tool_summaries {
                tool_ids.insert(tool.tool_call_id.clone(), true);
            }
            for turn in &out {
                if turn.tool_total <= 0 {
                    continue;
                }
                let has_any = tool_summaries
                    .iter()
                    .any(|tool| tool.turn_id == turn.turn_id);
                if has_any {
                    continue;
                }
                let tools = self.list_turn_tools(session.id, turn.turn_id).await?;
                for tool in tools {
                    if tool_ids.contains_key(&tool.tool_call_id) {
                        continue;
                    }
                    tool_ids.insert(tool.tool_call_id.clone(), true);
                    tool_summaries.push(summarize_session_turn_tool(&tool));
                }
            }
            tool_summaries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        }
        let last_status = out.last().map(|t| t.status.clone());
        let has_running_turn = out
            .iter()
            .any(|turn| matches!(turn.status, SessionTurnStatus::Running));
        let activity = derive_activity_from_status(last_status, has_running_turn);
        let mut events = if include_events {
            let mut events = self
                .list_session_events_tail_by_seq(session.id, limits.event_limit as u32, false)
                .await?;
            events.sort_by(|a, b| a.seq.cmp(&b.seq));
            events
        } else {
            Vec::new()
        };

        let summary_checkpoint = self.get_session_summary_checkpoint(session.id).await?;
        let head_window = trim_session_head_window(
            &mut out,
            &mut messages,
            &mut tool_summaries,
            &mut events,
            &mut has_more_turns,
            limits.turn_limit,
            limits.message_limit,
            limits.event_limit,
            limits.byte_limit,
        );

        Ok(SessionHead {
            session: session.clone(),
            turns: out,
            tool_summaries,
            events,
            messages,
            last_event_seq,
            activity,
            has_more_turns,
            summary_checkpoint,
            head_window,
        })
    }

    pub async fn get_session_head(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionHead>> {
        self.get_session_head_with_kind(session_id, limit, include_events, None)
            .await
    }

    pub async fn get_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionHeadSnapshot>> {
        Ok(self
            .get_session_head(session_id, limit, include_events)
            .await?
            .map(session_head_to_snapshot))
    }

    async fn get_session_head_with_kind(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
        head_kind_override: Option<SessionHeadKind>,
    ) -> Result<Option<SessionHead>> {
        let session = match self.get_session(session_id).await? {
            Some(session) => session,
            None => return Ok(None),
        };
        let head_kind = match head_kind_override {
            Some(kind) => kind,
            None => self.session_head_kind_for_task(session.task_id).await?,
        };
        let last_event_seq = self.session_last_event_seq(session_id).await?;

        if let Some(materialized) = self
            .load_session_head_materialization(session_id, head_kind)
            .await?
        {
            if materialized.last_event_seq == last_event_seq {
                let summary_checkpoint = self.get_session_summary_checkpoint(session_id).await?;
                let head = materialized.into_session_head(session, summary_checkpoint);
                let limits = session_head_limits(head_kind, limit);
                return Ok(Some(apply_session_head_limits(
                    head,
                    limits,
                    include_events,
                )));
            }
        }

        let head = self
            .materialize_session_head(&session, head_kind, last_event_seq)
            .await?;
        let limits = session_head_limits(head_kind, limit);
        Ok(Some(apply_session_head_limits(
            head,
            limits,
            include_events,
        )))
    }

    pub async fn get_session_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionSnapshot>> {
        let summary = match self.get_session_snapshot_summary(session_id).await? {
            Some(summary) => summary,
            None => return Ok(None),
        };
        let head = match self
            .get_session_head_with_kind(session_id, limit, include_events, None)
            .await?
        {
            Some(head) => head,
            None => return Ok(None),
        };
        let state = self.get_session_state(session_id).await?;
        Ok(Some(SessionSnapshot {
            summary,
            head: session_head_to_snapshot(head),
            state: Some(state),
        }))
    }

    pub async fn get_session_history_page(
        &self,
        session_id: SessionId,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Option<SessionHistoryPage>> {
        if self.get_session(session_id).await?.is_none() {
            return Ok(None);
        }
        let limit = limit.clamp(1, 200) as i64;
        let rows = if let Some(before_seq) = before_seq {
            sqlx::query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ? AND start_seq < ?
                   ORDER BY start_seq DESC
                   LIMIT ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(before_seq)
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ?
                   ORDER BY start_seq DESC
                   LIMIT ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(limit + 1)
            .fetch_all(&self.pool)
            .await?
        };

        let mut has_more = false;
        let mut turns = Vec::with_capacity(rows.len());
        for r in rows {
            if turns.len() as i64 >= limit {
                has_more = true;
                break;
            }
            if let Ok(turn) = build_session_turn_from_row(r) {
                turns.push(turn);
            }
        }
        turns.reverse();

        let next_cursor = if has_more {
            turns.first().and_then(|t| t.start_seq)
        } else {
            None
        };

        let turn_ids: Vec<TurnId> = turns.iter().map(|t| t.turn_id).collect();
        let messages = self.list_messages_for_turns(session_id, &turn_ids).await?;

        Ok(Some(SessionHistoryPage {
            session_id,
            turns,
            messages,
            next_cursor,
            has_more,
        }))
    }

    pub async fn list_turn_tools(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<Vec<SessionTurnTool>> {
        let rows = sqlx::query(
            r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      output_text, created_at, updated_at
               FROM session_turn_tools
               WHERE session_id = ? AND turn_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;
        if !rows.is_empty() {
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                if let Ok(tool) = build_session_turn_tool_from_row(r) {
                    out.push(tool);
                }
            }
            return Ok(out);
        }

        let events = self
            .list_session_events_for_turn(session_id, turn_id, false)
            .await?;
        let tools = build_turn_tools_from_events(session_id, turn_id, &events);
        for tool in &tools {
            let _ = self.upsert_session_turn_tool(tool.clone()).await;
        }
        Ok(tools)
    }

    pub async fn list_turn_tool_summaries_for_turns(
        &self,
        session_id: SessionId,
        turn_ids: &[TurnId],
    ) -> Result<Vec<SessionTurnToolSummary>> {
        if turn_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut qb = QueryBuilder::new(
            r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      created_at, updated_at
               FROM session_turn_tools
               WHERE session_id = "#,
        );
        qb.push_bind(session_id.0.to_string());
        qb.push(" AND turn_id IN (");
        let mut separated = qb.separated(", ");
        for turn_id in turn_ids {
            separated.push_bind(turn_id.0.to_string());
        }
        qb.push(") ORDER BY created_at ASC");
        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(tool) = build_session_turn_tool_summary_from_row(r) {
                out.push(tool);
            }
        }
        Ok(out)
    }

    pub async fn get_session_turn_tool(
        &self,
        session_id: SessionId,
        tool_call_id: &str,
    ) -> Result<Option<SessionTurnTool>> {
        let row = sqlx::query(
            r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      output_text, created_at, updated_at
               FROM session_turn_tools
               WHERE session_id = ? AND tool_call_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .bind(tool_call_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| build_session_turn_tool_from_row(r).ok()))
    }

    pub async fn upsert_session_turn_tool(&self, tool: SessionTurnTool) -> Result<SessionTurnTool> {
        let input_json = tool
            .input_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serializing tool input")?;
        sqlx::query(
            r#"INSERT INTO session_turn_tools (
                    session_id, tool_call_id, turn_id, tool_kind, title, status,
                    input_json, output_text, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id, tool_call_id) DO UPDATE SET
                   turn_id = excluded.turn_id,
                   tool_kind = COALESCE(excluded.tool_kind, session_turn_tools.tool_kind),
                   title = COALESCE(excluded.title, session_turn_tools.title),
                   status = COALESCE(excluded.status, session_turn_tools.status),
                   input_json = COALESCE(excluded.input_json, session_turn_tools.input_json),
                   output_text = COALESCE(excluded.output_text, session_turn_tools.output_text),
                   updated_at = excluded.updated_at"#,
        )
        .bind(tool.session_id.0.to_string())
        .bind(&tool.tool_call_id)
        .bind(tool.turn_id.0.to_string())
        .bind(tool.tool_kind.as_deref())
        .bind(tool.title.as_deref())
        .bind(tool.status.as_deref())
        .bind(input_json)
        .bind(tool.output_text.as_deref())
        .bind(tool.created_at.to_rfc3339())
        .bind(tool.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(tool)
    }

    // Blob APIs
    pub async fn insert_blob(
        &self,
        id: &str,
        sha256: &str,
        bytes: i64,
        mime_type: &str,
        name: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO blobs (id, sha256, bytes, mime_type, name, created_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(id)
        .bind(sha256)
        .bind(bytes)
        .bind(mime_type)
        .bind(name)
        .bind(created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_blob(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, i64, Option<String>, DateTime<Utc>)>> {
        let row = sqlx::query(
            r#"SELECT sha256, mime_type, bytes, name, created_at
               FROM blobs WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let sha256: String = r.try_get("sha256").unwrap_or_default();
            let mime_type: String = r.try_get("mime_type").unwrap_or_default();
            let bytes: i64 = r.try_get("bytes").unwrap_or_default();
            let name: Option<String> = r.try_get("name").ok();
            let created_at: String = r.try_get("created_at").unwrap_or_default();
            let created_at = parse_dt(&created_at).unwrap_or_else(|_| Utc::now());
            (sha256, mime_type, bytes, name, created_at)
        }))
    }

    // Artifact APIs
    pub async fn list_session_artifacts(&self, session_id: SessionId) -> Result<Vec<Artifact>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, task_id, workspace_id, worktree_id,
                      name, absolute_path, mime_type, bytes, created_at
               FROM artifacts
               WHERE session_id = ?
               ORDER BY position ASC"#,
        )
        .bind(session_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(artifact) = build_artifact_from_row(r) {
                out.push(artifact);
            }
        }
        Ok(out)
    }

    pub async fn upsert_session_git_status_summary(
        &self,
        session_id: SessionId,
        worktree_id: WorktreeId,
        summary: &SessionGitStatusSummary,
    ) -> Result<()> {
        let summary_json =
            serde_json::to_string(summary).context("serializing git status summary")?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO session_git_status_snapshots (
                    session_id, worktree_id, summary_json, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(session_id) DO UPDATE SET
                   worktree_id = excluded.worktree_id,
                   summary_json = excluded.summary_json,
                   updated_at = excluded.updated_at"#,
        )
        .bind(session_id.0.to_string())
        .bind(worktree_id.0.to_string())
        .bind(summary_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session_git_status_summary(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionGitStatusSummary>> {
        let row = sqlx::query(
            r#"SELECT summary_json
               FROM session_git_status_snapshots
               WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let summary_json: String = row.try_get("summary_json")?;
        Ok(serde_json::from_str(&summary_json).ok())
    }

    pub async fn get_session_state(&self, session_id: SessionId) -> Result<SessionState> {
        let artifacts = self.list_session_artifacts(session_id).await?;
        let git_status = self.get_session_git_status_summary(session_id).await?;
        Ok(SessionState {
            artifacts,
            git_status,
        })
    }

    pub async fn get_artifact(&self, id: ArtifactId) -> Result<Option<Artifact>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, task_id, workspace_id, worktree_id,
                      name, absolute_path, mime_type, bytes, created_at
               FROM artifacts
               WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| build_artifact_from_row(r).ok()))
    }

    pub async fn replace_session_artifacts(
        &self,
        session_id: SessionId,
        artifacts: &[Artifact],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(r#"DELETE FROM artifacts WHERE session_id = ?"#)
            .bind(session_id.0.to_string())
            .execute(&mut *tx)
            .await?;

        for (idx, artifact) in artifacts.iter().enumerate() {
            sqlx::query(
                r#"INSERT INTO artifacts (
                        id, session_id, task_id, workspace_id, worktree_id,
                        position, name, absolute_path, mime_type, bytes, created_at
                   )
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(artifact.id.0.to_string())
            .bind(artifact.session_id.0.to_string())
            .bind(artifact.task_id.0.to_string())
            .bind(artifact.workspace_id.0.to_string())
            .bind(artifact.worktree_id.0.to_string())
            .bind(idx as i64)
            .bind(artifact.name.as_deref())
            .bind(&artifact.absolute_path)
            .bind(&artifact.mime_type)
            .bind(artifact.bytes)
            .bind(artifact.created_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    // Session event APIs
    pub async fn append_session_event(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> Result<SessionEvent> {
        crate::fault_injection::maybe_fail("ctx_store.append_session_event")?;
        let transient = is_transient_session_event(&event_type, &payload_json);
        let mut event = SessionEvent {
            seq: 0,
            id: SessionEventId::new(),
            session_id,
            run_id,
            turn_id,
            event_type,
            payload_json: payload_json.clone(),
            transient,
            created_at: Utc::now(),
        };
        let seq: i64 = sqlx::query_scalar(
            r#"INSERT INTO session_events (id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)
               RETURNING seq"#,
        )
        .bind(event.id.0.to_string())
        .bind(event.session_id.0.to_string())
        .bind(event.run_id.map(|r| r.0.to_string()))
        .bind(event.turn_id.map(|t| t.0.to_string()))
        .bind(session_event_type_to_str(&event.event_type))
        .bind(payload_json.to_string())
        .bind(if transient { 1 } else { 0 })
        .bind(event.created_at.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        event.seq = seq;
        self.update_session_snapshot_last_event_seq(event.session_id, event.seq)
            .await?;
        if let Some(turn_id) = event.turn_id {
            if let Some(tool) = build_turn_tool_from_event(&event, turn_id) {
                let _ = self.upsert_session_turn_tool(tool).await;
            }
        }
        Ok(event)
    }

    pub async fn list_session_events(&self, session_id: SessionId) -> Result<Vec<SessionEvent>> {
        self.list_session_events_page_by_seq(session_id, None, None, false)
            .await
    }

    async fn session_last_event_seq(&self, session_id: SessionId) -> Result<i64> {
        let seq = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT MAX(seq) FROM session_events WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(seq.unwrap_or(0))
    }

    pub async fn get_session_last_event_seq(&self, session_id: SessionId) -> Result<i64> {
        self.session_last_event_seq(session_id).await
    }

    pub async fn list_session_events_page_by_seq(
        &self,
        session_id: SessionId,
        after_seq: Option<i64>,
        limit: Option<u32>,
        include_transient: bool,
    ) -> Result<Vec<SessionEvent>> {
        crate::fault_injection::maybe_fail("ctx_store.list_session_events_page_by_seq")?;
        let session_id_str = session_id.0.to_string();
        let limit_i64 = limit.map(|n| n as i64);
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = if let Some(after_seq) = after_seq {
            if let Some(limit) = limit_i64 {
                sqlx::query(
                    r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
                       FROM session_events
                       WHERE session_id = ?
                         AND (? = 1 OR transient = 0)
                         AND seq > ?
                       ORDER BY seq ASC
                       LIMIT ?"#,
                )
                .bind(&session_id_str)
                .bind(include_transient)
                .bind(after_seq)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query(
                    r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
                       FROM session_events
                       WHERE session_id = ?
                         AND (? = 1 OR transient = 0)
                         AND seq > ?
                       ORDER BY seq ASC"#,
                )
                .bind(&session_id_str)
                .bind(include_transient)
                .bind(after_seq)
                .fetch_all(&self.pool)
                .await?
            }
        } else if let Some(limit) = limit_i64 {
            sqlx::query(
                r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
                   FROM session_events
                   WHERE session_id = ?
                     AND (? = 1 OR transient = 0)
                   ORDER BY seq ASC
                   LIMIT ?"#,
            )
            .bind(&session_id_str)
            .bind(include_transient)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
                   FROM session_events
                   WHERE session_id = ?
                     AND (? = 1 OR transient = 0)
                   ORDER BY seq ASC"#,
            )
            .bind(&session_id_str)
            .bind(include_transient)
            .fetch_all(&self.pool)
            .await?
        };

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let seq: i64 = r.try_get("seq")?;
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let created_at: String = r.try_get("created_at")?;
            let payload_json: String = r.try_get("payload_json")?;
            let transient: i64 = r.try_get("transient")?;
            out.push(SessionEvent {
                seq,
                id: SessionEventId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                event_type: parse_session_event_type(
                    r.try_get::<String, _>("event_type")?.as_str(),
                ),
                payload_json: serde_json::from_str(&payload_json)
                    .context("parsing payload_json")?,
                transient: transient != 0,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_session_events_for_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        include_transient: bool,
    ) -> Result<Vec<SessionEvent>> {
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = sqlx::query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
               FROM session_events
               WHERE session_id = ? AND turn_id = ? AND (? = 1 OR transient = 0)
               ORDER BY seq ASC"#,
        )
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
        .bind(include_transient)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let created_at: String = r.try_get("created_at")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let payload_json: String = r.try_get("payload_json")?;
            let transient: i64 = r.try_get("transient")?;
            out.push(SessionEvent {
                seq: r.try_get("seq")?,
                id: SessionEventId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                event_type: parse_session_event_type(
                    r.try_get::<String, _>("event_type")?.as_str(),
                ),
                payload_json: serde_json::from_str(&payload_json)
                    .context("parsing session event payload")?,
                transient: transient != 0,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_terminal_event_for_run(
        &self,
        session_id: SessionId,
        run_id: RunId,
    ) -> Result<Option<SessionEvent>> {
        let row = sqlx::query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
               FROM session_events
               WHERE session_id = ? AND run_id = ? AND event_type IN ('done', 'error', 'turn_interrupted')
               ORDER BY seq DESC
               LIMIT 1"#,
        )
        .bind(session_id.0.to_string())
        .bind(run_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let session_id: String = r.try_get("session_id").ok()?;
            let run_id: Option<String> = r.try_get("run_id").ok()?;
            let turn_id: Option<String> = r.try_get("turn_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let payload_json: String = r.try_get("payload_json").ok()?;
            let transient: i64 = r.try_get("transient").ok()?;
            Some(SessionEvent {
                seq: r.try_get("seq").ok()?,
                id: SessionEventId(uuid::Uuid::parse_str(&id).ok()?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id).ok()?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                event_type: parse_session_event_type(
                    r.try_get::<String, _>("event_type").ok()?.as_str(),
                ),
                payload_json: serde_json::from_str(&payload_json).ok()?,
                transient: transient != 0,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn delete_session_events_for_turn_types(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        event_types: &[SessionEventType],
    ) -> Result<()> {
        if event_types.is_empty() {
            return Ok(());
        }
        let mut sql = String::from(
            "DELETE FROM session_events WHERE session_id = ? AND turn_id = ? AND event_type IN (",
        );
        for i in 0..event_types.len() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
        }
        sql.push(')');
        let mut q = sqlx::query(&sql)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string());
        for t in event_types {
            q = q.bind(session_event_type_to_str(t));
        }
        q.execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_session_events_tail_by_seq(
        &self,
        session_id: SessionId,
        limit: u32,
        include_transient: bool,
    ) -> Result<Vec<SessionEvent>> {
        let session_id_str = session_id.0.to_string();
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = sqlx::query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
               FROM session_events
               WHERE session_id = ?
                 AND (? = 1 OR transient = 0)
               ORDER BY seq DESC
               LIMIT ?"#,
        )
        .bind(session_id_str)
        .bind(include_transient)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let seq: i64 = r.try_get("seq")?;
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let created_at: String = r.try_get("created_at")?;
            let payload_json: String = r.try_get("payload_json")?;
            let transient: i64 = r.try_get("transient")?;
            out.push(SessionEvent {
                seq,
                id: SessionEventId(uuid::Uuid::parse_str(&id)?),
                session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                run_id: run_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RunId),
                turn_id: turn_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(TurnId),
                event_type: parse_session_event_type(
                    r.try_get::<String, _>("event_type")?.as_str(),
                ),
                payload_json: serde_json::from_str(&payload_json)
                    .context("parsing payload_json")?,
                transient: transient != 0,
                created_at: parse_dt(&created_at)?,
            });
        }
        out.reverse(); // return ASC
        Ok(out)
    }

    // Mobile connection profiles + devices
    pub async fn create_mobile_connection_profile(
        &self,
        label: String,
        base_url: String,
        token_hash: String,
        token_prefix: String,
        scopes: Vec<String>,
    ) -> Result<MobileConnectionProfile> {
        let now = Utc::now();
        let profile = MobileConnectionProfile {
            id: ConnectionProfileId::new(),
            label,
            base_url,
            token_prefix,
            scopes,
            created_at: now,
            last_used_at: None,
        };
        let scopes_json = serde_json::to_string(&profile.scopes)?;
        sqlx::query(
            r#"INSERT INTO mobile_connection_profiles
               (id, label, base_url, token_hash, token_prefix, scopes_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(profile.id.0.to_string())
        .bind(&profile.label)
        .bind(&profile.base_url)
        .bind(&token_hash)
        .bind(&profile.token_prefix)
        .bind(scopes_json)
        .bind(profile.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(profile)
    }

    pub async fn list_mobile_connection_profiles(&self) -> Result<Vec<MobileConnectionProfile>> {
        let rows = sqlx::query(
            r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles
               ORDER BY created_at DESC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(build_mobile_connection_profile_from_row(row)?);
        }
        Ok(out)
    }

    pub async fn get_mobile_connection_profile(
        &self,
        id: ConnectionProfileId,
    ) -> Result<Option<MobileConnectionProfile>> {
        let row = sqlx::query(
            r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(build_mobile_connection_profile_from_row)
            .transpose()
    }

    pub async fn get_mobile_connection_profile_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<MobileConnectionProfile>> {
        let row = sqlx::query(
            r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles WHERE token_hash = ?"#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(build_mobile_connection_profile_from_row)
            .transpose()
    }

    pub async fn mark_mobile_connection_profile_used(&self, id: ConnectionProfileId) -> Result<()> {
        sqlx::query(r#"UPDATE mobile_connection_profiles SET last_used_at = ? WHERE id = ?"#)
            .bind(Utc::now().to_rfc3339())
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_mobile_connection_profile(&self, id: ConnectionProfileId) -> Result<()> {
        sqlx::query(r#"DELETE FROM mobile_connection_profiles WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_mobile_access_config(&self) -> Result<Option<MobileAccessConfig>> {
        let row = sqlx::query(
            r#"SELECT id, profile_id, tunnel_id, public_base_url, relay_base_url, tunnel_secret,
                      daemon_public_key, daemon_private_key, enabled, created_at, updated_at
               FROM mobile_access_config
               WHERE id = ?"#,
        )
        .bind("default")
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(MobileAccessConfig {
            id: row.try_get("id")?,
            profile_id: ConnectionProfileId(uuid::Uuid::parse_str(
                &row.try_get::<String, _>("profile_id")?,
            )?),
            tunnel_id: row.try_get("tunnel_id")?,
            public_base_url: row.try_get("public_base_url")?,
            relay_base_url: row.try_get("relay_base_url")?,
            tunnel_secret: row.try_get("tunnel_secret")?,
            daemon_public_key: row.try_get("daemon_public_key")?,
            daemon_private_key: row.try_get("daemon_private_key")?,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            created_at: parse_dt(&row.try_get::<String, _>("created_at")?)?,
            updated_at: parse_dt(&row.try_get::<String, _>("updated_at")?)?,
        }))
    }

    pub async fn upsert_mobile_access_config(
        &self,
        config: MobileAccessConfig,
    ) -> Result<MobileAccessConfig> {
        let created_at = config.created_at.to_rfc3339();
        let updated_at = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO mobile_access_config
                (id, profile_id, tunnel_id, public_base_url, relay_base_url, tunnel_secret, daemon_public_key, daemon_private_key, enabled, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                    profile_id=excluded.profile_id,
                    tunnel_id=excluded.tunnel_id,
                    public_base_url=excluded.public_base_url,
                    relay_base_url=excluded.relay_base_url,
                    tunnel_secret=excluded.tunnel_secret,
                    daemon_public_key=excluded.daemon_public_key,
                    daemon_private_key=excluded.daemon_private_key,
                    enabled=excluded.enabled,
                    updated_at=excluded.updated_at"#,
        )
        .bind(config.id)
        .bind(config.profile_id.0.to_string())
        .bind(config.tunnel_id)
        .bind(config.public_base_url)
        .bind(config.relay_base_url)
        .bind(config.tunnel_secret)
        .bind(config.daemon_public_key)
        .bind(config.daemon_private_key)
        .bind(if config.enabled { 1 } else { 0 })
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        self.get_mobile_access_config()
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back mobile access config"))
    }

    pub async fn set_mobile_access_enabled(&self, enabled: bool) -> Result<()> {
        sqlx::query(r#"UPDATE mobile_access_config SET enabled = ?, updated_at = ? WHERE id = ?"#)
            .bind(if enabled { 1 } else { 0 })
            .bind(Utc::now().to_rfc3339())
            .bind("default")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_mobile_pairing_token(
        &self,
        token_id: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO mobile_pairing_tokens
                (id, token_hash, created_at, expires_at)
               VALUES (?, ?, ?, ?)"#,
        )
        .bind(token_id)
        .bind(token_hash)
        .bind(Utc::now().to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_mobile_pairing_token(&self, token_hash: &str) -> Result<bool> {
        let row =
            sqlx::query(r#"SELECT id, expires_at FROM mobile_pairing_tokens WHERE token_hash = ?"#)
                .bind(token_hash)
                .fetch_optional(&self.pool)
                .await?;

        let Some(row) = row else {
            return Ok(false);
        };
        let expires_at: String = row.try_get("expires_at")?;
        let expires_at = parse_dt(&expires_at)?;
        if expires_at < Utc::now() {
            let _ = sqlx::query(r#"DELETE FROM mobile_pairing_tokens WHERE token_hash = ?"#)
                .bind(token_hash)
                .execute(&self.pool)
                .await;
            return Ok(false);
        }

        sqlx::query(r#"DELETE FROM mobile_pairing_tokens WHERE token_hash = ?"#)
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(true)
    }

    pub async fn update_mobile_device_seq(
        &self,
        id: MobileDeviceId,
        seq: i64,
    ) -> Result<Option<i64>> {
        let row = sqlx::query(r#"SELECT last_seen_seq FROM mobile_devices WHERE id = ?"#)
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        let last_seen: Option<i64> = row.and_then(|r| r.try_get("last_seen_seq").ok());
        sqlx::query(
            r#"UPDATE mobile_devices
               SET last_seen_seq = ?, last_seen_at = ?
               WHERE id = ?"#,
        )
        .bind(seq)
        .bind(Utc::now().to_rfc3339())
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(last_seen)
    }

    pub async fn upsert_mobile_device(
        &self,
        id: MobileDeviceId,
        profile_id: ConnectionProfileId,
        update: MobileDeviceUpsert,
    ) -> Result<MobileDeviceRegistration> {
        let MobileDeviceUpsert {
            device_label,
            platform,
            push_token,
            push_provider,
            public_key,
            app_version,
        } = update;
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO mobile_devices
                (id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    device_label=excluded.device_label,
                    platform=excluded.platform,
                    push_token=excluded.push_token,
                    push_provider=excluded.push_provider,
                    public_key=excluded.public_key,
                    app_version=excluded.app_version,
                    last_seen_at=excluded.last_seen_at"#,
        )
        .bind(id.0.to_string())
        .bind(profile_id.0.to_string())
        .bind(device_label.clone())
        .bind(platform.clone())
        .bind(push_token.clone())
        .bind(push_provider.clone())
        .bind(public_key.clone())
        .bind(app_version.clone())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.get_mobile_device(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back mobile device {}", id.0))
    }

    pub async fn get_mobile_device(
        &self,
        id: MobileDeviceId,
    ) -> Result<Option<MobileDeviceRegistration>> {
        let row = sqlx::query(
            r#"SELECT id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at
               FROM mobile_devices WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(build_mobile_device_from_row).transpose()
    }

    pub async fn list_mobile_devices(
        &self,
        profile_id: ConnectionProfileId,
    ) -> Result<Vec<MobileDeviceRegistration>> {
        let rows = sqlx::query(
            r#"SELECT id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at
               FROM mobile_devices WHERE profile_id = ? ORDER BY created_at DESC"#,
        )
        .bind(profile_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(build_mobile_device_from_row(row)?);
        }
        Ok(out)
    }
}

fn build_mobile_connection_profile_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<MobileConnectionProfile> {
    let id: String = row.try_get("id")?;
    let scopes_json: String = row.try_get("scopes_json")?;
    let created_at: String = row.try_get("created_at")?;
    let last_used_at: Option<String> = row.try_get("last_used_at")?;
    let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
    Ok(MobileConnectionProfile {
        id: ConnectionProfileId(uuid::Uuid::parse_str(&id)?),
        label: row.try_get("label")?,
        base_url: row.try_get("base_url")?,
        token_prefix: row.try_get("token_prefix")?,
        scopes,
        created_at: parse_dt(&created_at)?,
        last_used_at: last_used_at.as_deref().map(parse_dt).transpose()?,
    })
}

fn build_mobile_device_from_row(row: sqlx::sqlite::SqliteRow) -> Result<MobileDeviceRegistration> {
    let id: String = row.try_get("id")?;
    let profile_id: String = row.try_get("profile_id")?;
    let created_at: String = row.try_get("created_at")?;
    let last_seen_at: String = row.try_get("last_seen_at")?;
    Ok(MobileDeviceRegistration {
        id: MobileDeviceId(uuid::Uuid::parse_str(&id)?),
        profile_id: ConnectionProfileId(uuid::Uuid::parse_str(&profile_id)?),
        device_label: row.try_get("device_label")?,
        platform: row.try_get("platform")?,
        push_token: row.try_get("push_token")?,
        push_provider: row.try_get("push_provider")?,
        public_key: row.try_get("public_key")?,
        app_version: row.try_get("app_version")?,
        created_at: parse_dt(&created_at)?,
        last_seen_at: parse_dt(&last_seen_at)?,
    })
}

fn map_merge_queue_entry(row: sqlx::sqlite::SqliteRow) -> Option<MergeQueueEntry> {
    let id: String = row.try_get("id").ok()?;
    let workspace_id: String = row.try_get("workspace_id").ok()?;
    let worktree_id: Option<String> = row.try_get("worktree_id").ok()?;
    let session_id: Option<String> = row.try_get("session_id").ok()?;
    let target_branch: String = row.try_get("target_branch").ok()?;
    let patch_source: String = row.try_get("patch_source").ok()?;
    let created_at: String = row.try_get("created_at").ok()?;
    let updated_at: String = row.try_get("updated_at").ok()?;
    let status: String = row.try_get("status").ok()?;
    Some(MergeQueueEntry {
        id: MergeQueueEntryId(uuid::Uuid::parse_str(&id).ok()?),
        workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id).ok()?),
        worktree_id: worktree_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(WorktreeId),
        session_id: parse_optional_session_id(session_id),
        target_branch,
        message: row.try_get("message").ok(),
        patch_source: parse_merge_queue_patch_source(&patch_source),
        base_commit_sha: row.try_get("base_commit_sha").ok(),
        head_commit_sha: row.try_get("head_commit_sha").ok(),
        patch_path: row.try_get("patch_path").ok()?,
        patch_size: row.try_get("patch_size").ok()?,
        status: parse_merge_queue_entry_status(&status),
        result_commit_sha: row.try_get("result_commit_sha").ok(),
        error_message: row.try_get("error_message").ok(),
        created_at: parse_dt(&created_at).ok()?,
        updated_at: parse_dt(&updated_at).ok()?,
    })
}

fn map_merge_queue_run(row: sqlx::sqlite::SqliteRow) -> Option<MergeQueueRun> {
    let id: String = row.try_get("id").ok()?;
    let entry_id: String = row.try_get("entry_id").ok()?;
    let status: String = row.try_get("status").ok()?;
    let started_at: String = row.try_get("started_at").ok()?;
    let finished_at: Option<String> = row.try_get("finished_at").ok()?;
    Some(MergeQueueRun {
        id: MergeQueueRunId(uuid::Uuid::parse_str(&id).ok()?),
        entry_id: MergeQueueEntryId(uuid::Uuid::parse_str(&entry_id).ok()?),
        status: parse_merge_queue_run_status(&status),
        started_at: parse_dt(&started_at).ok()?,
        finished_at: finished_at.as_deref().and_then(|v| parse_dt(v).ok()),
        exit_code: row.try_get("exit_code").ok(),
        log_path: row.try_get("log_path").ok(),
        error_message: row.try_get("error_message").ok(),
        result_commit_sha: row.try_get("result_commit_sha").ok(),
    })
}

struct SessionSnapshotRow {
    session: Session,
    last_message_at: Option<DateTime<Utc>>,
    last_message_preview: Option<String>,
    last_event_seq: Option<i64>,
    activity: SessionActivityState,
}

fn session_metadata_from_session(session: &Session) -> SessionMetadata {
    SessionMetadata {
        id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        parent_session_id: session.parent_session_id,
        relationship: session.relationship.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        title: session.title.clone(),
        agent_role: session.agent_role.clone(),
        status: session.status.clone(),
        provider_session_ref: session.provider_session_ref.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

fn session_head_to_snapshot(head: SessionHead) -> SessionHeadSnapshot {
    SessionHeadSnapshot {
        session: session_metadata_from_session(&head.session),
        turns: head.turns,
        tool_summaries: head.tool_summaries,
        events: head.events,
        messages: head.messages,
        last_event_seq: head.last_event_seq,
        state_rev: head.last_event_seq,
        activity: head.activity,
        has_more_turns: head.has_more_turns,
        history_cursor: None,
        has_more_history: false,
        summary_checkpoint: head.summary_checkpoint,
        head_window: head.head_window,
    }
}

fn derive_message_preview(content: &str) -> String {
    let trimmed = content.trim();
    let line = trimmed.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return String::new();
    }
    const MAX_CHARS: usize = 160;
    let mut out: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        out.push_str("...");
    }
    out
}

fn derive_activity_from_status(
    last_status: Option<SessionTurnStatus>,
    has_running_turn: bool,
) -> SessionActivityState {
    SessionActivityState {
        is_working: has_running_turn,
        last_turn_status: last_status,
    }
}

fn parse_dt(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn task_status_to_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn parse_task_status(value: &str) -> TaskStatus {
    match value {
        "pending" => TaskStatus::Pending,
        "running" => TaskStatus::Running,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    }
}

fn session_status_to_str(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
        SessionStatus::Cancelled => "cancelled",
    }
}

fn parse_session_status(value: &str) -> SessionStatus {
    match value {
        "active" => SessionStatus::Active,
        "completed" => SessionStatus::Completed,
        "failed" => SessionStatus::Failed,
        "cancelled" => SessionStatus::Cancelled,
        _ => SessionStatus::Active,
    }
}

fn merge_queue_entry_status_to_str(status: &MergeQueueEntryStatus) -> &'static str {
    match status {
        MergeQueueEntryStatus::Queued => "queued",
        MergeQueueEntryStatus::Running => "running",
        MergeQueueEntryStatus::Passed => "passed",
        MergeQueueEntryStatus::Failed => "failed",
        MergeQueueEntryStatus::Conflict => "conflict",
        MergeQueueEntryStatus::Cancelled => "cancelled",
    }
}

fn parse_merge_queue_entry_status(value: &str) -> MergeQueueEntryStatus {
    match value {
        "queued" => MergeQueueEntryStatus::Queued,
        "running" => MergeQueueEntryStatus::Running,
        "passed" => MergeQueueEntryStatus::Passed,
        "failed" => MergeQueueEntryStatus::Failed,
        "conflict" => MergeQueueEntryStatus::Conflict,
        "cancelled" => MergeQueueEntryStatus::Cancelled,
        _ => MergeQueueEntryStatus::Queued,
    }
}

fn merge_queue_run_status_to_str(status: &MergeQueueRunStatus) -> &'static str {
    match status {
        MergeQueueRunStatus::Running => "running",
        MergeQueueRunStatus::Passed => "passed",
        MergeQueueRunStatus::Failed => "failed",
        MergeQueueRunStatus::Conflict => "conflict",
        MergeQueueRunStatus::Cancelled => "cancelled",
    }
}

fn parse_merge_queue_run_status(value: &str) -> MergeQueueRunStatus {
    match value {
        "running" => MergeQueueRunStatus::Running,
        "passed" => MergeQueueRunStatus::Passed,
        "failed" => MergeQueueRunStatus::Failed,
        "conflict" => MergeQueueRunStatus::Conflict,
        "cancelled" => MergeQueueRunStatus::Cancelled,
        _ => MergeQueueRunStatus::Running,
    }
}

fn merge_queue_patch_source_to_str(source: &MergeQueuePatchSource) -> &'static str {
    match source {
        MergeQueuePatchSource::Generated => "generated",
        MergeQueuePatchSource::Provided => "provided",
    }
}

fn parse_merge_queue_patch_source(value: &str) -> MergeQueuePatchSource {
    match value {
        "provided" => MergeQueuePatchSource::Provided,
        "generated" => MergeQueuePatchSource::Generated,
        _ => MergeQueuePatchSource::Generated,
    }
}

fn message_role_to_str(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    }
}

fn parse_message_role(value: &str) -> MessageRole {
    match value {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        _ => MessageRole::User,
    }
}

fn attachment_kind_to_str(kind: &WorkspaceAttachmentKind) -> &'static str {
    match kind {
        WorkspaceAttachmentKind::ReferenceRepo => "reference_repo",
        WorkspaceAttachmentKind::DocMirror => "doc_mirror",
    }
}

fn parse_attachment_kind(value: &str) -> WorkspaceAttachmentKind {
    match value {
        "reference_repo" => WorkspaceAttachmentKind::ReferenceRepo,
        "doc_mirror" => WorkspaceAttachmentKind::DocMirror,
        _ => WorkspaceAttachmentKind::ReferenceRepo,
    }
}

fn attachment_mode_to_str(mode: &AttachmentMode) -> &'static str {
    match mode {
        AttachmentMode::Ro => "ro",
        AttachmentMode::Rw => "rw",
    }
}

fn parse_attachment_mode(value: &str) -> AttachmentMode {
    match value {
        "rw" => AttachmentMode::Rw,
        "ro" => AttachmentMode::Ro,
        _ => AttachmentMode::Ro,
    }
}

fn attachment_update_policy_to_str(policy: &AttachmentUpdatePolicy) -> &'static str {
    match policy {
        AttachmentUpdatePolicy::Manual => "manual",
        AttachmentUpdatePolicy::OnOpen => "on_open",
        AttachmentUpdatePolicy::Scheduled => "scheduled",
    }
}

fn parse_attachment_update_policy(value: &str) -> AttachmentUpdatePolicy {
    match value {
        "on_open" => AttachmentUpdatePolicy::OnOpen,
        "scheduled" => AttachmentUpdatePolicy::Scheduled,
        "manual" => AttachmentUpdatePolicy::Manual,
        _ => AttachmentUpdatePolicy::Manual,
    }
}

fn worktree_attachment_status_to_str(status: &WorktreeAttachmentStatus) -> &'static str {
    match status {
        WorktreeAttachmentStatus::Ready => "ready",
        WorktreeAttachmentStatus::Stale => "stale",
        WorktreeAttachmentStatus::Error => "error",
    }
}

fn parse_worktree_attachment_status(value: &str) -> WorktreeAttachmentStatus {
    match value {
        "ready" => WorktreeAttachmentStatus::Ready,
        "stale" => WorktreeAttachmentStatus::Stale,
        "error" => WorktreeAttachmentStatus::Error,
        _ => WorktreeAttachmentStatus::Error,
    }
}

fn message_delivery_to_str(delivery: &MessageDelivery) -> &'static str {
    match delivery {
        MessageDelivery::Immediate => "immediate",
        MessageDelivery::Queued => "queued",
    }
}

fn parse_message_delivery(value: &str) -> MessageDelivery {
    match value {
        "immediate" => MessageDelivery::Immediate,
        "queued" => MessageDelivery::Queued,
        _ => MessageDelivery::Queued,
    }
}

fn session_turn_status_to_str(status: &SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Interrupted => "interrupted",
        SessionTurnStatus::Failed => "failed",
    }
}

fn parse_session_turn_status(value: &str) -> SessionTurnStatus {
    match value {
        "queued" => SessionTurnStatus::Queued,
        "running" => SessionTurnStatus::Running,
        "completed" => SessionTurnStatus::Completed,
        "interrupted" => SessionTurnStatus::Interrupted,
        "failed" => SessionTurnStatus::Failed,
        _ => SessionTurnStatus::Running,
    }
}

fn session_event_type_to_str(event_type: &SessionEventType) -> &'static str {
    match event_type {
        SessionEventType::Init => "init",
        SessionEventType::UserMessage => "user_message",
        SessionEventType::InputQueued => "input_queued",
        SessionEventType::AuthRequired => "auth_required",
        SessionEventType::Notice => "notice",
        SessionEventType::AssistantChunk => "assistant_chunk",
        SessionEventType::ThoughtChunk => "thought_chunk",
        SessionEventType::AssistantComplete => "assistant_complete",
        SessionEventType::AssistantMessageInserted => "assistant_message_inserted",
        SessionEventType::ToolCall => "tool_call",
        SessionEventType::ToolCallUpdate => "tool_call_update",
        SessionEventType::ToolResult => "tool_result",
        SessionEventType::Plan => "plan",
        SessionEventType::ArtifactsSet => "artifacts_set",
        SessionEventType::Done => "done",
        SessionEventType::InterruptRequested => "interrupt_requested",
        SessionEventType::TurnInterrupted => "turn_interrupted",
        SessionEventType::Error => "error",
    }
}

fn parse_session_event_type(value: &str) -> SessionEventType {
    match value {
        "init" => SessionEventType::Init,
        "user_message" => SessionEventType::UserMessage,
        "input_queued" => SessionEventType::InputQueued,
        "auth_required" => SessionEventType::AuthRequired,
        "notice" => SessionEventType::Notice,
        "assistant_chunk" => SessionEventType::AssistantChunk,
        "thought_chunk" => SessionEventType::ThoughtChunk,
        "assistant_complete" => SessionEventType::AssistantComplete,
        "assistant_message_inserted" => SessionEventType::AssistantMessageInserted,
        "tool_call" => SessionEventType::ToolCall,
        "tool_call_update" => SessionEventType::ToolCallUpdate,
        "tool_result" => SessionEventType::ToolResult,
        "plan" => SessionEventType::Plan,
        "artifacts_set" => SessionEventType::ArtifactsSet,
        "done" => SessionEventType::Done,
        "interrupt_requested" => SessionEventType::InterruptRequested,
        "turn_interrupted" => SessionEventType::TurnInterrupted,
        "error" => SessionEventType::Error,
        _ => SessionEventType::Error,
    }
}

fn is_transient_session_event(
    event_type: &SessionEventType,
    payload_json: &serde_json::Value,
) -> bool {
    if matches!(event_type, SessionEventType::AuthRequired) {
        return true;
    }
    if !matches!(event_type, SessionEventType::Notice) {
        return false;
    }

    if let Some(kind) = payload_json.get("kind").and_then(|v| v.as_str()) {
        if matches!(
            kind,
            "provider_guard_warning"
                | "provider_guard_kill"
                | "title_generated"
                | "git_status_snapshot"
                | "auth_started"
                | "auth_finished"
                | "auth_failed"
                | "auth_required"
        ) {
            return true;
        }
    }

    let update = payload_json
        .get("acp_update")
        .or_else(|| payload_json.get("acpUpdate"));
    if let Some(update) = update {
        if let Some(session_update) = update
            .get("sessionUpdate")
            .or_else(|| update.get("session_update"))
            .and_then(|v| v.as_str())
        {
            return session_update == "available_commands_update";
        }
    }

    false
}

fn build_subagent_invocation_from_row(r: sqlx::sqlite::SqliteRow) -> Result<SubagentInvocation> {
    let id: String = r.try_get("id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let parent_session_id: String = r.try_get("parent_session_id")?;
    let parent_turn_id: Option<String> = r.try_get("parent_turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let request_json = r
        .try_get::<Option<String>, _>("request_json")?
        .and_then(|raw| serde_json::from_str(&raw).ok());

    Ok(SubagentInvocation {
        id,
        tool_call_id,
        parent_session_id: SessionId(uuid::Uuid::parse_str(&parent_session_id)?),
        parent_turn_id: parent_turn_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(TurnId),
        requested_count: r.try_get("requested_count")?,
        request_json,
        status: r.try_get("status")?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
        children: Vec::new(),
    })
}

fn build_subagent_invocation_child_from_row(
    r: sqlx::sqlite::SqliteRow,
) -> Result<SubagentInvocationChild> {
    let invocation_id: String = r.try_get("invocation_id")?;
    let child_session_id: String = r.try_get("child_session_id")?;
    let run_id: Option<String> = r.try_get("run_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;

    Ok(SubagentInvocationChild {
        invocation_id,
        child_session_id: SessionId(uuid::Uuid::parse_str(&child_session_id)?),
        run_id: run_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(RunId),
        position: r.try_get("position")?,
        status: r.try_get("status")?,
        label: r.try_get("label")?,
        harness: r.try_get("harness")?,
        model: r.try_get("model")?,
        reasoning_effort: r.try_get("reasoning_effort")?,
        prompt_length: r.try_get("prompt_length")?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn build_artifact_from_row(r: sqlx::sqlite::SqliteRow) -> Result<Artifact> {
    let id: String = r.try_get("id")?;
    let session_id: String = r.try_get("session_id")?;
    let task_id: String = r.try_get("task_id")?;
    let workspace_id: String = r.try_get("workspace_id")?;
    let worktree_id: String = r.try_get("worktree_id")?;
    let created_at: String = r.try_get("created_at")?;

    Ok(Artifact {
        id: ArtifactId(uuid::Uuid::parse_str(&id)?),
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
        workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id)?),
        worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
        name: r.try_get("name")?,
        absolute_path: r.try_get("absolute_path")?,
        mime_type: r.try_get("mime_type")?,
        bytes: r.try_get("bytes")?,
        created_at: parse_dt(&created_at)?,
        missing: None,
    })
}

fn build_session_turn_from_row(r: sqlx::sqlite::SqliteRow) -> Result<SessionTurn> {
    let turn_id: String = r.try_get("turn_id")?;
    let session_id: String = r.try_get("session_id")?;
    let run_id: Option<String> = r.try_get("run_id")?;
    let user_message_id: Option<String> = r.try_get("user_message_id")?;
    let status: String = r.try_get("status")?;
    let started_at: String = r.try_get("started_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let metrics_json: Option<String> = r.try_get("metrics_json")?;
    let metrics_json = metrics_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    Ok(SessionTurn {
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        run_id: run_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(RunId),
        user_message_id: user_message_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(MessageId),
        status: parse_session_turn_status(status.as_str()),
        start_seq: r.try_get("start_seq")?,
        end_seq: r.try_get("end_seq")?,
        started_at: parse_dt(&started_at)?,
        updated_at: parse_dt(&updated_at)?,
        assistant_partial: r.try_get("assistant_partial")?,
        thought_partial: r.try_get("thought_partial")?,
        metrics_json,
        tool_total: r.try_get("tool_total")?,
        tool_pending: r.try_get("tool_pending")?,
        tool_running: r.try_get("tool_running")?,
        tool_completed: r.try_get("tool_completed")?,
        tool_failed: r.try_get("tool_failed")?,
    })
}

fn build_session_turn_tool_from_row(r: sqlx::sqlite::SqliteRow) -> Result<SessionTurnTool> {
    let session_id: String = r.try_get("session_id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let turn_id: String = r.try_get("turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let input_json: Option<String> = r.try_get("input_json")?;
    let input_json = input_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    Ok(SessionTurnTool {
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        tool_call_id,
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        tool_kind: r.try_get("tool_kind")?,
        title: r.try_get("title")?,
        status: r.try_get("status")?,
        input_json,
        output_text: r.try_get("output_text")?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn build_session_turn_tool_summary_from_row(
    r: sqlx::sqlite::SqliteRow,
) -> Result<SessionTurnToolSummary> {
    let session_id: String = r.try_get("session_id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let turn_id: String = r.try_get("turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let input_json: Option<String> = r.try_get("input_json")?;
    let input_json = input_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let input_preview = tool_input_preview_from_value(input_json.as_ref());

    Ok(SessionTurnToolSummary {
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        tool_call_id,
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        tool_kind: r.try_get("tool_kind")?,
        title: r.try_get("title")?,
        status: r.try_get("status")?,
        input_preview,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn summarize_session_turn_tool(tool: &SessionTurnTool) -> SessionTurnToolSummary {
    SessionTurnToolSummary {
        session_id: tool.session_id,
        tool_call_id: tool.tool_call_id.clone(),
        turn_id: tool.turn_id,
        tool_kind: tool.tool_kind.clone(),
        title: tool.title.clone(),
        status: tool.status.clone(),
        input_preview: tool_input_preview_from_value(tool.input_json.as_ref()),
        created_at: tool.created_at,
        updated_at: tool.updated_at,
    }
}

fn tool_input_preview_from_value(input: Option<&Value>) -> Option<Value> {
    let input = input?;
    let obj = input.as_object()?;
    let mut out = serde_json::Map::new();
    for key in [
        "command",
        "query",
        "pattern",
        "text",
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "paths",
        "paths_total",
        "files",
        "file_paths",
        "filePaths",
        "target",
        "glob",
        "parsed_cmd",
        "cwd",
        "root",
        "url",
        "uri",
        "href",
        "method",
        "regex",
        "diff_stats",
    ] {
        if let Some(value) = obj.get(key) {
            if value.is_string() || value.is_number() || value.is_array() || value.is_object() {
                out.insert(key.to_string(), value.clone());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn normalize_tool_status(status: &str, event_type: SessionEventType) -> String {
    let s = status.trim().to_lowercase();
    if s == "inprogress" || s == "in_progress" || s == "running" {
        return "in_progress".to_string();
    }
    if s == "pending" || s == "queued" {
        return "pending".to_string();
    }
    if s == "completed" || s == "complete" || s == "ok" || s == "succeeded" {
        return "completed".to_string();
    }
    if s == "failed" || s == "error" {
        return "failed".to_string();
    }
    if matches!(event_type, SessionEventType::ToolResult) {
        return "completed".to_string();
    }
    if s.is_empty() {
        return "pending".to_string();
    }
    s
}

fn extract_tool_update(payload: &Value) -> &Value {
    payload.get("acp_update").unwrap_or(payload)
}

fn build_turn_tool_from_event(event: &SessionEvent, turn_id: TurnId) -> Option<SessionTurnTool> {
    if !matches!(
        event.event_type,
        SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
    ) {
        return None;
    }
    let tool_call_id = tool_call_id_from_payload(&event.payload_json)?;
    let update = extract_tool_update(&event.payload_json);

    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|v| v.to_string());

    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|v| v.to_string());

    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw_status) = raw_status {
        Some(normalize_tool_status(raw_status, event.event_type.clone()))
    } else if matches!(event.event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else if matches!(event.event_type, SessionEventType::ToolCall) {
        Some("pending".to_string())
    } else {
        None
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_json = input
        .cloned()
        .or_else(|| update.get("input_preview").cloned());

    let output_text = extract_tool_output_text(update);

    Some(SessionTurnTool {
        session_id: event.session_id,
        tool_call_id,
        turn_id,
        tool_kind,
        title,
        status,
        input_json,
        output_text,
        created_at: event.created_at,
        updated_at: event.created_at,
    })
}

fn tool_call_id_from_payload(payload: &Value) -> Option<String> {
    let direct = payload.get("tool_call_id").and_then(|v| v.as_str());
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let update = extract_tool_update(payload);
    let direct = update
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("tool_call_id").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let from_raw = update
        .pointer("/rawInput/call_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            update
                .pointer("/raw_input/call_id")
                .and_then(|v| v.as_str())
        });
    from_raw.map(|v| v.to_string())
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| update.get("output_preview").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/toolCall/outputText")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            update
                .pointer("/toolCall/output_text")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/rawOutput/aggregated_output")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.pointer("/rawOutput/output").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let blocks = update.get("content").and_then(|v| v.as_array())?;
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
        {
            out.push_str(t);
        } else if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

fn merge_streaming_text(prev: Option<&str>, next: &str) -> String {
    let prev = prev.unwrap_or("");
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.starts_with(next) {
        return prev.to_string();
    }
    if next.len() >= prev.len() {
        next.to_string()
    } else {
        prev.to_string()
    }
}

fn build_turn_tools_from_events(
    session_id: SessionId,
    turn_id: TurnId,
    events: &[SessionEvent],
) -> Vec<SessionTurnTool> {
    #[derive(Default)]
    struct ToolAgg {
        tool_kind: Option<String>,
        title: Option<String>,
        status: Option<String>,
        input_json: Option<Value>,
        output_text: Option<String>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        initialized: bool,
    }

    let mut map: HashMap<String, ToolAgg> = HashMap::new();

    for ev in events {
        if !matches!(
            ev.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            continue;
        }
        let tool_call_id = match tool_call_id_from_payload(&ev.payload_json) {
            Some(v) => v,
            None => continue,
        };
        let update = extract_tool_update(&ev.payload_json);
        let entry = map.entry(tool_call_id.clone()).or_default();
        if !entry.initialized {
            entry.created_at = ev.created_at;
            entry.updated_at = ev.created_at;
            entry.initialized = true;
        }
        entry.updated_at = ev.created_at;

        if let Some(kind) = update
            .get("kind")
            .and_then(|v| v.as_str())
            .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        {
            entry.tool_kind = Some(kind.to_string());
        }
        if let Some(title) = update
            .get("title")
            .and_then(|v| v.as_str())
            .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
            .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        {
            entry.title = Some(title.to_string());
        }

        let raw_status = update
            .get("status")
            .and_then(|v| v.as_str())
            .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
        if let Some(raw_status) = raw_status {
            entry.status = Some(normalize_tool_status(raw_status, ev.event_type.clone()));
        } else if matches!(ev.event_type, SessionEventType::ToolResult) {
            entry.status = Some("completed".to_string());
        } else if matches!(ev.event_type, SessionEventType::ToolCall) {
            entry.status = entry.status.clone().or(Some("pending".to_string()));
        }

        let input = update
            .pointer("/rawInput")
            .or_else(|| update.pointer("/toolCall/rawInput"))
            .or_else(|| update.pointer("/toolCall/input"))
            .or_else(|| update.pointer("/input"))
            .or_else(|| update.pointer("/args"));
        if let Some(value) = input {
            entry.input_json = Some(value.clone());
        } else if let Some(preview) = update.get("input_preview") {
            entry.input_json = Some(preview.clone());
        }

        if let Some(output) = extract_tool_output_text(update) {
            let merged = merge_streaming_text(entry.output_text.as_deref(), &output);
            entry.output_text = Some(merged);
        }
    }

    let mut out: Vec<SessionTurnTool> = map
        .into_iter()
        .map(|(tool_call_id, agg)| SessionTurnTool {
            session_id,
            tool_call_id,
            turn_id,
            tool_kind: agg.tool_kind,
            title: agg.title,
            status: agg.status,
            input_json: agg.input_json,
            output_text: agg.output_text,
            created_at: agg.created_at,
            updated_at: agg.updated_at,
        })
        .collect();

    out.sort_by_key(|t| t.created_at);
    out
}
