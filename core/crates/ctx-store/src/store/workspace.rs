use super::*;

#[derive(Clone, Debug)]
pub struct WorkspaceMergeQueueEntryIndexRecord {
    pub entry_id: MergeQueueEntryId,
    pub status: MergeQueueEntryStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct QueuedMergeQueueEntryRoute {
    pub entry_id: MergeQueueEntryId,
    pub workspace_id: WorkspaceId,
}

impl Store {
    // Workspace APIs
    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let rows = self
            .query(
                r#"SELECT id, name, root_path, created_at, vcs_kind FROM workspaces ORDER BY created_at ASC"#,
            )
            .fetch_all(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let name: String = r.try_get("name")?;
            let root_path: String = r.try_get("root_path")?;
            let created_at: String = r.try_get("created_at")?;
            let vcs_kind: Option<String> = r.try_get("vcs_kind").ok();
            out.push(Workspace {
                id: WorkspaceId(uuid::Uuid::parse_str(&id)?),
                name,
                root_path,
                created_at: parse_dt(&created_at)?,
                vcs_kind: parse_vcs_kind(vcs_kind),
            });
        }
        Ok(out)
    }

    pub async fn get_workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        let row = self
            .query(
                r#"SELECT id, name, root_path, created_at, vcs_kind FROM workspaces WHERE id = ?"#,
            )
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| {
            let id: String = r.try_get("id").ok()?;
            let vcs_kind: Option<String> = r.try_get("vcs_kind").ok()?;
            Some(Workspace {
                id: WorkspaceId(uuid::Uuid::parse_str(&id).ok()?),
                name: r.try_get("name").ok()?,
                root_path: r.try_get("root_path").ok()?,
                created_at: parse_dt(r.try_get::<String, _>("created_at").ok()?.as_str()).ok()?,
                vcs_kind: parse_vcs_kind(vcs_kind),
            })
        }))
    }

    pub async fn create_workspace(
        &self,
        name: String,
        root_path: String,
        vcs_kind: VcsKind,
    ) -> Result<Workspace> {
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name,
            root_path,
            created_at: Utc::now(),
            vcs_kind: Some(vcs_kind),
        };
        self.query(
            r#"INSERT INTO workspaces (id, name, root_path, created_at, vcs_kind) VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(workspace.id.0.to_string())
        .bind(&workspace.name)
        .bind(&workspace.root_path)
        .bind(workspace.created_at.to_rfc3339())
        .bind(workspace.vcs_kind.as_ref().map(vcs_kind_to_str))
        .execute(&self.pool)
        .await?;
        Ok(workspace)
    }

    pub async fn delete_workspace(&self, id: WorkspaceId) -> Result<()> {
        self.query(r#"DELETE FROM workspaces WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_workspace(&self, workspace: &Workspace) -> Result<()> {
        self.query(
            r#"INSERT INTO workspaces (id, name, root_path, created_at, vcs_kind)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 root_path = excluded.root_path,
                 vcs_kind = excluded.vcs_kind"#,
        )
        .bind(workspace.id.0.to_string())
        .bind(&workspace.name)
        .bind(&workspace.root_path)
        .bind(workspace.created_at.to_rfc3339())
        .bind(workspace.vcs_kind.as_ref().map(vcs_kind_to_str))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_task_index(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.query(
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
        self.query(
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
        self.query(
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
        self.query(r#"DELETE FROM workspace_task_index WHERE task_id = ?"#)
            .bind(task_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_session_index(&self, session_id: SessionId) -> Result<()> {
        self.query(r#"DELETE FROM workspace_session_index WHERE session_id = ?"#)
            .bind(session_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_worktree_index(&self, worktree_id: WorktreeId) -> Result<()> {
        self.query(r#"DELETE FROM workspace_worktree_index WHERE worktree_id = ?"#)
            .bind(worktree_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_workspace_id_for_task(&self, task_id: TaskId) -> Result<Option<WorkspaceId>> {
        let row = self
            .query(r#"SELECT workspace_id FROM workspace_task_index WHERE task_id = ?"#)
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
        let row = self
            .query(r#"SELECT workspace_id FROM workspace_session_index WHERE session_id = ?"#)
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
        let row = self
            .query(r#"SELECT workspace_id FROM workspace_worktree_index WHERE worktree_id = ?"#)
            .bind(worktree_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn upsert_workspace_artifact_index(
        &self,
        artifact_id: ArtifactId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.query(
            r#"INSERT INTO workspace_artifact_index (artifact_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(artifact_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(artifact_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_message_index(
        &self,
        message_id: MessageId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.query(
            r#"INSERT INTO workspace_message_index (message_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(message_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(message_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_subagent_invocation_index(
        &self,
        invocation_id: &str,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.query(
            r#"INSERT INTO workspace_subagent_invocation_index (invocation_id, workspace_id)
               VALUES (?, ?)
               ON CONFLICT(invocation_id) DO UPDATE SET workspace_id = excluded.workspace_id"#,
        )
        .bind(invocation_id)
        .bind(workspace_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_workspace_merge_queue_entry_index(
        &self,
        entry_id: MergeQueueEntryId,
        workspace_id: WorkspaceId,
        status: &MergeQueueEntryStatus,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        self.query(
            r#"INSERT INTO workspace_merge_queue_entry_index (entry_id, workspace_id, status, created_at)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(entry_id) DO UPDATE SET
                   workspace_id = excluded.workspace_id,
                   status = excluded.status,
                   created_at = excluded.created_at"#,
        )
        .bind(entry_id.0.to_string())
        .bind(workspace_id.0.to_string())
        .bind(merge_queue_entry_status_to_str(status))
        .bind(created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_workspace_artifact_index(&self, artifact_id: ArtifactId) -> Result<()> {
        self.query(r#"DELETE FROM workspace_artifact_index WHERE artifact_id = ?"#)
            .bind(artifact_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_message_index(&self, message_id: MessageId) -> Result<()> {
        self.query(r#"DELETE FROM workspace_message_index WHERE message_id = ?"#)
            .bind(message_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_subagent_invocation_index(
        &self,
        invocation_id: &str,
    ) -> Result<()> {
        self.query(r#"DELETE FROM workspace_subagent_invocation_index WHERE invocation_id = ?"#)
            .bind(invocation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_workspace_merge_queue_entry_index(
        &self,
        entry_id: MergeQueueEntryId,
    ) -> Result<()> {
        self.query(r#"DELETE FROM workspace_merge_queue_entry_index WHERE entry_id = ?"#)
            .bind(entry_id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_workspace_id_for_artifact(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<Option<WorkspaceId>> {
        let row = self
            .query(r#"SELECT workspace_id FROM workspace_artifact_index WHERE artifact_id = ?"#)
            .bind(artifact_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn get_workspace_id_for_message(
        &self,
        message_id: MessageId,
    ) -> Result<Option<WorkspaceId>> {
        let row = self
            .query(r#"SELECT workspace_id FROM workspace_message_index WHERE message_id = ?"#)
            .bind(message_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn get_workspace_id_for_subagent_invocation(
        &self,
        invocation_id: &str,
    ) -> Result<Option<WorkspaceId>> {
        let row = self
            .query(
                r#"SELECT workspace_id
                   FROM workspace_subagent_invocation_index
                   WHERE invocation_id = ?"#,
            )
            .bind(invocation_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn get_workspace_id_for_merge_queue_entry(
        &self,
        entry_id: MergeQueueEntryId,
    ) -> Result<Option<WorkspaceId>> {
        let row = self
            .query(
                r#"SELECT workspace_id
                   FROM workspace_merge_queue_entry_index
                   WHERE entry_id = ?"#,
            )
            .bind(entry_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let value: String = r.try_get("workspace_id").ok()?;
            uuid::Uuid::parse_str(&value).ok().map(WorkspaceId)
        }))
    }

    pub async fn list_queued_merge_queue_entry_routes(
        &self,
    ) -> Result<Vec<QueuedMergeQueueEntryRoute>> {
        let rows = self
            .query(
                r#"SELECT entry_id, workspace_id
                   FROM workspace_merge_queue_entry_index
                   WHERE status = ?
                   ORDER BY created_at ASC"#,
            )
            .bind(merge_queue_entry_status_to_str(
                &MergeQueueEntryStatus::Queued,
            ))
            .fetch_all(&self.pool)
            .await?;
        let mut routes = Vec::with_capacity(rows.len());
        for row in rows {
            let entry_id: String = row.try_get("entry_id")?;
            let workspace_id: String = row.try_get("workspace_id")?;
            routes.push(QueuedMergeQueueEntryRoute {
                entry_id: MergeQueueEntryId(uuid::Uuid::parse_str(&entry_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id)?),
            });
        }
        Ok(routes)
    }

    pub async fn replace_workspace_artifact_index(
        &self,
        workspace_id: WorkspaceId,
        artifact_ids: &[ArtifactId],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.query(r#"DELETE FROM workspace_artifact_index WHERE workspace_id = ?"#)
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        for artifact_id in artifact_ids {
            self.query(
                r#"INSERT INTO workspace_artifact_index (artifact_id, workspace_id)
                   VALUES (?, ?)"#,
            )
            .bind(artifact_id.0.to_string())
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn replace_workspace_message_index(
        &self,
        workspace_id: WorkspaceId,
        message_ids: &[MessageId],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.query(r#"DELETE FROM workspace_message_index WHERE workspace_id = ?"#)
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        for message_id in message_ids {
            self.query(
                r#"INSERT INTO workspace_message_index (message_id, workspace_id)
                   VALUES (?, ?)"#,
            )
            .bind(message_id.0.to_string())
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn replace_workspace_subagent_invocation_index(
        &self,
        workspace_id: WorkspaceId,
        invocation_ids: &[String],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.query(r#"DELETE FROM workspace_subagent_invocation_index WHERE workspace_id = ?"#)
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        for invocation_id in invocation_ids {
            self.query(
                r#"INSERT INTO workspace_subagent_invocation_index (invocation_id, workspace_id)
                   VALUES (?, ?)"#,
            )
            .bind(invocation_id)
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn replace_workspace_merge_queue_entry_index(
        &self,
        workspace_id: WorkspaceId,
        entries: &[WorkspaceMergeQueueEntryIndexRecord],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.query(r#"DELETE FROM workspace_merge_queue_entry_index WHERE workspace_id = ?"#)
            .bind(workspace_id.0.to_string())
            .execute(&mut *tx)
            .await?;
        for entry in entries {
            self.query(
                r#"INSERT INTO workspace_merge_queue_entry_index (entry_id, workspace_id, status, created_at)
                   VALUES (?, ?, ?, ?)"#,
            )
            .bind(entry.entry_id.0.to_string())
            .bind(workspace_id.0.to_string())
            .bind(merge_queue_entry_status_to_str(&entry.status))
            .bind(entry.created_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn refresh_workspace_indexes(&self, workspace_id: WorkspaceId) -> Result<()> {
        let workspace_id = workspace_id.0.to_string();
        self.query(
            r#"INSERT INTO workspace_task_index (task_id, workspace_id)
               SELECT id, workspace_id FROM tasks WHERE workspace_id = ?
               ON CONFLICT(task_id) DO UPDATE SET
                   workspace_id = excluded.workspace_id"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;

        self.query(
            r#"INSERT INTO workspace_session_index (session_id, workspace_id)
               SELECT id, workspace_id FROM sessions WHERE workspace_id = ?
               ON CONFLICT(session_id) DO UPDATE SET
                   workspace_id = excluded.workspace_id"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;

        self.query(
            r#"INSERT INTO workspace_worktree_index (worktree_id, workspace_id)
               SELECT id, workspace_id FROM worktrees WHERE workspace_id = ?
               ON CONFLICT(worktree_id) DO UPDATE SET
                   workspace_id = excluded.workspace_id"#,
        )
        .bind(&workspace_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_workspace_indexes(&self, workspace_id: WorkspaceId) -> Result<()> {
        let workspace_id = workspace_id.0.to_string();
        self.query(r#"DELETE FROM workspace_task_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        self.query(r#"DELETE FROM workspace_session_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        self.query(r#"DELETE FROM workspace_worktree_index WHERE workspace_id = ?"#)
            .bind(&workspace_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
