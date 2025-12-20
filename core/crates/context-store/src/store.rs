use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use context_core::ids::*;
use context_core::models::*;
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};

#[derive(Clone)]
pub struct Store {
    pool: Pool<Sqlite>,
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
        let row = sqlx::query(
            r#"SELECT id, name, root_path, created_at FROM workspaces WHERE id = ?"#,
        )
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
            let last_assistant_message_at: Option<String> = r.try_get("last_assistant_message_at")?;
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
                last_assistant_message_at: last_assistant_message_at.as_deref().map(parse_dt).transpose()?,
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
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose().ok()?,
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
            let last_assistant_message_at: Option<String> = r.try_get("last_assistant_message_at").ok()?;
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
                assistant_seen_at: assistant_seen_at.as_deref().map(parse_dt).transpose().ok()?,
                last_activity_at: last_activity_at.as_deref().map(parse_dt).transpose().ok()?,
                last_assistant_message_at: last_assistant_message_at.as_deref().map(parse_dt).transpose().ok()?,
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
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at
               FROM worktrees WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let ws_id: String = r.try_get("workspace_id").ok()?;
            let created_at: String = r.try_get("created_at").ok()?;
            Some(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                root_path: r.try_get("root_path").ok()?,
                base_commit_sha: r.try_get("base_commit_sha").ok()?,
                git_branch: r.try_get("git_branch").ok()?,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn get_local_worktree_for_root(
        &self,
        workspace_id: WorkspaceId,
        root_path: &str,
    ) -> Result<Option<Worktree>> {
        let row = sqlx::query(
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at
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
            Some(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id).ok()?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id).ok()?),
                root_path: r.try_get("root_path").ok()?,
                base_commit_sha: r.try_get("base_commit_sha").ok()?,
                git_branch: r.try_get("git_branch").ok()?,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn list_worktrees(&self, workspace_id: WorkspaceId) -> Result<Vec<Worktree>> {
        let rows = sqlx::query(
            r#"SELECT id, workspace_id, root_path, base_commit_sha, git_branch, created_at
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
            out.push(Worktree {
                id: WorktreeId(uuid::Uuid::parse_str(&id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&ws_id)?),
                root_path: r.try_get("root_path")?,
                base_commit_sha: r.try_get("base_commit_sha")?,
                git_branch: r.try_get("git_branch")?,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
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
    pub async fn create_session(
        &self,
        track: &Track,
        provider_id: String,
        model_id: String,
        agent_role: String,
        provider_session_ref: Option<String>,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: SessionId::new(),
            track_id: track.id,
            task_id: track.task_id,
            workspace_id: track.workspace_id,
            worktree_id: track.worktree_id,
            provider_id,
            model_id,
            agent_role,
            status: SessionStatus::Active,
            provider_session_ref,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"INSERT INTO sessions (id, track_id, task_id, workspace_id, worktree_id, provider_id, model_id,
               agent_role, status, provider_session_ref, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(session.id.0.to_string())
        .bind(session.track_id.0.to_string())
        .bind(session.task_id.0.to_string())
        .bind(session.workspace_id.0.to_string())
        .bind(session.worktree_id.0.to_string())
        .bind(&session.provider_id)
        .bind(&session.model_id)
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
            r#"SELECT id, track_id, task_id, workspace_id, worktree_id, provider_id, model_id, agent_role,
               status, provider_session_ref, created_at, updated_at
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
                provider_id: r.try_get("provider_id").ok()?,
                model_id: r.try_get("model_id").ok()?,
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
            r#"SELECT id, track_id, task_id, workspace_id, worktree_id, provider_id, model_id, agent_role,
               status, provider_session_ref, created_at, updated_at
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
                provider_id: r.try_get("provider_id")?,
                model_id: r.try_get("model_id")?,
                agent_role: r.try_get("agent_role")?,
                status: parse_session_status(r.try_get::<String, _>("status")?.as_str()),
                provider_session_ref: r.try_get("provider_session_ref")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            });
        }
        Ok(out)
    }

    // Message APIs
    pub async fn insert_message(&self, mut message: Message) -> Result<Message> {
        if matches!(message.delivery, MessageDelivery::Immediate) && message.delivered_at.is_none() {
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

    pub async fn delete_session_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<()> {
        sqlx::query(
            r#"DELETE FROM session_turns WHERE session_id = ? AND turn_id = ?"#,
        )
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
        delta_total: i64,
        delta_pending: i64,
        delta_running: i64,
        delta_completed: i64,
        delta_failed: i64,
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
        .bind(delta_total)
        .bind(delta_pending)
        .bind(delta_running)
        .bind(delta_completed)
        .bind(delta_failed)
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
        self.ensure_session_turns(session_id).await?;
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

    async fn ensure_session_turns(&self, session_id: SessionId) -> Result<()> {
        let count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM session_turns WHERE session_id = ?"#,
        )
        .bind(session_id.0.to_string())
        .fetch_one(&self.pool)
        .await?;
        if count > 0 {
            return Ok(());
        }
        self.backfill_session_turns(session_id).await
    }

    async fn backfill_session_turns(&self, session_id: SessionId) -> Result<()> {
        let messages = self.list_messages_for_session(session_id).await?;
        let events = self.list_session_events(session_id).await?;

        let mut user_by_turn: HashMap<TurnId, Message> = HashMap::new();
        let mut assistant_last_by_turn: HashMap<TurnId, Message> = HashMap::new();
        for m in messages {
            let Some(turn_id) = m.turn_id else { continue };
            match m.role {
                MessageRole::User => {
                    user_by_turn.insert(turn_id, m);
                }
                MessageRole::Assistant => {
                    let update = match assistant_last_by_turn.get(&turn_id) {
                        Some(existing) => m.created_at > existing.created_at,
                        None => true,
                    };
                    if update {
                        assistant_last_by_turn.insert(turn_id, m);
                    }
                }
                _ => {}
            }
        }

        let mut events_by_turn: HashMap<TurnId, Vec<SessionEvent>> = HashMap::new();
        for ev in events {
            let Some(turn_id) = ev.turn_id else { continue };
            events_by_turn.entry(turn_id).or_default().push(ev);
        }

        let mut turn_ids: HashSet<TurnId> = HashSet::new();
        turn_ids.extend(user_by_turn.keys().cloned());
        turn_ids.extend(events_by_turn.keys().cloned());

        for turn_id in turn_ids {
            let user = user_by_turn.get(&turn_id);
            let assistant = assistant_last_by_turn.get(&turn_id);
            let mut evs = events_by_turn.remove(&turn_id).unwrap_or_default();
            evs.sort_by_key(|e| e.seq);

            let mut start_seq: Option<i64> = None;
            let mut end_seq: Option<i64> = None;
            let mut assistant_partial = String::new();
            let mut thought_partial = String::new();
            let mut tool_statuses: HashMap<String, String> = HashMap::new();
            let mut saw_turn_interrupted = false;
            let mut saw_error = false;
            let mut saw_assistant_complete = false;
            let mut has_activity = false;
            let mut metrics_json: Option<serde_json::Value> = None;

            let mut last_event_at = user
                .map(|m| m.created_at)
                .or_else(|| evs.first().map(|e| e.created_at))
                .unwrap_or_else(Utc::now);

            for ev in &evs {
                if ev.created_at > last_event_at {
                    last_event_at = ev.created_at;
                }
                match ev.event_type {
                    SessionEventType::UserMessage => {
                        start_seq = start_seq.or(Some(ev.seq));
                    }
                    SessionEventType::AssistantChunk => {
                        if let Some(fragment) = ev.payload_json.get("content_fragment").and_then(|v| v.as_str()) {
                            assistant_partial.push_str(fragment);
                        }
                        has_activity = true;
                    }
                    SessionEventType::ThoughtChunk => {
                        if let Some(fragment) = ev.payload_json.get("content_fragment").and_then(|v| v.as_str()) {
                            thought_partial.push_str(fragment);
                        }
                        has_activity = true;
                    }
                    SessionEventType::AssistantComplete => {
                        saw_assistant_complete = true;
                        has_activity = true;
                    }
                    SessionEventType::AssistantMessageInserted => {
                        saw_assistant_complete = true;
                        has_activity = true;
                    }
                    SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult => {
                        if let Some((tool_call_id, status)) =
                            extract_tool_status_for_backfill(ev)
                        {
                            tool_statuses.insert(tool_call_id, status);
                        }
                        has_activity = true;
                    }
                    SessionEventType::TurnInterrupted => {
                        saw_turn_interrupted = true;
                        end_seq = Some(ev.seq);
                        has_activity = true;
                    }
                    SessionEventType::Done => {
                        end_seq = Some(ev.seq);
                        metrics_json = ev
                            .payload_json
                            .get("context_window")
                            .cloned()
                            .or(metrics_json);
                        has_activity = true;
                    }
                    SessionEventType::Error => {
                        saw_error = true;
                        has_activity = true;
                    }
                    SessionEventType::Init
                    | SessionEventType::Notice
                    | SessionEventType::AuthRequired
                    | SessionEventType::InputQueued
                    | SessionEventType::InterruptRequested
                    | SessionEventType::Plan => {}
                }
            }

            let (tool_pending, tool_running, tool_completed, tool_failed) =
                tally_tool_statuses(&tool_statuses);

            if start_seq.is_none() {
                if let Some(first_ev) = evs.first() {
                    start_seq = Some(first_ev.seq);
                }
            }

            let status = if saw_turn_interrupted {
                SessionTurnStatus::Interrupted
            } else if saw_error && assistant.is_none() && !saw_assistant_complete {
                SessionTurnStatus::Failed
            } else if assistant.is_some() || saw_assistant_complete {
                SessionTurnStatus::Completed
            } else if user
                .map(|m| matches!(m.delivery, MessageDelivery::Queued))
                .unwrap_or(false)
                && !has_activity
            {
                SessionTurnStatus::Queued
            } else {
                SessionTurnStatus::Running
            };

            let started_at = user
                .map(|m| m.created_at)
                .or_else(|| evs.first().map(|e| e.created_at))
                .unwrap_or_else(Utc::now);
            let updated_at = assistant
                .map(|m| m.created_at)
                .unwrap_or(last_event_at);

            let turn = SessionTurn {
                turn_id,
                session_id,
                run_id: user.and_then(|m| m.run_id),
                user_message_id: user.map(|m| m.id),
                status,
                start_seq,
                end_seq,
                started_at,
                updated_at,
                assistant_partial: if assistant_partial.is_empty() {
                    None
                } else {
                    Some(assistant_partial)
                },
                thought_partial: if thought_partial.is_empty() {
                    None
                } else {
                    Some(thought_partial)
                },
                metrics_json,
                tool_total: tool_statuses.len() as i64,
                tool_pending,
                tool_running,
                tool_completed,
                tool_failed,
            };
            let _ = self.insert_session_turn(turn).await;
        }

        Ok(())
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

    // Session event APIs
    pub async fn append_session_event(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> Result<SessionEvent> {
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
        Ok(event)
    }

    pub async fn list_session_events(&self, session_id: SessionId) -> Result<Vec<SessionEvent>> {
        self.list_session_events_page_by_seq(session_id, None, None).await
    }

    pub async fn list_session_events_page_by_seq(
        &self,
        session_id: SessionId,
        after_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<SessionEvent>> {
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
                event_type: parse_session_event_type(r.try_get::<String, _>("event_type")?.as_str()),
                payload_json: serde_json::from_str(&payload_json)
                    .context("parsing session event payload")?,
                created_at: parse_dt(&created_at)?,
            });
        }
        Ok(out)
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
                payload_json: serde_json::from_str(&payload_json).context("parsing payload_json")?,
                created_at: parse_dt(&created_at)?,
            });
        }
        out.reverse(); // return ASC
        Ok(out)
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
        "done" => SessionEventType::Done,
        "interrupt_requested" => SessionEventType::InterruptRequested,
        "turn_interrupted" => SessionEventType::TurnInterrupted,
        "error" => SessionEventType::Error,
        _ => SessionEventType::Error,
    }
}

fn build_session_turn_from_row(
    r: sqlx::sqlite::SqliteRow,
) -> Result<SessionTurn> {
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

fn build_session_turn_tool_from_row(
    r: sqlx::sqlite::SqliteRow,
) -> Result<SessionTurnTool> {
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

fn extract_tool_status_for_backfill(ev: &SessionEvent) -> Option<(String, String)> {
    let tool_call_id = tool_call_id_from_payload(&ev.payload_json)?;
    let update = extract_tool_update(&ev.payload_json);
    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));

    let status = if let Some(raw) = raw_status {
        normalize_tool_status(raw, ev.event_type.clone())
    } else if matches!(ev.event_type, SessionEventType::ToolResult) {
        "completed".to_string()
    } else if matches!(ev.event_type, SessionEventType::ToolCall) {
        "pending".to_string()
    } else {
        return None;
    };

    Some((tool_call_id, status))
}

fn tally_tool_statuses(statuses: &HashMap<String, String>) -> (i64, i64, i64, i64) {
    let mut pending = 0;
    let mut running = 0;
    let mut completed = 0;
    let mut failed = 0;
    for status in statuses.values() {
        match status.as_str() {
            "pending" => pending += 1,
            "in_progress" => running += 1,
            "completed" => completed += 1,
            "failed" => failed += 1,
            _ => pending += 1,
        }
    }
    (pending, running, completed, failed)
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
        .or_else(|| update.pointer("/raw_input/call_id").and_then(|v| v.as_str()));
    from_raw.map(|v| v.to_string())
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/outputText").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/output_text").and_then(|v| v.as_str()))
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/rawOutput/aggregated_output").and_then(|v| v.as_str()))
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
        if let Some(t) = b.get("content").and_then(|c| c.get("text")).and_then(|v| v.as_str()) {
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
        let entry = map.entry(tool_call_id.clone()).or_insert_with(ToolAgg::default);
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
