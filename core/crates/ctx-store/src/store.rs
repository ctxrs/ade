use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ctx_core::ids::*;
use ctx_core::models::*;
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, Pool, QueryBuilder, Row, Sqlite};

#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
}

pub struct SessionRetentionPruneStats {
    pub tool_summaries_deleted: u64,
    pub turn_thoughts_cleared: u64,
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
    raw.and_then(|value| uuid::Uuid::parse_str(&value).ok()).map(SessionId)
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

    // Task APIs
    pub async fn list_tasks(&self, workspace_id: WorkspaceId) -> Result<Vec<Task>> {
        let rows = sqlx::query(
            r#"
            SELECT
              t.id, t.workspace_id, t.title, t.description, t.status, t.exec_plan_id,
              t.created_at, t.updated_at, t.archived_at, t.assistant_seen_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id
              ) AS last_activity_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id AND m.role = 'assistant'
              ) AS last_assistant_message_at,
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
            archived_at: None,
            assistant_seen_at: None,
            last_activity_at: None,
            last_assistant_message_at: None,
            has_active_session: false,
        };
        sqlx::query(
            r#"INSERT INTO tasks (id, workspace_id, title, description, status, exec_plan_id, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(task.id.0.to_string())
        .bind(task.workspace_id.0.to_string())
        .bind(&task.title)
        .bind(&task.description)
        .bind(task_status_to_str(&task.status))
        .bind(&task.exec_plan_id)
        .bind(task.created_at.to_rfc3339())
        .bind(task.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(task)
    }

    pub async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, title, description, status, exec_plan_id, created_at, updated_at, archived_at, assistant_seen_at
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
            Some(Task {
                id: TaskId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                title: r.try_get("title").ok()?,
                description: r.try_get("description").ok()?,
                status: parse_task_status(r.try_get::<String, _>("status").ok()?.as_str()),
                created_at: parse_dt(&created_at).ok()?,
                updated_at: parse_dt(&updated_at).ok()?,
                exec_plan_id: r.try_get("exec_plan_id").ok()?,
                archived_at: archived_at.as_deref().map(parse_dt).transpose().ok()?,
                assistant_seen_at: assistant_seen_at
                    .as_deref()
                    .map(parse_dt)
                    .transpose()
                    .ok()?,
                last_activity_at: None,
                last_assistant_message_at: None,
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
              t.created_at, t.updated_at, t.archived_at, t.assistant_seen_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id
              ) AS last_activity_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id AND m.role = 'assistant'
              ) AS last_assistant_message_at,
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

    pub async fn list_track_attachment_mounts(
        &self,
        track_id: TrackId,
    ) -> Result<Vec<TrackAttachmentMount>> {
        let rows = sqlx::query(
            r#"SELECT track_id, attachment_id, mount_abs_path, materialized_id, status,
                      last_sync_at, error_message, created_at, updated_at
               FROM track_attachment_mounts
               WHERE track_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(track_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let track_id: String = r.try_get("track_id")?;
            let attachment_id: String = r.try_get("attachment_id")?;
            let status: String = r.try_get("status")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(TrackAttachmentMount {
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
                attachment_id: WorkspaceAttachmentId(uuid::Uuid::parse_str(&attachment_id)?),
                mount_abs_path: r.try_get("mount_abs_path")?,
                materialized_id: r.try_get("materialized_id")?,
                status: parse_track_attachment_status(&status),
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

    pub async fn list_track_attachment_mounts_for_attachment(
        &self,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<Vec<TrackAttachmentMount>> {
        let rows = sqlx::query(
            r#"SELECT track_id, attachment_id, mount_abs_path, materialized_id, status,
                      last_sync_at, error_message, created_at, updated_at
               FROM track_attachment_mounts
               WHERE attachment_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(attachment_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let track_id: String = r.try_get("track_id")?;
            let attachment_id: String = r.try_get("attachment_id")?;
            let status: String = r.try_get("status")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(TrackAttachmentMount {
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
                attachment_id: WorkspaceAttachmentId(uuid::Uuid::parse_str(&attachment_id)?),
                mount_abs_path: r.try_get("mount_abs_path")?,
                materialized_id: r.try_get("materialized_id")?,
                status: parse_track_attachment_status(&status),
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

    pub async fn upsert_track_attachment_mount(&self, mount: &TrackAttachmentMount) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO track_attachment_mounts
               (track_id, attachment_id, mount_abs_path, materialized_id, status, last_sync_at, error_message, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(track_id, attachment_id) DO UPDATE SET
                 mount_abs_path = excluded.mount_abs_path,
                 materialized_id = excluded.materialized_id,
                 status = excluded.status,
                 last_sync_at = excluded.last_sync_at,
                 error_message = excluded.error_message,
                 updated_at = excluded.updated_at"#,
        )
        .bind(mount.track_id.0.to_string())
        .bind(mount.attachment_id.0.to_string())
        .bind(&mount.mount_abs_path)
        .bind(&mount.materialized_id)
        .bind(track_attachment_status_to_str(&mount.status))
        .bind(mount.last_sync_at.map(|v| v.to_rfc3339()))
        .bind(&mount.error_message)
        .bind(mount.created_at.to_rfc3339())
        .bind(mount.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_track_attachment_mounts_for_attachment(
        &self,
        attachment_id: WorkspaceAttachmentId,
    ) -> Result<()> {
        sqlx::query(r#"DELETE FROM track_attachment_mounts WHERE attachment_id = ?"#)
            .bind(attachment_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Track APIs
    pub async fn create_track(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        label: String,
    ) -> Result<Track> {
        let now = Utc::now();
        let track = Track {
            id: TrackId::new(),
            task_id,
            workspace_id,
            worktree_id,
            label,
            status: TrackStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"INSERT INTO tracks (id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(track.id.0.to_string())
        .bind(track.task_id.0.to_string())
        .bind(track.workspace_id.0.to_string())
        .bind(track.worktree_id.0.to_string())
        .bind(&track.label)
        .bind(track_status_to_str(&track.status))
        .bind(track.created_at.to_rfc3339())
        .bind(track.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(track)
    }

    pub async fn list_tracks_for_task(&self, task_id: TaskId) -> Result<Vec<Track>> {
        let rows = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at
               FROM tracks WHERE task_id = ? ORDER BY created_at ASC"#,
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
            out.push(Track {
                id: TrackId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                label: r.try_get("label")?,
                status: parse_track_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_tracks_for_workspace(&self, workspace_id: WorkspaceId) -> Result<Vec<Track>> {
        let rows = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at
               FROM tracks
               WHERE workspace_id = ?
               ORDER BY created_at ASC"#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let worktree_id: String = r.try_get("worktree_id")?;
            let status: String = r.try_get("status")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(Track {
                id: TrackId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
                label: r.try_get("label")?,
                status: parse_track_status(&status),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub async fn get_track(&self, id: TrackId) -> Result<Option<Track>> {
        let row = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at
               FROM tracks WHERE id = ?"#,
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
            Some(Track {
                id: TrackId(uuid::Uuid::parse_str(&id).ok()?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id).ok()?),
                label: r.try_get("label").ok()?,
                status: parse_track_status(r.try_get::<String, _>("status").ok()?.as_str()),
                created_at: parse_dt(&created_at).ok()?,
                updated_at: parse_dt(&updated_at).ok()?,
            })
        }))
    }

    // Session APIs
    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &self,
        track: &Track,
        provider_id: String,
        model_id: String,
        agent_role: String,
        parent_session_id: Option<SessionId>,
        relationship: Option<String>,
        provider_session_ref: Option<String>,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: SessionId::new(),
            track_id: track.id,
            task_id: track.task_id,
            workspace_id: track.workspace_id,
            worktree_id: track.worktree_id,
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
            r#"INSERT INTO sessions (id, track_id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, title, agent_role, status, provider_session_ref, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(session.id.0.to_string())
        .bind(session.track_id.0.to_string())
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
        Ok(session)
    }

    pub async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        let row = sqlx::query(
            r#"SELECT id, track_id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let track_id: String = r.try_get("track_id").ok()?;
            let task_id: String = r.try_get("task_id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let wt_id: String = r.try_get("worktree_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            let updated_at: String = r.try_get("updated_at").ok()?;
            Some(Session {
                id: SessionId(uuid::Uuid::parse_str(&id).ok()?),
                track_id: TrackId(uuid::Uuid::parse_str(&track_id).ok()?),
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

    pub async fn list_sessions_for_track(&self, track_id: TrackId) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            r#"SELECT id, track_id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions WHERE track_id = ? ORDER BY created_at ASC"#,
        )
        .bind(track_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let track_id: String = r.try_get("track_id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(Session {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
            r#"SELECT id, track_id, task_id, workspace_id, parent_session_id, relationship,
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
            let track_id: String = r.try_get("track_id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            out.push(SessionSummary {
                id: SessionId(uuid::Uuid::parse_str(&id)?),
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
            r#"INSERT INTO messages (id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(message.id.0.to_string())
        .bind(message.session_id.0.to_string())
        .bind(message.task_id.0.to_string())
        .bind(message.track_id.0.to_string())
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
        Ok(message)
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
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        const ACTIVITY_EXPR: &str = "
            COALESCE(
                (
                    SELECT MAX(m.created_at)
                    FROM messages m
                    WHERE m.task_id = t.id
                ),
                t.updated_at,
                t.created_at
            )
        ";
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
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id AND m.role = 'assistant'
              ) AS last_assistant_message_at,
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

        if !include_archived {
            sql.push_str(" AND t.archived_at IS NULL");
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

    pub async fn list_workspace_catchup_page(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceCatchupCursor>,
        limit: i64,
        archived_only: bool,
    ) -> Result<(
        Vec<WorkspaceCatchupTaskSummary>,
        Option<WorkspaceCatchupCursor>,
    )> {
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        const ACTIVITY_EXPR: &str = "
            COALESCE(
                (
                    SELECT MAX(m.created_at)
                    FROM messages m
                    WHERE m.task_id = t.id
                ),
                t.updated_at,
                t.created_at
            )
        ";
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
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id AND m.role = 'assistant'
              ) AS last_assistant_message_at,
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

        if archived_only {
            sql.push_str(" AND t.archived_at IS NOT NULL");
        } else {
            sql.push_str(" AND t.archived_at IS NULL");
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

        let mut next_cursor: Option<WorkspaceCatchupCursor> = None;
        if task_rows.len() as i64 > limit {
            if let Some((task, sort_at)) = task_rows.pop() {
                next_cursor = Some(WorkspaceCatchupCursor {
                    sort_at,
                    task_id: task.id,
                });
            }
        }

        let summaries = self
            .build_workspace_catchup_task_summaries(task_rows)
            .await?;
        Ok((summaries, next_cursor))
    }

    pub async fn get_workspace_task_summary(
        &self,
        task_id: TaskId,
    ) -> Result<Option<WorkspaceTaskSummary>> {
        const ACTIVITY_EXPR: &str = "
            COALESCE(
                (
                    SELECT MAX(m.created_at)
                    FROM messages m
                    WHERE m.task_id = t.id
                ),
                t.updated_at,
                t.created_at
            )
        ";
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
              t.created_at,
              t.updated_at,
              t.archived_at,
              t.assistant_seen_at,
              (
                SELECT MAX(m.created_at)
                FROM messages m
                WHERE m.task_id = t.id AND m.role = 'assistant'
              ) AS last_assistant_message_at,
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

    pub async fn list_messages_for_session(&self, session_id: SessionId) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
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
            let track_id: String = r.try_get("track_id")?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
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
            let track_id: String = r.try_get("track_id").ok()?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id).ok()?),
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
            "SELECT id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
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
            let track_id: String = r.try_get("track_id")?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
                tracks: Vec::new(),
                sort_at,
            });
        }

        let task_ids: Vec<TaskId> = summaries.iter().map(|s| s.task.id).collect();

        let mut track_query =
            QueryBuilder::new("SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at FROM tracks WHERE task_id IN (");
        let mut first = true;
        for task_id in &task_ids {
            if !first {
                track_query.push(", ");
            }
            first = false;
            track_query.push_bind(task_id.0.to_string());
        }
        track_query.push(") ORDER BY created_at ASC");

        let track_rows = track_query.build().fetch_all(&self.pool).await?;

        let mut track_index: HashMap<TrackId, (usize, usize)> = HashMap::new();

        for r in track_rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let track = Track {
                id: TrackId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                label: r.try_get("label")?,
                status: parse_track_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            };
            if let Some(task_idx) = index_by_task.get(&track.task_id) {
                let idx = *task_idx;
                let track_pos = summaries[idx].tracks.len();
                summaries[idx].tracks.push(TrackSummary {
                    track,
                    sessions: Vec::new(),
                });
                track_index.insert(summaries[idx].tracks[track_pos].track.id, (idx, track_pos));
            }
        }

        if !track_index.is_empty() {
            let mut session_query = QueryBuilder::new(
                "
                SELECT id, track_id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
                       provider_id, model_id, title, status, created_at, updated_at
                FROM (
                    SELECT
                        s.*,
                        ROW_NUMBER() OVER (
                            PARTITION BY s.track_id
                            ORDER BY
                                CASE s.status
                                    WHEN 'active' THEN 0
                                    ELSE 1
                                END,
                                s.updated_at DESC
                        ) AS rn
                    FROM sessions s
                    WHERE s.track_id IN (",
            );
            let mut first = true;
            for track_id in track_index.keys() {
                if !first {
                    session_query.push(", ");
                }
                first = false;
                session_query.push_bind(track_id.0.to_string());
            }
            session_query.push(")) WHERE rn <= ");
            session_query.push_bind(SESSION_LIMIT);
            session_query.push(" ORDER BY track_id, rn");

            let session_rows = session_query.build().fetch_all(&self.pool).await?;

            for r in session_rows {
                let id: String = r.try_get("id")?;
                let track_id: String = r.try_get("track_id")?;
                let task_id: String = r.try_get("task_id")?;
                let ws_id: String = r.try_get("workspace_id")?;
                let created_at: String = r.try_get("created_at")?;
                let updated_at: String = r.try_get("updated_at")?;
                let summary = SessionSummary {
                    id: SessionId(uuid::Uuid::parse_str(&id)?),
                    track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
                if let Some((task_idx, track_pos)) = track_index.get(&summary.track_id) {
                    summaries[*task_idx].tracks[*track_pos]
                        .sessions
                        .push(summary.clone());
                }

                if let Some(task_idx) = index_by_task.get(&summary.task_id) {
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

    pub async fn get_workspace_catchup_task_summary(
        &self,
        task_id: TaskId,
    ) -> Result<Option<WorkspaceCatchupTaskSummary>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, title, description, status, exec_plan_id,
                      created_at, updated_at, archived_at, assistant_seen_at,
                      (
                        SELECT MAX(m.created_at)
                        FROM messages m
                        WHERE m.task_id = t.id AND m.role = 'assistant'
                      ) AS last_assistant_message_at,
                      EXISTS(
                        SELECT 1
                        FROM sessions s
                        WHERE s.task_id = t.id AND s.status = 'active'
                      ) AS has_active_session,
                      COALESCE(
                        (
                          SELECT MAX(m.created_at)
                          FROM messages m
                          WHERE m.task_id = t.id
                        ),
                        t.updated_at,
                        t.created_at
                      ) AS activity_at,
                      COALESCE(t.archived_at, t.created_at) AS sort_at
               FROM tasks t WHERE id = ?"#,
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
                .build_workspace_catchup_task_summaries(vec![(task, sort_at_dt)])
                .await?;
            return Ok(summaries.into_iter().next());
        }
        Ok(None)
    }

    pub async fn get_workspace_catchup_track_summary(
        &self,
        track_id: TrackId,
    ) -> Result<Option<WorkspaceCatchupTrackSummary>> {
        let row = sqlx::query(
            r#"SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at
               FROM tracks WHERE id = ?"#,
        )
        .bind(track_id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let id: String = row.try_get("id")?;
        let task_id: String = row.try_get("task_id")?;
        let ws_id: String = row.try_get("workspace_id")?;
        let wt_id: String = row.try_get("worktree_id")?;
        let created_at: String = row.try_get("created_at")?;
        let updated_at: String = row.try_get("updated_at")?;

        let track = Track {
            id: TrackId(uuid::Uuid::parse_str(&id)?),
            task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
            workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
            worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
            label: row.try_get("label")?,
            status: parse_track_status(row.try_get::<String, _>("status")?.as_str()),
            created_at: parse_dt(&created_at)?,
            updated_at: parse_dt(&updated_at)?,
        };

        let mut summary = WorkspaceCatchupTrackSummary {
            track,
            primary_session_id: None,
            sessions: Vec::new(),
            diff_summary: None,
        };

        let session_rows = self.list_session_catchup_rows(&[track_id]).await?;

        let mut primary: Option<(i32, DateTime<Utc>, SessionId)> = None;
        for row in session_rows {
            if row.session.track_id != track_id {
                continue;
            }
            summary.sessions.push(SessionCatchupSummary {
                session: row.session.clone(),
                last_message_at: row.last_message_at,
                last_message_preview: row.last_message_preview.clone(),
                last_event_seq: row.last_event_seq,
                activity: row.activity.clone(),
                unread: None,
            });

            let rank = if matches!(row.session.status, SessionStatus::Active) {
                0
            } else {
                1
            };
            let candidate = (rank, row.session.updated_at, row.session.id);
            if primary
                .as_ref()
                .map(|(r, t, _)| candidate.0 < *r || (candidate.0 == *r && candidate.1 > *t))
                .unwrap_or(true)
            {
                primary = Some(candidate);
            }
        }

        if let Some((_rank, _updated_at, session_id)) = primary {
            summary.primary_session_id = Some(session_id);
        }

        Ok(Some(summary))
    }

    async fn build_workspace_catchup_task_summaries(
        &self,
        rows: Vec<(Task, DateTime<Utc>)>,
    ) -> Result<Vec<WorkspaceCatchupTaskSummary>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let mut summaries = Vec::with_capacity(rows.len());
        let mut index_by_task = HashMap::new();
        for (idx, (task, sort_at)) in rows.into_iter().enumerate() {
            index_by_task.insert(task.id, idx);
            summaries.push(WorkspaceCatchupTaskSummary {
                task,
                tracks: Vec::new(),
                sort_at,
            });
        }

        let task_ids: Vec<TaskId> = summaries.iter().map(|s| s.task.id).collect();

        let mut track_query =
            QueryBuilder::new("SELECT id, task_id, workspace_id, worktree_id, label, status, created_at, updated_at FROM tracks WHERE task_id IN (");
        let mut first = true;
        for task_id in &task_ids {
            if !first {
                track_query.push(", ");
            }
            first = false;
            track_query.push_bind(task_id.0.to_string());
        }
        track_query.push(") ORDER BY created_at ASC");

        let track_rows = track_query.build().fetch_all(&self.pool).await?;

        let mut track_index: HashMap<TrackId, (usize, usize)> = HashMap::new();

        for r in track_rows {
            let id: String = r.try_get("id")?;
            let task_id: String = r.try_get("task_id")?;
            let ws_id: String = r.try_get("workspace_id")?;
            let wt_id: String = r.try_get("worktree_id")?;
            let created_at: String = r.try_get("created_at")?;
            let updated_at: String = r.try_get("updated_at")?;
            let track = Track {
                id: TrackId(uuid::Uuid::parse_str(&id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                label: r.try_get("label")?,
                status: parse_track_status(r.try_get::<String, _>("status")?.as_str()),
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            };
            if let Some(task_idx) = index_by_task.get(&track.task_id) {
                let idx = *task_idx;
                let track_pos = summaries[idx].tracks.len();
                summaries[idx].tracks.push(WorkspaceCatchupTrackSummary {
                    track,
                    primary_session_id: None,
                    sessions: Vec::new(),
                    diff_summary: None,
                });
                track_index.insert(summaries[idx].tracks[track_pos].track.id, (idx, track_pos));
            }
        }

        if !track_index.is_empty() {
            let session_rows = self
                .list_session_catchup_rows(&track_index.keys().cloned().collect::<Vec<_>>())
                .await?;

            let mut primary_by_track: HashMap<TrackId, (i32, DateTime<Utc>, SessionId)> =
                HashMap::new();

            for row in session_rows {
                let summary = SessionCatchupSummary {
                    session: row.session.clone(),
                    last_message_at: row.last_message_at,
                    last_message_preview: row.last_message_preview.clone(),
                    last_event_seq: row.last_event_seq,
                    activity: row.activity.clone(),
                    unread: None,
                };

                if let Some((task_idx, track_pos)) = track_index.get(&summary.session.track_id) {
                    summaries[*task_idx].tracks[*track_pos]
                        .sessions
                        .push(summary);
                }

                let rank = if matches!(row.session.status, SessionStatus::Active) {
                    0
                } else {
                    1
                };
                let candidate = (rank, row.session.updated_at, row.session.id);
                let replace = primary_by_track
                    .get(&row.session.track_id)
                    .map(|(r, t, _)| candidate.0 < *r || (candidate.0 == *r && candidate.1 > *t))
                    .unwrap_or(true);
                if replace {
                    primary_by_track.insert(row.session.track_id, candidate);
                }
            }

            for (track_id, (_rank, _updated_at, session_id)) in primary_by_track {
                if let Some((task_idx, track_pos)) = track_index.get(&track_id) {
                    summaries[*task_idx].tracks[*track_pos].primary_session_id = Some(session_id);
                }
            }
        }

        Ok(summaries)
    }

    async fn list_session_catchup_rows(
        &self,
        track_ids: &[TrackId],
    ) -> Result<Vec<SessionCatchupRow>> {
        if track_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut session_query = QueryBuilder::new(
            r#"
            WITH last_assistant_messages AS (
                SELECT session_id, content, created_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY session_id
                           ORDER BY created_at DESC, id DESC
                       ) AS rn
                FROM messages
                WHERE role = 'assistant'
            ),
            last_events AS (
                SELECT session_id, MAX(seq) AS last_event_seq
                FROM session_events
                GROUP BY session_id
            ),
            last_turns AS (
                SELECT session_id, turn_id, status, started_at, updated_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY session_id
                           ORDER BY COALESCE(start_seq, -1) DESC, started_at DESC, turn_id DESC
                       ) AS rn
                FROM session_turns
            ),
            running_turns AS (
                SELECT session_id, COUNT(*) AS running_count
                FROM session_turns
                WHERE status = 'running'
                GROUP BY session_id
            )
            SELECT
                s.id,
                s.track_id,
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
                lm.content AS last_message_content,
                lm.created_at AS last_message_at,
                le.last_event_seq AS last_event_seq,
                lt.status AS last_turn_status,
                COALESCE(rt.running_count, 0) AS running_turn_count
            FROM sessions s
            LEFT JOIN last_assistant_messages lm
              ON lm.session_id = s.id AND lm.rn = 1
            LEFT JOIN last_events le
              ON le.session_id = s.id
            LEFT JOIN last_turns lt
              ON lt.session_id = s.id AND lt.rn = 1
            LEFT JOIN running_turns rt
              ON rt.session_id = s.id
            WHERE s.track_id IN ("#,
        );

        let mut first = true;
        for track_id in track_ids {
            if !first {
                session_query.push(", ");
            }
            first = false;
            session_query.push_bind(track_id.0.to_string());
        }
        session_query.push(") ORDER BY s.created_at ASC");

        let session_rows = session_query.build().fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(session_rows.len());

        for r in session_rows {
            let id: String = r.try_get("id")?;
            let track_id: String = r.try_get("track_id")?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&wt_id)?),
                parent_session_id: r
                    .try_get::<Option<String>, _>("parent_session_id")?
                    .and_then(|value| uuid::Uuid::parse_str(&value).ok())
                    .map(SessionId),
                relationship: r.try_get("relationship")?,
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

            let row = SessionCatchupRow {
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

    pub async fn list_queued_messages_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
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
            let track_id: String = r.try_get("track_id")?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, turn_sequence, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let session_id: String = r.try_get("session_id").ok()?;
            let task_id: String = r.try_get("task_id").ok()?;
            let track_id: String = r.try_get("track_id").ok()?;
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
                track_id: TrackId(uuid::Uuid::parse_str(&track_id).ok()?),
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

    pub async fn get_session_head(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Result<Option<SessionHead>> {
        const EVENT_HEAD_LIMIT: u32 = 200;
        let session = match self.get_session(session_id).await? {
            Some(session) => session,
            None => return Ok(None),
        };
        let limit = limit.clamp(1, 200) as i64;
        let rows = sqlx::query(
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
        let messages = self.list_messages_for_turns(session_id, &turn_ids).await?;
        let mut tool_summaries = self
            .list_turn_tool_summaries_for_turns(session_id, &turn_ids)
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
                let tools = self.list_turn_tools(session_id, turn.turn_id).await?;
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
        let last_event_seq = self.session_last_event_seq(session_id).await?;
        let last_status = out.last().map(|t| t.status.clone());
        let has_running_turn = out
            .iter()
            .any(|turn| matches!(turn.status, SessionTurnStatus::Running));
        let activity = derive_activity_from_status(last_status, has_running_turn);
        let events = if include_events {
            let mut events = self
                .list_session_events_tail_by_seq(session_id, EVENT_HEAD_LIMIT)
                .await?;
            events.sort_by(|a, b| a.seq.cmp(&b.seq));
            events
        } else {
            Vec::new()
        };

        Ok(Some(SessionHead {
            session,
            turns: out,
            tool_summaries,
            events,
            messages,
            last_event_seq,
            activity,
            has_more_turns,
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
            .list_session_events_for_turn(session_id, turn_id)
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
            r#"SELECT id, session_id, track_id, task_id, workspace_id, worktree_id,
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

    pub async fn get_artifact(&self, id: ArtifactId) -> Result<Option<Artifact>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, track_id, task_id, workspace_id, worktree_id,
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
                        id, session_id, track_id, task_id, workspace_id, worktree_id,
                        position, name, absolute_path, mime_type, bytes, created_at
                   )
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(artifact.id.0.to_string())
            .bind(artifact.session_id.0.to_string())
            .bind(artifact.track_id.0.to_string())
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
        let mut event = SessionEvent {
            seq: 0,
            id: SessionEventId::new(),
            session_id,
            run_id,
            turn_id,
            event_type,
            payload_json: payload_json.clone(),
            created_at: Utc::now(),
        };
        let seq: i64 = sqlx::query_scalar(
            r#"INSERT INTO session_events (id, session_id, run_id, turn_id, event_type, payload_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               RETURNING seq"#,
        )
        .bind(event.id.0.to_string())
        .bind(event.session_id.0.to_string())
        .bind(event.run_id.map(|r| r.0.to_string()))
        .bind(event.turn_id.map(|t| t.0.to_string()))
        .bind(session_event_type_to_str(&event.event_type))
        .bind(payload_json.to_string())
        .bind(event.created_at.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        event.seq = seq;
        if let Some(turn_id) = event.turn_id {
            if let Some(tool) = build_turn_tool_from_event(&event, turn_id) {
                let _ = self.upsert_session_turn_tool(tool).await;
            }
        }
        Ok(event)
    }

    pub async fn list_session_events(&self, session_id: SessionId) -> Result<Vec<SessionEvent>> {
        self.list_session_events_page_by_seq(session_id, None, None)
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
    ) -> Result<Vec<SessionEvent>> {
        crate::fault_injection::maybe_fail("ctx_store.list_session_events_page_by_seq")?;
        let session_id_str = session_id.0.to_string();
        let limit_i64 = limit.map(|n| n as i64);
        let rows = if let Some(after_seq) = after_seq {
            if let Some(limit) = limit_i64 {
                sqlx::query(
                    r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
                       FROM session_events
                       WHERE session_id = ?
                         AND seq > ?
                       ORDER BY seq ASC
                       LIMIT ?"#,
                )
                .bind(&session_id_str)
                .bind(after_seq)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query(
                    r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
                       FROM session_events
                       WHERE session_id = ?
                         AND seq > ?
                       ORDER BY seq ASC"#,
                )
                .bind(&session_id_str)
                .bind(after_seq)
                .fetch_all(&self.pool)
                .await?
            }
        } else if let Some(limit) = limit_i64 {
            sqlx::query(
                r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
                   FROM session_events
                   WHERE session_id = ?
                   ORDER BY seq ASC
                   LIMIT ?"#,
            )
            .bind(&session_id_str)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
                   FROM session_events
                   WHERE session_id = ?
                   ORDER BY seq ASC"#,
            )
            .bind(&session_id_str)
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
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
    }

    pub async fn list_session_events_for_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<Vec<SessionEvent>> {
        let rows = sqlx::query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
               FROM session_events
               WHERE session_id = ? AND turn_id = ?
               ORDER BY seq ASC"#,
        )
        .bind(session_id.0.to_string())
        .bind(turn_id.0.to_string())
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
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
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
    ) -> Result<Vec<SessionEvent>> {
        let session_id_str = session_id.0.to_string();
        let rows = sqlx::query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, created_at
               FROM session_events
               WHERE session_id = ?
               ORDER BY seq DESC
               LIMIT ?"#,
        )
        .bind(session_id_str)
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

struct SessionCatchupRow {
    session: Session,
    last_message_at: Option<DateTime<Utc>>,
    last_message_preview: Option<String>,
    last_event_seq: Option<i64>,
    activity: SessionActivityState,
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

fn track_status_to_str(status: &TrackStatus) -> &'static str {
    match status {
        TrackStatus::Pending => "pending",
        TrackStatus::Running => "running",
        TrackStatus::Completed => "completed",
        TrackStatus::Failed => "failed",
        TrackStatus::Cancelled => "cancelled",
    }
}

fn parse_track_status(value: &str) -> TrackStatus {
    match value {
        "pending" => TrackStatus::Pending,
        "running" => TrackStatus::Running,
        "completed" => TrackStatus::Completed,
        "failed" => TrackStatus::Failed,
        "cancelled" => TrackStatus::Cancelled,
        _ => TrackStatus::Pending,
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

fn track_attachment_status_to_str(status: &TrackAttachmentStatus) -> &'static str {
    match status {
        TrackAttachmentStatus::Ready => "ready",
        TrackAttachmentStatus::Stale => "stale",
        TrackAttachmentStatus::Error => "error",
    }
}

fn parse_track_attachment_status(value: &str) -> TrackAttachmentStatus {
    match value {
        "ready" => TrackAttachmentStatus::Ready,
        "stale" => TrackAttachmentStatus::Stale,
        "error" => TrackAttachmentStatus::Error,
        _ => TrackAttachmentStatus::Error,
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

fn build_artifact_from_row(r: sqlx::sqlite::SqliteRow) -> Result<Artifact> {
    let id: String = r.try_get("id")?;
    let session_id: String = r.try_get("session_id")?;
    let track_id: String = r.try_get("track_id")?;
    let task_id: String = r.try_get("task_id")?;
    let workspace_id: String = r.try_get("workspace_id")?;
    let worktree_id: String = r.try_get("worktree_id")?;
    let created_at: String = r.try_get("created_at")?;

    Ok(Artifact {
        id: ArtifactId(uuid::Uuid::parse_str(&id)?),
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        track_id: TrackId(uuid::Uuid::parse_str(&track_id)?),
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
