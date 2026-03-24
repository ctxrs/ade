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
        let mut tx = self.pool.begin().await?;
        let message_rows_affected = sqlx::query(
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
        .execute(&mut *tx)
        .await?
        .rows_affected();
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
        .execute(&mut *tx)
        .await?;
        let session_snapshot_write_bytes =
            if matches!(message.role, MessageRole::Assistant | MessageRole::User) {
                let session_snapshot_id = message.session_id.0.to_string();
                let session_snapshot_created_at = message.created_at.to_rfc3339();
                let session_snapshot_content = message.content.clone();
                let now = Utc::now().to_rfc3339();
                let ensure_write_bytes =
                    bytes_str(&session_snapshot_id) + I64_BYTES + (bytes_str(&now) * 2);
                sqlx::query(
                    r#"INSERT INTO session_snapshot_summaries (
                            session_id, running_turn_count, created_at, updated_at
                       )
                       VALUES (?, 0, ?, ?)
                       ON CONFLICT(session_id) DO NOTHING"#,
                )
                .bind(&session_snapshot_id)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
                let update_write_bytes = bytes_str(&session_snapshot_created_at) * 2
                    + bytes_str(&session_snapshot_content);
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
                           projection_rev = projection_rev + 1,
                           updated_at = ?
                       WHERE session_id = ?"#,
                )
                .bind(&session_snapshot_created_at)
                .bind(&session_snapshot_created_at)
                .bind(&session_snapshot_created_at)
                .bind(&session_snapshot_content)
                .bind(&session_snapshot_created_at)
                .bind(&session_snapshot_id)
                .execute(&mut *tx)
                .await?;
                ensure_write_bytes + update_write_bytes
            } else {
                0
            };
        if let Err(err) =
            crate::fault_injection::maybe_fail("ctx_store.insert_message.after_insert")
        {
            return Err(anyhow::anyhow!(
                "database is locked (fault injection): {err}"
            ));
        }
        tx.commit().await?;

        record_write(
            WriteMetricTable::Messages,
            message_rows_affected,
            write_bytes,
        );
        if session_snapshot_write_bytes > 0 {
            record_write(
                WriteMetricTable::SessionSnapshotSummaries,
                1,
                session_snapshot_write_bytes,
            );
        }
        self.schedule_active_snapshot_head_refresh(message.session_id, None)
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
                   projection_rev = projection_rev + 1,
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
                   projection_rev = projection_rev + 1,
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

    pub async fn get_session_projection_rev(&self, session_id: SessionId) -> Result<i64> {
        self.ensure_session_snapshot_summary(session_id).await?;
        let session_id = session_id.0.to_string();
        let projection_rev = self
            .query_scalar::<Option<i64>>(
                r#"SELECT projection_rev
                   FROM session_snapshot_summaries
                   WHERE session_id = ?"#,
            )
            .bind(&session_id)
            .fetch_optional(&self.pool)
            .await?
            .flatten()
            .unwrap_or(0);
        Ok(projection_rev)
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
