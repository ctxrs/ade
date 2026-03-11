use super::*;

impl Store {
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
        let id = message.id.0.to_string();
        let session_id = message.session_id.0.to_string();
        let task_id = message.task_id.0.to_string();
        let run_id = message.run_id.map(|r| r.0.to_string());
        let turn_id = message.turn_id.map(|t| t.0.to_string());
        let role = message_role_to_str(&message.role);
        let delivery = message_delivery_to_str(&message.delivery);
        let delivered_at = message.delivered_at.map(|d| d.to_rfc3339());
        let created_at = message.created_at.to_rfc3339();
        let write_bytes = bytes_str(&id)
            + bytes_str(&session_id)
            + bytes_str(&task_id)
            + bytes_opt_str(run_id.as_deref())
            + bytes_opt_str(turn_id.as_deref())
            + bytes_opt_i64(message.turn_sequence)
            + bytes_opt_i64(message.order_seq)
            + bytes_str(role)
            + bytes_str(&message.content)
            + bytes_opt_str(attachments_json.as_deref())
            + bytes_str(delivery)
            + bytes_opt_str(delivered_at.as_deref())
            + bytes_str(&created_at);
        let result = self.query(
            r#"INSERT INTO messages (id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&id)
        .bind(&session_id)
        .bind(&task_id)
        .bind(run_id)
        .bind(turn_id)
        .bind(message.turn_sequence)
        .bind(message.order_seq)
        .bind(role)
        .bind(&message.content)
        .bind(attachments_json)
        .bind(delivery)
        .bind(delivered_at)
        .bind(&created_at)
        .execute(&self.pool)
        .await?;
        record_write(
            WriteMetricTable::Messages,
            result.rows_affected(),
            write_bytes,
        );
        self.update_task_activity_from_message(&message).await?;
        if matches!(message.role, MessageRole::Assistant | MessageRole::User) {
            self.update_session_snapshot_last_message(&message).await?;
        }
        self.refresh_active_snapshot_head(message.session_id, None)
            .await?;
        Ok(message)
    }

