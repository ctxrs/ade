use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use context_core::ids::*;
use context_core::models::*;
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
            .busy_timeout(Duration::from_secs(5));
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
            r#"INSERT INTO messages (id, session_id, task_id, track_id, run_id, turn_id, role, content, attachments_json, delivery, delivered_at, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(message.id.0.to_string())
        .bind(message.session_id.0.to_string())
        .bind(message.task_id.0.to_string())
        .bind(message.track_id.0.to_string())
        .bind(message.run_id.map(|r| r.0.to_string()))
        .bind(message.turn_id.map(|t| t.0.to_string()))
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
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages WHERE session_id = ? ORDER BY created_at ASC"#,
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
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, role, content, attachments_json, delivery, delivered_at, created_at
               FROM messages
               WHERE session_id = ? AND delivery = 'queued' AND delivered_at IS NULL
               ORDER BY created_at ASC"#,
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
            r#"SELECT id, session_id, task_id, track_id, run_id, turn_id, role, content, attachments_json, delivery, delivered_at, created_at
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
        let event = SessionEvent {
            id: SessionEventId::new(),
            session_id,
            run_id,
            turn_id,
            event_type,
            payload_json: payload_json.clone(),
            created_at: Utc::now(),
        };
        sqlx::query(
            r#"INSERT INTO session_events (id, session_id, run_id, turn_id, event_type, payload_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(event.id.0.to_string())
        .bind(event.session_id.0.to_string())
        .bind(event.run_id.map(|r| r.0.to_string()))
        .bind(event.turn_id.map(|t| t.0.to_string()))
        .bind(session_event_type_to_str(&event.event_type))
        .bind(payload_json.to_string())
        .bind(event.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(event)
    }

    pub async fn list_session_events(&self, session_id: SessionId) -> Result<Vec<SessionEvent>> {
        self.list_session_events_page(session_id, None, None).await
    }

    pub async fn list_session_events_page(
        &self,
        session_id: SessionId,
        after: Option<SessionEventId>,
        limit: Option<u32>,
    ) -> Result<Vec<SessionEvent>> {
        let session_id_str = session_id.0.to_string();
        let limit_i64 = limit.map(|n| n as i64);

        let (query, binds): (String, Vec<String>) = if let Some(after_id) = after {
            let after_id_str = after_id.0.to_string();
            let row = sqlx::query(
                r#"SELECT created_at FROM session_events WHERE id = ? AND session_id = ?"#,
            )
            .bind(&after_id_str)
            .bind(&session_id_str)
            .fetch_optional(&self.pool)
            .await?;
            let Some(row) = row else {
                anyhow::bail!("after event not found for session");
            };
            let after_created_at: String = row.try_get("created_at")?;

            let mut q = String::from(
                r#"SELECT id, session_id, run_id, turn_id, event_type, payload_json, created_at
                   FROM session_events
                   WHERE session_id = ?
                     AND (created_at > ? OR (created_at = ? AND id > ?))
                   ORDER BY created_at ASC, id ASC"#,
            );
            if limit_i64.is_some() {
                q.push_str(" LIMIT ?");
            }
            (
                q,
                vec![
                    session_id_str.clone(),
                    after_created_at.clone(),
                    after_created_at,
                    after_id_str,
                ],
            )
        } else {
            let mut q = String::from(
                r#"SELECT id, session_id, run_id, turn_id, event_type, payload_json, created_at
                   FROM session_events WHERE session_id = ?
                   ORDER BY created_at ASC, id ASC"#,
            );
            if limit_i64.is_some() {
                q.push_str(" LIMIT ?");
            }
            (q, vec![session_id_str.clone()])
        };

        let mut sql = sqlx::query(&query);
        for b in binds {
            sql = sql.bind(b);
        }
        if let Some(l) = limit_i64 {
            sql = sql.bind(l);
        }

        let rows = sql.fetch_all(&self.pool).await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let session_id: String = r.try_get("session_id")?;
            let run_id: Option<String> = r.try_get("run_id")?;
            let turn_id: Option<String> = r.try_get("turn_id")?;
            let created_at: String = r.try_get("created_at")?;
            let payload_json: String = r.try_get("payload_json")?;
            out.push(SessionEvent {
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
