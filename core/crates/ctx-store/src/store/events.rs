use super::*;

fn is_terminal_session_event(event_type: &SessionEventType) -> bool {
    matches!(event_type, SessionEventType::TurnFinished)
}

impl Store {
    // Session event APIs
    pub(super) async fn upsert_event_log_checkpoint(
        &self,
        checkpoint_seq: i64,
        payload: Option<Value>,
    ) -> Result<()> {
        let payload_json = payload.map(|value| value.to_string());
        let now = Utc::now().to_rfc3339();
        self.query(
            r#"INSERT INTO event_log_checkpoints
               (id, checkpoint_seq, payload_json, created_at, updated_at)
               VALUES (1, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   checkpoint_seq = excluded.checkpoint_seq,
                   payload_json = COALESCE(excluded.payload_json, event_log_checkpoints.payload_json),
                   updated_at = excluded.updated_at"#,
        )
        .bind(checkpoint_seq)
        .bind(payload_json.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn append_session_event(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> Result<SessionEvent> {
        crate::fault_injection::maybe_fail("ctx_store.append_session_event")?;
        let payload_json = if matches!(
            event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            sanitize_tool_event_payload(&event_type, &payload_json)
        } else {
            payload_json
        };
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
        if matches!(
            event.event_type,
            SessionEventType::AssistantChunk
                | SessionEventType::ThoughtChunk
                | SessionEventType::ToolCallUpdate
        ) {
            event.seq = next_stream_only_event_seq();
            event.transient = true;
            return Ok(event);
        }
        event.seq = self.event_log.next_seq();
        if let Err(err) = self.event_log.enqueue(event.clone()).await {
            tracing::warn!("event log enqueue failed, falling back to sync persist: {err:#}");
            self.persist_session_events_batch(std::slice::from_ref(&event))
                .await?;
        }
        Ok(event)
    }

    pub(super) async fn flush_event_log_for_reads(&self) {
        if let Err(err) = self.event_log.flush().await {
            tracing::warn!("event log flush failed before read: {err:#}");
        }
    }

    pub async fn flush_session_event_log(&self) -> Result<()> {
        self.event_log.flush().await
    }

    pub async fn persist_turn_terminal_events(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        turn_id: TurnId,
        events: Vec<(SessionEventType, Value)>,
    ) -> Result<Vec<SessionEvent>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        self.flush_event_log_for_reads().await;

        let mut persisted = Vec::with_capacity(events.len());
        for (event_type, payload_json) in events {
            persisted.push(SessionEvent {
                seq: self.event_log.next_seq(),
                id: SessionEventId::new(),
                session_id,
                run_id,
                turn_id: Some(turn_id),
                event_type,
                payload_json,
                transient: false,
                created_at: Utc::now(),
            });
        }

        let max_seq = persisted
            .iter()
            .map(|event| event.seq)
            .max()
            .unwrap_or_default();

        {
            let _write_guard = self.write_gate.lock().await;
            let mut tx = self.pool.begin().await?;

            let status_row = sqlx::query(
                r#"SELECT status
                   FROM session_turns
                   WHERE session_id = ? AND turn_id = ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .fetch_optional(&mut *tx)
            .await?;

            let Some(status_row) = status_row else {
                tx.rollback().await?;
                return Ok(Vec::new());
            };

            let current_status: String = status_row.try_get("status")?;
            if matches!(
                parse_session_turn_status(&current_status),
                SessionTurnStatus::Completed
                    | SessionTurnStatus::Failed
                    | SessionTurnStatus::Interrupted
            ) {
                tx.rollback().await?;
                return Ok(Vec::new());
            }

            let mut builder = sqlx::QueryBuilder::<Sqlite>::new(
                "INSERT INTO session_events (seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at) ",
            );
            builder.push_values(persisted.iter(), |mut b, event| {
                b.push_bind(event.seq)
                    .push_bind(event.id.0.to_string())
                    .push_bind(event.session_id.0.to_string())
                    .push_bind(event.run_id.map(|id| id.0.to_string()))
                    .push_bind(event.turn_id.map(|id| id.0.to_string()))
                    .push_bind(session_event_type_to_str(&event.event_type))
                    .push_bind(event.payload_json.to_string())
                    .push_bind(if event.transient { 1 } else { 0 })
                    .push_bind(event.created_at.to_rfc3339());
            });
            builder.build().execute(&mut *tx).await?;

            self.update_session_snapshot_last_event_seq_tx(&mut tx, session_id, max_seq)
                .await?;
            let _ = self
                .repair_session_turn_projection_from_events_tx(&mut tx, session_id, turn_id)
                .await?;
            self.refresh_session_turn_summary_tx(&mut tx, session_id)
                .await?;
            tx.commit().await?;
        }

        for event in &persisted {
            let write_bytes = bytes_str(&event.id.0.to_string())
                + bytes_str(&event.session_id.0.to_string())
                + bytes_opt_str(event.run_id.map(|id| id.0.to_string()).as_deref())
                + bytes_opt_str(event.turn_id.map(|id| id.0.to_string()).as_deref())
                + bytes_str(session_event_type_to_str(&event.event_type))
                + bytes_str(&event.payload_json.to_string())
                + bytes_str(&event.created_at.to_rfc3339())
                + BOOL_BYTES;
            record_write(WriteMetricTable::SessionEvents, 1, write_bytes);
        }

        if let Err(err) = self
            .update_active_snapshot_head_last_event_seq(session_id, max_seq)
            .await
        {
            tracing::warn!(
                "failed to update active snapshot head last_event_seq for {}: {err:#}",
                session_id.0
            );
        }

        if let Err(err) = self
            .schedule_active_snapshot_head_refresh(session_id, Some(max_seq))
            .await
        {
            tracing::warn!(
                "failed to refresh active snapshot head for {}: {err:#}",
                session_id.0
            );
        }

        Ok(persisted)
    }

    pub(super) async fn persist_session_events_batch(&self, events: &[SessionEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        struct EventInsertRow {
            seq: i64,
            id: String,
            session_id: String,
            run_id: Option<String>,
            turn_id: Option<String>,
            event_type: &'static str,
            payload_text: String,
            transient: i64,
            created_at: String,
            write_bytes: u64,
        }

        let mut rows = Vec::with_capacity(events.len());
        let mut max_seq_by_session: HashMap<SessionId, i64> = HashMap::new();
        let mut refresh_sessions: HashSet<SessionId> = HashSet::new();
        let mut terminal_turns: HashSet<(SessionId, TurnId)> = HashSet::new();
        let mut summary_refresh_sessions: HashSet<SessionId> = HashSet::new();

        for event in events {
            let id = event.id.0.to_string();
            let session_id = event.session_id.0.to_string();
            let run_id = event.run_id.map(|r| r.0.to_string());
            let turn_id = event.turn_id.map(|t| t.0.to_string());
            let event_type = session_event_type_to_str(&event.event_type);
            let payload_text = event.payload_json.to_string();
            let created_at = event.created_at.to_rfc3339();
            let write_bytes = bytes_str(&id)
                + bytes_str(&session_id)
                + bytes_opt_str(run_id.as_deref())
                + bytes_opt_str(turn_id.as_deref())
                + bytes_str(event_type)
                + bytes_str(&payload_text)
                + bytes_str(&created_at)
                + BOOL_BYTES;
            rows.push(EventInsertRow {
                seq: event.seq,
                id,
                session_id,
                run_id,
                turn_id,
                event_type,
                payload_text,
                transient: if event.transient { 1 } else { 0 },
                created_at,
                write_bytes,
            });

            max_seq_by_session
                .entry(event.session_id)
                .and_modify(|current| {
                    *current = (*current).max(event.seq);
                })
                .or_insert(event.seq);

            if let Some(turn_id) = event.turn_id {
                if is_terminal_session_event(&event.event_type) {
                    terminal_turns.insert((event.session_id, turn_id));
                    summary_refresh_sessions.insert(event.session_id);
                    refresh_sessions.insert(event.session_id);
                }
                if let Some(tool) = build_turn_tool_from_event(event, turn_id) {
                    let _ = self.upsert_session_turn_tool(tool).await;
                    refresh_sessions.insert(event.session_id);
                }
            }
        }

        {
            let _write_guard = self.write_gate.lock().await;
            let mut tx = self.pool.begin().await?;
            let mut builder = sqlx::QueryBuilder::<Sqlite>::new(
                "INSERT INTO session_events (seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at) ",
            );
            builder.push_values(rows.iter(), |mut b, row| {
                b.push_bind(row.seq)
                    .push_bind(&row.id)
                    .push_bind(&row.session_id)
                    .push_bind(row.run_id.as_deref())
                    .push_bind(row.turn_id.as_deref())
                    .push_bind(row.event_type)
                    .push_bind(&row.payload_text)
                    .push_bind(row.transient)
                    .push_bind(&row.created_at);
            });
            builder.build().execute(&mut *tx).await?;
            for (session_id, seq) in &max_seq_by_session {
                self.update_session_snapshot_last_event_seq_tx(&mut tx, *session_id, *seq)
                    .await?;
            }
            for (session_id, turn_id) in &terminal_turns {
                if self
                    .repair_session_turn_projection_from_events_tx(&mut tx, *session_id, *turn_id)
                    .await?
                {
                    summary_refresh_sessions.insert(*session_id);
                }
            }
            for session_id in &summary_refresh_sessions {
                self.refresh_session_turn_summary_tx(&mut tx, *session_id)
                    .await?;
            }
            tx.commit().await?;
        }

        for row in rows {
            record_write(WriteMetricTable::SessionEvents, 1, row.write_bytes);
        }

        for (session_id, seq) in &max_seq_by_session {
            if let Err(err) = self
                .update_active_snapshot_head_last_event_seq(*session_id, *seq)
                .await
            {
                tracing::warn!(
                    "failed to update active snapshot head last_event_seq for {}: {err:#}",
                    session_id.0
                );
            }
        }

        for session_id in refresh_sessions {
            let last_seq = max_seq_by_session.get(&session_id).copied();
            if let Err(err) = self
                .schedule_active_snapshot_head_refresh(session_id, last_seq)
                .await
            {
                tracing::warn!(
                    "failed to refresh active snapshot head for {}: {err:#}",
                    session_id.0
                );
            }
        }

        Ok(())
    }

    pub async fn list_session_events(&self, session_id: SessionId) -> Result<Vec<SessionEvent>> {
        self.list_session_events_page_by_seq(session_id, None, None, false)
            .await
    }

    pub(super) async fn session_last_event_seq(&self, session_id: SessionId) -> Result<i64> {
        self.flush_event_log_for_reads().await;
        let seq = self
            .query_scalar::<Option<i64>>(
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
        self.flush_event_log_for_reads().await;
        let session_id_str = session_id.0.to_string();
        let limit_i64 = limit.map(|n| n as i64);
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = if let Some(after_seq) = after_seq {
            if let Some(limit) = limit_i64 {
                self.query(
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
                self.query(
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
            self.query(
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
            self.query(
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
        self.flush_event_log_for_reads().await;
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = self.query(
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
        self.flush_event_log_for_reads().await;
        let row = self.query(
            r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
               FROM session_events
               WHERE session_id = ? AND run_id = ? AND event_type IN ('done', 'error', 'turn_interrupted', 'turn_finished')
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
        let sql = self.rewrite_sql(&sql);
        let mut q = sqlx::query(sql.as_ref())
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
        self.flush_event_log_for_reads().await;
        let session_id_str = session_id.0.to_string();
        let include_transient = if include_transient { 1 } else { 0 };
        let rows = self.query(
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
}