    pub(super) async fn ensure_session_snapshot_summary(
        &self,
        session_id: SessionId,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let session_id = session_id.0.to_string();
        let write_bytes = bytes_str(&session_id) + I64_BYTES + (bytes_str(&now) * 2);
        let result = self
            .query(
                r#"INSERT INTO session_snapshot_summaries (
                    session_id, running_turn_count, created_at, updated_at
               )
               VALUES (?, 0, ?, ?)
               ON CONFLICT(session_id) DO NOTHING"#,
            )
            .bind(&session_id)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionSnapshotSummaries,
            result.rows_affected(),
            write_bytes,
        );
        Ok(())
    }

    pub(super) async fn update_task_activity_from_message(&self, message: &Message) -> Result<()> {
        let created_at = message.created_at.to_rfc3339();
        let is_assistant = matches!(message.role, MessageRole::Assistant);
        self.query(
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

    pub(super) async fn update_session_snapshot_last_message(
        &self,
        message: &Message,
    ) -> Result<()> {
        self.ensure_session_snapshot_summary(message.session_id)
            .await?;
        let created_at = message.created_at.to_rfc3339();
        let content = message.content.clone();
        let session_id = message.session_id.0.to_string();
        let write_bytes = bytes_str(&created_at) * 2 + bytes_str(&content);
        let result = self
            .query(
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
            .bind(&session_id)
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionSnapshotSummaries,
            result.rows_affected(),
            write_bytes,
        );
        Ok(())
    }

    pub(super) async fn update_session_snapshot_last_event_seq(
        &self,
        session_id: SessionId,
        seq: i64,
    ) -> Result<()> {
        self.ensure_session_snapshot_summary(session_id).await?;
        let now = Utc::now().to_rfc3339();
        let session_id = session_id.0.to_string();
        let write_bytes = I64_BYTES + bytes_str(&now);
        let result = self
            .query(
                r#"UPDATE session_snapshot_summaries
               SET last_event_seq = ?,
                   updated_at = ?
               WHERE session_id = ?"#,
            )
            .bind(seq)
            .bind(&now)
            .bind(&session_id)
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionSnapshotSummaries,
            result.rows_affected(),
            write_bytes,
        );
        Ok(())
    }

    pub(super) async fn refresh_session_turn_summary(&self, session_id: SessionId) -> Result<()> {
        self.ensure_session_snapshot_summary(session_id).await?;
        let last_row = self
            .query(
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
        let running_count: i64 = self
            .query_scalar(
                r#"SELECT COUNT(*)
               FROM session_turns
               WHERE session_id = ? AND status = 'running'"#,
            )
            .bind(session_id.0.to_string())
            .fetch_one(&self.pool)
            .await?;
        let now = Utc::now().to_rfc3339();
        let session_id = session_id.0.to_string();
        let write_bytes = bytes_opt_str(last_status.as_deref())
            + bytes_opt_i64(last_seq)
            + I64_BYTES
            + bytes_str(&now);
        let result = self
            .query(
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
            .bind(&session_id)
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionSnapshotSummaries,
            result.rows_affected(),
            write_bytes,
        );
        Ok(())
    }

    pub async fn workspace_task_counts(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        let active: i64 = self
            .query_scalar(
                r#"SELECT COUNT(*) FROM tasks WHERE workspace_id = ? AND archived_at IS NULL"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_one(&self.pool)
            .await?;

        let archived: i64 = self
            .query_scalar(
                r#"SELECT COUNT(*) FROM tasks WHERE workspace_id = ? AND archived_at IS NOT NULL"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_one(&self.pool)
            .await?;

        Ok((active, archived))
    }
    pub async fn count_active_tasks_for_worktree(
        &self,
        worktree_id: WorktreeId,
        exclude_task_id: Option<TaskId>,
    ) -> Result<i64> {
        let count: i64 = if let Some(task_id) = exclude_task_id {
            self.query_scalar(
                r#"SELECT COUNT(*) FROM tasks
                   WHERE primary_worktree_id = ? AND archived_at IS NULL AND id != ?"#,
            )
            .bind(worktree_id.0.to_string())
            .bind(task_id.0.to_string())
            .fetch_one(&self.pool)
            .await?
        } else {
            self.query_scalar(
                r#"SELECT COUNT(*) FROM tasks
                   WHERE primary_worktree_id = ? AND archived_at IS NULL"#,
            )
            .bind(worktree_id.0.to_string())
            .fetch_one(&self.pool)
            .await?
        };
        Ok(count)
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

    pub(super) async fn list_workspace_index_page_filtered(
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

        let sql = self.rewrite_sql(&sql);
        let mut query = sqlx::query(sql.as_ref()).bind(workspace_id.0.to_string());

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

        let total_count: i64 = self
            .query_scalar(
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

        let sql = self.rewrite_sql(&sql);
        let rows = sqlx::query(sql.as_ref())
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

    pub async fn list_workspace_active_page_base(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        const ACTIVITY_EXPR: &str = "COALESCE(t.last_activity_at, t.updated_at, t.created_at)";

        let total_count: i64 = self
            .query_scalar(
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

        let sql = self.rewrite_sql(&sql);
        let rows = sqlx::query(sql.as_ref())
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

        let task_ids: Vec<TaskId> = task_rows.iter().map(|(task, _)| task.id).collect();
        let session_rows = self.list_session_snapshot_rows_base(&task_ids).await?;
        let summaries =
            Self::build_workspace_active_task_summaries_from_rows(task_rows, session_rows);
        Ok((summaries, total_count))
    }

    pub async fn list_workspace_active_session_ids(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionId>> {
        let rows = self
            .query(
                r#"SELECT s.id
               FROM tasks t
               JOIN sessions s ON s.id = t.primary_session_id
               WHERE t.workspace_id = ?
                 AND t.archived_at IS NULL
               ORDER BY t.created_at ASC, t.id ASC"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            out.push(SessionId(uuid::Uuid::parse_str(&id)?));
        }
        Ok(out)
    }

    pub async fn get_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(i64, i64)> {
        crate::fault_injection::maybe_fail("ctx_store.get_workspace_active_snapshot_state")?;
        let row = self
            .query(
                r#"SELECT snapshot_rev, archived_rev
               FROM workspace_active_snapshot_state
               WHERE workspace_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .map(|r| {
                (
                    r.try_get("snapshot_rev").unwrap_or(0),
                    r.try_get("archived_rev").unwrap_or(0),
                )
            })
            .unwrap_or((0, 0)))
    }

    pub async fn bump_workspace_active_snapshot_rev(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<i64> {
        let (snapshot_rev, _) = self
            .get_workspace_active_snapshot_state(workspace_id)
            .await?;
        Ok(snapshot_rev)
    }

    pub async fn bump_workspace_archived_snapshot_rev(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        let archived_rev: i64 = self
            .query_scalar(
                r#"INSERT INTO workspace_active_snapshot_state (
                    workspace_id, snapshot_rev, archived_rev, updated_at
               )
               VALUES (?, 0, 1, ?)
               ON CONFLICT(workspace_id) DO UPDATE SET
                   archived_rev = workspace_active_snapshot_state.archived_rev + 1,
                   updated_at = excluded.updated_at
               RETURNING archived_rev"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(&now)
            .fetch_one(&self.pool)
            .await?;
        Ok(archived_rev)
    }

    pub async fn upsert_workspace_active_task_summary_read_model(
        &self,
        summary: &WorkspaceActiveTaskSummary,
    ) -> Result<i64> {
        let (snapshot_rev, _) = self
            .get_workspace_active_snapshot_state(summary.task.workspace_id)
            .await?;
        Ok(snapshot_rev)
    }

    pub async fn delete_workspace_active_task_summary_read_model(
        &self,
        workspace_id: WorkspaceId,
        _task_id: TaskId,
    ) -> Result<i64> {
        let (snapshot_rev, _) = self
            .get_workspace_active_snapshot_state(workspace_id)
            .await?;
        Ok(snapshot_rev)
    }

    pub async fn list_workspace_active_page_read_model(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
        crate::fault_injection::maybe_fail("ctx_store.list_workspace_active_page_read_model")?;
        const MAX_LIMIT: i64 = 200;
        let limit = limit.clamp(1, MAX_LIMIT);

        let timing_enabled = snapshot_timing_enabled();
        let acquire_start = timing_enabled.then(Instant::now);
        let mut conn = self.pool.acquire().await?;
        let acquire_ms = acquire_start
            .map(|start| start.elapsed())
            .unwrap_or_default();

        let query_start = timing_enabled.then(Instant::now);
        let total_count: i64 = self
            .query_scalar(
                r#"SELECT COUNT(*)
               FROM workspace_active_task_summaries
               WHERE workspace_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_one(&mut *conn)
            .await?;

        let rows = self
            .query(
                r#"SELECT summary_json
               FROM workspace_active_task_summaries
               WHERE workspace_id = ?
               ORDER BY sort_at DESC, task_id DESC
               LIMIT ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(limit)
            .fetch_all(&mut *conn)
            .await?;
        let query_ms = query_start.map(|start| start.elapsed()).unwrap_or_default();

        let row_count = rows.len();
        let parse_start = timing_enabled.then(Instant::now);
        let mut read_models = Vec::with_capacity(rows.len());
        for row in rows {
            let summary_json: String = row.try_get("summary_json")?;
            let summary: WorkspaceActiveTaskSummaryReadModel = serde_json::from_str(&summary_json)
                .context("deserializing workspace active task summary read model")?;
            read_models.push(summary);
        }
        let parse_ms = parse_start.map(|start| start.elapsed()).unwrap_or_default();

        let mut summaries = Vec::with_capacity(read_models.len());
        for summary in read_models {
            let primary_session = summary.primary_session;
            summaries.push(WorkspaceActiveTaskSummary {
                task: summary.task,
                primary_session,
                primary_session_head: None,
                sessions: summary.sessions,
                sort_at: summary.sort_at,
            });
        }

        if timing_enabled {
            info!(
                target: "ctx_store.snapshot_timing",
                snapshot = "active_snapshot",
                workspace_id = %workspace_id.0,
                limit,
                total_count,
                rows = row_count,
                acquire_ms = acquire_ms.as_millis(),
                query_ms = query_ms.as_millis(),
                parse_ms = parse_ms.as_millis(),
            );
        }

        Ok((summaries, total_count))
    }

    pub async fn list_workspace_active_head_snapshots(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionHeadSnapshot>> {
        crate::fault_injection::maybe_fail("ctx_store.list_workspace_active_head_snapshots")?;
        let rows = self
            .query(
                r#"SELECT s.id AS session_id,
                          s.task_id,
                          s.workspace_id,
                          s.worktree_id,
                          s.execution_environment,
                          s.parent_session_id,
                          s.relationship,
                          s.provider_id,
                          s.model_id,
                          s.agent_role,
                          s.title,
                          s.status,
                          s.provider_session_ref,
                          s.created_at,
                          s.updated_at,
                          h.last_event_seq,
                          h.turns_json,
                          h.tool_summaries_json,
                          h.messages_json,
                          h.has_more_turns,
                          h.head_window_json,
                          h.summary_checkpoint_json
                   FROM session_active_snapshot_heads h
                   JOIN sessions s ON s.id = h.session_id
                   JOIN tasks t ON t.id = s.task_id
                   WHERE s.workspace_id = ?
                     AND t.archived_at IS NULL
                   ORDER BY s.created_at ASC, s.id ASC"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let session_id: String = row.try_get("session_id")?;
            let task_id: String = row.try_get("task_id")?;
            let workspace_id_value: String = row.try_get("workspace_id")?;
            let worktree_id: String = row.try_get("worktree_id")?;
            let created_at: String = row.try_get("created_at")?;
            let updated_at: String = row.try_get("updated_at")?;
            let status: String = row.try_get("status")?;

            let session = Session {
                id: SessionId(uuid::Uuid::parse_str(&session_id)?),
                task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
                workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id_value)?),
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
                execution_environment: parse_execution_environment(
                    row.try_get::<String, _>("execution_environment")?.as_str(),
                ),
                parent_session_id: parse_optional_session_id(row.try_get("parent_session_id")?),
                relationship: row.try_get("relationship")?,
                provider_id: row.try_get("provider_id")?,
                model_id: row.try_get("model_id")?,
                title: row.try_get("title")?,
                agent_role: row.try_get("agent_role")?,
                status: parse_session_status(&status),
                provider_session_ref: row.try_get("provider_session_ref")?,
                created_at: parse_dt(&created_at)?,
                updated_at: parse_dt(&updated_at)?,
            };

            let turns_json: String = row.try_get("turns_json")?;
            let tool_summaries_json: String = row.try_get("tool_summaries_json")?;
            let messages_json: String = row.try_get("messages_json")?;
            let head_window_json: String = row.try_get("head_window_json")?;
            let summary_checkpoint_json: Option<String> = row.try_get("summary_checkpoint_json")?;

            let mut turns: Vec<SessionTurn> =
                serde_json::from_str(&turns_json).context("deserializing active head turns")?;
            let tool_summaries: Vec<SessionTurnToolSummary> =
                serde_json::from_str(&tool_summaries_json)
                    .context("deserializing active head tool summaries")?;
            let messages: Vec<Message> = serde_json::from_str(&messages_json)
                .context("deserializing active head messages")?;
            let head_window: SessionHeadWindow = serde_json::from_str(&head_window_json)
                .context("deserializing active head window")?;
            let summary_checkpoint: Option<SessionSummaryCheckpoint> = summary_checkpoint_json
                .map(|json| {
                    serde_json::from_str(&json)
                        .context("deserializing active head summary checkpoint")
                })
                .transpose()?;

            let mut events = Vec::new();
            strip_snapshot_partials(&mut turns, &mut events);

            let last_status = turns.last().map(|t| t.status.clone());
            let has_running_turn = turns
                .iter()
                .any(|turn| matches!(turn.status, SessionTurnStatus::Running));
            let activity = derive_activity_from_status(last_status, has_running_turn);
            let has_more_turns: i64 = row.try_get("has_more_turns")?;
            let last_event_seq: i64 = row.try_get("last_event_seq")?;

            out.push(SessionHeadSnapshot {
                session: session_metadata_from_session(&session),
                turns,
                tool_summaries,
                events,
                messages,
                last_event_seq,
                state_rev: last_event_seq,
                activity,
                has_more_turns: has_more_turns != 0,
                history_cursor: None,
                has_more_history: false,
                summary_checkpoint,
                head_window,
            });
        }
        Ok(out)
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

        let sql = self.rewrite_sql(&sql);
        if let Some(r) = sqlx::query(sql.as_ref())
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
        let row = self
            .query(
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

    pub(super) async fn build_workspace_active_task_summaries(
        &self,
        rows: Vec<(Task, DateTime<Utc>)>,
    ) -> Result<Vec<WorkspaceActiveTaskSummary>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let task_ids: Vec<TaskId> = rows.iter().map(|(task, _)| task.id).collect();
        let session_rows = self.list_session_snapshot_rows(&task_ids).await?;
        Ok(Self::build_workspace_active_task_summaries_from_rows(
            rows,
            session_rows,
        ))
    }

    fn build_workspace_active_task_summaries_from_rows(
        seeds: Vec<(Task, DateTime<Utc>)>,
        session_rows: Vec<SessionSnapshotRow>,
    ) -> Vec<WorkspaceActiveTaskSummary> {
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

            let Some(primary_idx) = primary_idx else {
                continue;
            };
            let primary_summary = task_sessions.remove(primary_idx);
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

            summaries.push(WorkspaceActiveTaskSummary {
                task,
                primary_session: primary_summary,
                primary_session_head: None,
                sessions,
                sort_at,
            });
        }

        summaries
    }

    pub async fn list_messages_for_session(&self, session_id: SessionId) -> Result<Vec<Message>> {
        let rows = self.query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at
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
            let order_seq: Option<i64> = r.try_get("order_seq")?;
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
                order_seq,
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
        let row = self.query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at
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
            let order_seq: Option<i64> = r.try_get("order_seq").ok()?;
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
                order_seq,
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
        let count: i64 = self
            .query_scalar(r#"SELECT COUNT(*) FROM messages WHERE session_id = ? AND role = 'user'"#)
            .bind(session_id.0.to_string())
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    pub async fn get_first_user_message_content(
        &self,
        session_id: SessionId,
    ) -> Result<Option<String>> {
        let row = self
            .query(
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

    pub(super) async fn list_messages_for_turns(
        &self,
        session_id: SessionId,
        turn_ids: &[TurnId],
    ) -> Result<Vec<Message>> {
        let mut sql = String::from(
            "SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at
             FROM messages
             WHERE session_id = ?",
        );
        if turn_ids.is_empty() {
            sql.push_str(" AND delivery = 'queued' AND delivered_at IS NULL");
        } else {
            sql.push_str(" AND (turn_id IN (");
            for i in 0..turn_ids.len() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push('?');
            }
            sql.push_str(") OR (delivery = 'queued' AND delivered_at IS NULL))");
        }
        sql.push_str(" ORDER BY created_at ASC, turn_sequence ASC");

        let sql = self.rewrite_sql(&sql);
        let mut query = sqlx::query(sql.as_ref()).bind(session_id.0.to_string());
        if !turn_ids.is_empty() {
            for turn_id in turn_ids {
                query = query.bind(turn_id.0.to_string());
            }
        }
        let rows = query.fetch_all(&self.pool).await?;
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
            let order_seq: Option<i64> = r.try_get("order_seq")?;
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
                order_seq,
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

    pub(super) async fn build_workspace_task_summaries(
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
            let mut session_sql = String::from(
                "
                SELECT id, task_id, workspace_id, parent_session_id, relationship,
                       execution_environment, provider_id, model_id, title, status, created_at, updated_at
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
            for i in 0..task_ids.len() {
                if i > 0 {
                    session_sql.push_str(", ");
                }
                session_sql.push('?');
            }
            session_sql.push_str(")) WHERE rn <= ? ORDER BY task_id, rn");

            let session_sql = self.rewrite_sql(&session_sql);
            let mut session_query = sqlx::query(session_sql.as_ref());
            for task_id in &task_ids {
                session_query = session_query.bind(task_id.0.to_string());
            }
            session_query = session_query.bind(SESSION_LIMIT);
            let session_rows = session_query.fetch_all(&self.pool).await?;

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
                    execution_environment: parse_execution_environment(
                        r.try_get::<String, _>("execution_environment")?.as_str(),
                    ),
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
    pub(super) async fn list_session_snapshot_rows(
        &self,
        task_ids: &[TaskId],
    ) -> Result<Vec<SessionSnapshotRow>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut session_sql = String::from(
            r#"
            SELECT
                s.id,
                s.task_id,
                s.workspace_id,
                s.worktree_id,
                s.execution_environment,
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
        for i in 0..task_ids.len() {
            if i > 0 {
                session_sql.push_str(", ");
            }
            session_sql.push('?');
        }
        session_sql.push_str(") ORDER BY s.created_at ASC");

        let session_sql = self.rewrite_sql(&session_sql);
        let mut session_query = sqlx::query(session_sql.as_ref());
        for task_id in task_ids {
            session_query = session_query.bind(task_id.0.to_string());
        }
        let session_rows = session_query.fetch_all(&self.pool).await?;
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
                execution_environment: parse_execution_environment(
                    r.try_get::<String, _>("execution_environment")?.as_str(),
                ),
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

    pub(super) async fn list_session_snapshot_rows_base(
        &self,
        task_ids: &[TaskId],
    ) -> Result<Vec<SessionSnapshotRow>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut session_sql = String::from(
            r#"
            WITH session_scope AS (
                SELECT id
                FROM sessions
                WHERE task_id IN ("#,
        );
        for i in 0..task_ids.len() {
            if i > 0 {
                session_sql.push_str(", ");
            }
            session_sql.push('?');
        }
        session_sql.push_str(
            r#")
            ),
            last_messages AS (
                SELECT
                    m.session_id,
                    m.content,
                    m.created_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY m.session_id
                        ORDER BY m.created_at DESC,
                                 COALESCE(m.turn_sequence, -1) DESC,
                                 m.id DESC
                    ) AS rn
                FROM messages m
                JOIN session_scope ss ON ss.id = m.session_id
                WHERE m.role IN ('assistant', 'user')
            ),
            last_events AS (
                SELECT e.session_id, MAX(e.seq) AS last_event_seq
                FROM session_events e
                JOIN session_scope ss ON ss.id = e.session_id
                GROUP BY e.session_id
            ),
            last_turns AS (
                SELECT
                    t.session_id,
                    t.status,
                    t.start_seq,
                    ROW_NUMBER() OVER (
                        PARTITION BY t.session_id
                        ORDER BY COALESCE(t.start_seq, -1) DESC,
                                 t.started_at DESC,
                                 t.turn_id DESC
                    ) AS rn
                FROM session_turns t
                JOIN session_scope ss ON ss.id = t.session_id
            ),
            running_turns AS (
                SELECT t.session_id, COUNT(*) AS running_count
                FROM session_turns t
                JOIN session_scope ss ON ss.id = t.session_id
                WHERE t.status = 'running'
                GROUP BY t.session_id
            )
            SELECT
                s.id,
                s.task_id,
                s.workspace_id,
                s.worktree_id,
                s.execution_environment,
                s.parent_session_id,
                s.relationship,
                s.provider_id,
                s.model_id,
                s.title,
                s.agent_role,
                s.status,
                s.provider_session_ref,
                s.created_at,
                s.updated_at,
                lm.content AS last_message_content,
                lm.created_at AS last_message_at,
                le.last_event_seq AS last_event_seq,
                lt.status AS last_turn_status,
                COALESCE(rt.running_count, 0) AS running_turn_count
            FROM sessions s
            JOIN session_scope ss ON ss.id = s.id
            LEFT JOIN last_messages lm ON lm.session_id = s.id AND lm.rn = 1
            LEFT JOIN last_events le ON le.session_id = s.id
            LEFT JOIN last_turns lt ON lt.session_id = s.id AND lt.rn = 1
            LEFT JOIN running_turns rt ON rt.session_id = s.id
            ORDER BY s.created_at ASC"#,
        );

        let session_sql = self.rewrite_sql(&session_sql);
        let mut session_query = sqlx::query(session_sql.as_ref());
        for task_id in task_ids {
            session_query = session_query.bind(task_id.0.to_string());
        }
        let session_rows = session_query.fetch_all(&self.pool).await?;
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
                execution_environment: parse_execution_environment(
                    r.try_get::<String, _>("execution_environment")?.as_str(),
                ),
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

    pub(super) async fn get_session_snapshot_summary(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSnapshotSummary>> {
        let row = self
            .query(
                r#"
            SELECT
                s.id,
                s.task_id,
                s.workspace_id,
                s.worktree_id,
                s.execution_environment,
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
            execution_environment: parse_execution_environment(
                r.try_get::<String, _>("execution_environment")?.as_str(),
            ),
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
        let rows = self.query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at
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
            let order_seq: Option<i64> = r.try_get("order_seq")?;
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
                order_seq,
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
        let row = self.query(
            r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content, attachments_json, delivery, delivered_at, created_at
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
            let order_seq: Option<i64> = r.try_get("order_seq").ok()?;
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
                order_seq,
                role: parse_message_role(r.try_get::<String, _>("role").ok()?.as_str()),
                content: r.try_get("content").ok()?,
                attachments,
                delivery: parse_message_delivery(r.try_get::<String, _>("delivery").ok()?.as_str()),
                delivered_at: delivered_at.as_deref().map(parse_dt).transpose().ok()?,
                created_at: parse_dt(&created_at).ok()?,
            })
        }))
    }

    pub async fn list_message_ids(&self) -> Result<Vec<MessageId>> {
        let rows = self
            .query(r#"SELECT id FROM messages ORDER BY created_at ASC"#)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            out.push(MessageId(uuid::Uuid::parse_str(&id)?));
        }
        Ok(out)
    }

    pub async fn delete_message(&self, id: MessageId) -> Result<()> {
        self.query(r#"DELETE FROM messages WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_message_delivered(&self, id: MessageId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let delivery = "immediate";
        let write_bytes = bytes_str(delivery) + bytes_str(&now);
        let result = self
            .query(
                r#"UPDATE messages
               SET delivery = 'immediate', delivered_at = ?
               WHERE id = ?"#,
            )
            .bind(now)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::Messages,
            result.rows_affected(),
            write_bytes,
        );
        Ok(())
    }
}
