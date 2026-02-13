use super::*;

impl Store {
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
        self.create_session_with_id(
            SessionId::new(),
            task_id,
            workspace_id,
            worktree_id,
            provider_id,
            model_id,
            agent_role,
            parent_session_id,
            relationship,
            provider_session_ref,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_session_with_id(
        &self,
        session_id: SessionId,
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
        let title = if relationship.as_deref() == Some("sub_agent") {
            format!("subagent-{}", session_id.0)
        } else {
            "New Task".to_string()
        };
        let session = Session {
            id: session_id,
            task_id,
            workspace_id,
            worktree_id,
            parent_session_id,
            relationship,
            provider_id,
            model_id,
            title,
            agent_role,
            status: SessionStatus::Active,
            provider_session_ref,
            created_at: now,
            updated_at: now,
        };
        let result = self.query(
            r#"INSERT INTO sessions (id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, title, agent_role, status, provider_session_ref, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(id) DO NOTHING"#,
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

        if result.rows_affected() == 0 {
            return self
                .get_session(session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("session exists but could not be loaded"));
        }

        self.ensure_session_snapshot_summary(session.id).await?;
        self.refresh_active_snapshot_head(session.id, None).await?;
        Ok(session)
    }

    pub async fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        let row = self.query(
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
        self.query(
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
        let res = self
            .query(
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
        self.query(
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
        let rows = self.query(
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
        let rows = self.query(
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
        let rows = self
            .query(
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

    pub async fn get_subagent_session_by_label(
        &self,
        parent_session_id: SessionId,
        label: &str,
    ) -> Result<Option<Session>> {
        let row = self
            .query(
                r#"SELECT id, task_id, workspace_id, worktree_id, parent_session_id, relationship,
               provider_id, model_id, agent_role, title, status, provider_session_ref, created_at, updated_at
               FROM sessions
               WHERE parent_session_id = ? AND relationship = 'sub_agent' AND title = ?
               LIMIT 1"#,
            )
            .bind(parent_session_id.0.to_string())
            .bind(label)
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

    pub async fn subagent_label_exists(&self, task_id: TaskId, label: &str) -> Result<bool> {
        let row = self
            .query(
                r#"SELECT 1
               FROM sessions
               WHERE task_id = ? AND relationship = 'sub_agent' AND title = ?
               LIMIT 1"#,
            )
            .bind(task_id.0.to_string())
            .bind(label)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn get_session_summary_checkpoint(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSummaryCheckpoint>> {
        let row = self.query(
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
        self.query(
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

        self.refresh_active_snapshot_head(checkpoint.session_id, None)
            .await?;
        Ok(checkpoint)
    }

    pub async fn upsert_subagent_invocation(
        &self,
        invocation: SubagentInvocation,
    ) -> Result<SubagentInvocation> {
        self.query(
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
        self.query(
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
        self.query(
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
        let row = self
            .query(
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
        let rows = self
            .query(
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
        let mut sql = String::from(
            r#"SELECT id, tool_call_id, parent_session_id, parent_turn_id,
                      requested_count, request_json, status, created_at, updated_at
               FROM subagent_invocations
               WHERE parent_session_id = ?"#,
        );
        if parent_turn_id.is_some() {
            sql.push_str(" AND parent_turn_id = ?");
        }
        sql.push_str(" ORDER BY created_at ASC");
        let sql = self.rewrite_sql(&sql);
        let mut query = sqlx::query(sql.as_ref()).bind(parent_session_id.0.to_string());
        if let Some(turn_id) = parent_turn_id {
            query = query.bind(turn_id.0.to_string());
        }
        let rows = query.fetch_all(&self.pool).await?;

        let mut invocations = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(invocation) = build_subagent_invocation_from_row(r) {
                invocations.push(invocation);
            }
        }
        if invocations.is_empty() {
            return Ok(invocations);
        }

        let mut child_sql = String::from(
            r#"SELECT invocation_id, child_session_id, run_id, position, status,
                      label, harness, model, reasoning_effort, prompt_length,
                      created_at, updated_at
               FROM subagent_invocation_children
               WHERE invocation_id IN ("#,
        );
        for i in 0..invocations.len() {
            if i > 0 {
                child_sql.push_str(", ");
            }
            child_sql.push('?');
        }
        child_sql.push_str(") ORDER BY position ASC");
        let child_sql = self.rewrite_sql(&child_sql);
        let mut child_query = sqlx::query(child_sql.as_ref());
        for invocation in &invocations {
            child_query = child_query.bind(invocation.id.clone());
        }
        let child_rows = child_query.fetch_all(&self.pool).await?;

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
}
