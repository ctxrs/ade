use super::*;

impl Store {
    // Session Turn APIs
    pub async fn insert_session_turn(&self, turn: SessionTurn) -> Result<SessionTurn> {
        let metrics_json = turn
            .metrics_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serializing turn metrics")?;
        let turn_id = turn.turn_id.0.to_string();
        let session_id = turn.session_id.0.to_string();
        let run_id = turn.run_id.map(|r| r.0.to_string());
        let user_message_id = turn.user_message_id.map(|m| m.0.to_string());
        let status = session_turn_status_to_str(&turn.status);
        let started_at = turn.started_at.to_rfc3339();
        let updated_at = turn.updated_at.to_rfc3339();
        let write_bytes = bytes_str(&turn_id)
            + bytes_str(&session_id)
            + bytes_opt_str(run_id.as_deref())
            + bytes_opt_str(user_message_id.as_deref())
            + bytes_str(status)
            + bytes_opt_i64(turn.start_seq)
            + bytes_opt_i64(turn.end_seq)
            + bytes_str(&started_at)
            + bytes_str(&updated_at)
            + bytes_opt_str(turn.assistant_partial.as_deref())
            + bytes_opt_str(turn.thought_partial.as_deref())
            + bytes_opt_str(metrics_json.as_deref())
            + (I64_BYTES * 5);
        let result = self
            .query(
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
            .bind(&turn_id)
            .bind(&session_id)
            .bind(run_id)
            .bind(user_message_id)
            .bind(status)
            .bind(turn.start_seq)
            .bind(turn.end_seq)
            .bind(&started_at)
            .bind(&updated_at)
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
        record_write(
            WriteMetricTable::SessionTurns,
            result.rows_affected(),
            write_bytes,
        );
        self.refresh_session_turn_summary(turn.session_id).await?;
        self.refresh_active_snapshot_head(turn.session_id, None)
            .await?;
        Ok(turn)
    }

    pub async fn get_session_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<Option<SessionTurn>> {
        let row = self.query(
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

    pub async fn get_session_turn_by_id(&self, turn_id: TurnId) -> Result<Option<SessionTurn>> {
        let row = self
            .query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                      start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                      metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
               FROM session_turns
               WHERE turn_id = ?"#,
            )
            .bind(turn_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| build_session_turn_from_row(r).ok()))
    }

    pub async fn get_running_turn_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionTurn>> {
        let row = self
            .query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ? AND status IN ('queued', 'running')
                   ORDER BY start_seq DESC
                   LIMIT 1"#,
            )
            .bind(session_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| build_session_turn_from_row(r).ok()))
    }

    pub async fn get_latest_turn_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionTurn>> {
        let row = self
            .query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ?
                   ORDER BY start_seq DESC
                   LIMIT 1"#,
            )
            .bind(session_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| build_session_turn_from_row(r).ok()))
    }

    pub async fn get_latest_turn_for_run(
        &self,
        session_id: SessionId,
        run_id: RunId,
    ) -> Result<Option<SessionTurn>> {
        let row = self
            .query(
                r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                          start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                          metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
                   FROM session_turns
                   WHERE session_id = ? AND run_id = ?
                   ORDER BY start_seq DESC
                   LIMIT 1"#,
            )
            .bind(session_id.0.to_string())
            .bind(run_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| build_session_turn_from_row(r).ok()))
    }

    pub async fn delete_session_turn(&self, session_id: SessionId, turn_id: TurnId) -> Result<()> {
        self.query(r#"DELETE FROM session_turns WHERE session_id = ? AND turn_id = ?"#)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .execute(&self.pool)
            .await?;
        self.refresh_session_turn_summary(session_id).await?;
        self.refresh_active_snapshot_head(session_id, None).await?;
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
        let updated_at = updated_at.to_rfc3339();
        let write_bytes = bytes_opt_str(assistant_partial)
            + bytes_opt_str(thought_partial)
            + bytes_str(&updated_at);
        let result = self
            .query(
                r#"UPDATE session_turns
               SET assistant_partial = COALESCE(?, assistant_partial),
                   thought_partial = COALESCE(?, thought_partial),
                   updated_at = ?
               WHERE session_id = ? AND turn_id = ?"#,
            )
            .bind(assistant_partial.map(|s| s.to_string()))
            .bind(thought_partial.map(|s| s.to_string()))
            .bind(&updated_at)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionTurns,
            result.rows_affected(),
            write_bytes,
        );
        self.refresh_session_turn_summary(session_id).await?;
        self.refresh_active_snapshot_head(session_id, None).await?;
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
        let status = session_turn_status_to_str(&status);
        let updated_at = updated_at.to_rfc3339();
        let write_bytes = bytes_str(status)
            + bytes_opt_i64(end_seq)
            + bytes_opt_str(metrics_json.as_deref())
            + bytes_str(&updated_at);
        let result = self
            .query(
                r#"UPDATE session_turns
               SET status = ?,
                   end_seq = COALESCE(?, end_seq),
                   metrics_json = COALESCE(?, metrics_json),
                   updated_at = ?
               WHERE session_id = ? AND turn_id = ?"#,
            )
            .bind(status)
            .bind(end_seq)
            .bind(metrics_json)
            .bind(&updated_at)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionTurns,
            result.rows_affected(),
            write_bytes,
        );
        self.refresh_session_turn_summary(session_id).await?;
        self.refresh_active_snapshot_head(session_id, None).await?;
        Ok(())
    }

    pub async fn update_session_turn_tool_counts(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        deltas: SessionTurnToolCountDeltas,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        let updated_at = updated_at.to_rfc3339();
        let write_bytes = (I64_BYTES * 5) + bytes_str(&updated_at);
        let result = self
            .query(
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
            .bind(&updated_at)
            .bind(session_id.0.to_string())
            .bind(turn_id.0.to_string())
            .execute(&self.pool)
            .await?;
        record_write(
            WriteMetricTable::SessionTurns,
            result.rows_affected(),
            write_bytes,
        );
        self.refresh_active_snapshot_head(session_id, None).await?;
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
            self.query(
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
            self.query(
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
        let mut sql = String::from(
            r#"SELECT turn_id, session_id, run_id, user_message_id, status,
                      start_seq, end_seq, started_at, updated_at, assistant_partial, thought_partial,
                      metrics_json, tool_total, tool_pending, tool_running, tool_completed, tool_failed
               FROM session_turns
               WHERE status IN ("#,
        );
        for i in 0..statuses.len() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push('?');
        }
        sql.push_str(") ORDER BY updated_at ASC");
        let sql = self.rewrite_sql(&sql);
        let mut query = sqlx::query(sql.as_ref());
        for status in statuses {
            query = query.bind(session_turn_status_to_str(status));
        }
        let rows = query.fetch_all(&self.pool).await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(turn) = build_session_turn_from_row(r) {
                out.push(turn);
            }
        }
        Ok(out)
    }

    pub(super) async fn load_session_head_materialization(
        &self,
        session_id: SessionId,
        kind: SessionHeadKind,
    ) -> Result<Option<SessionHeadMaterialization>> {
        let timing_enabled = snapshot_timing_enabled();
        let acquire_start = timing_enabled.then(Instant::now);
        let mut conn = self.pool.acquire().await?;
        let acquire_ms = acquire_start
            .map(|start| start.elapsed())
            .unwrap_or_default();
        let query_start = timing_enabled.then(Instant::now);
        let row = self
            .query(
                r#"SELECT last_event_seq, turns_json, tool_summaries_json, events_json,
                      messages_json, has_more_turns, head_window_json
               FROM session_head_materializations
               WHERE session_id = ? AND head_kind = ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(session_head_kind_to_str(kind))
            .fetch_optional(&mut *conn)
            .await?;
        let query_ms = query_start.map(|start| start.elapsed()).unwrap_or_default();

        let Some(row) = row else {
            if timing_enabled {
                info!(
                    target: "ctx_store.snapshot_timing",
                    snapshot = "session_snapshot",
                    session_id = %session_id.0,
                    head_kind = session_head_kind_to_str(kind),
                    hit = false,
                    acquire_ms = acquire_ms.as_millis(),
                    query_ms = query_ms.as_millis(),
                    parse_ms = 0,
                );
            }
            return Ok(None);
        };

        let turns_json: String = row.try_get("turns_json")?;
        let tool_summaries_json: String = row.try_get("tool_summaries_json")?;
        let events_json: String = row.try_get("events_json")?;
        let messages_json: String = row.try_get("messages_json")?;
        let head_window_json: String = row.try_get("head_window_json")?;

        let parse_start = timing_enabled.then(Instant::now);
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
        let parse_ms = parse_start.map(|start| start.elapsed()).unwrap_or_default();

        let has_more_turns: i64 = row.try_get("has_more_turns")?;

        if timing_enabled {
            info!(
                target: "ctx_store.snapshot_timing",
                snapshot = "session_snapshot",
                session_id = %session_id.0,
                head_kind = session_head_kind_to_str(kind),
                hit = true,
                turns = turns.len(),
                tool_summaries = tool_summaries.len(),
                events = events.len(),
                messages = messages.len(),
                acquire_ms = acquire_ms.as_millis(),
                query_ms = query_ms.as_millis(),
                parse_ms = parse_ms.as_millis(),
            );
        }

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

    pub(super) async fn update_active_snapshot_head_last_event_seq(
        &self,
        session_id: SessionId,
        last_event_seq: i64,
    ) -> Result<()> {
        let _ = session_id;
        let _ = last_event_seq;
        // Active snapshot head projections are in-memory only.
        Ok(())
    }

    pub(super) async fn refresh_active_snapshot_head(
        &self,
        session_id: SessionId,
        last_event_seq: Option<i64>,
    ) -> Result<()> {
        let _ = session_id;
        let _ = last_event_seq;
        // Active snapshot head projections are in-memory only.
        Ok(())
    }

    pub(super) async fn delete_active_snapshot_heads_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<()> {
        let _ = task_id;
        // Active snapshot head projections are in-memory only.
        Ok(())
    }

    pub(super) async fn refresh_active_snapshot_heads_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<()> {
        let _ = task_id;
        // Active snapshot head projections are in-memory only.
        Ok(())
    }

    pub(super) async fn upsert_session_head_materialization(
        &self,
        session_id: SessionId,
        kind: SessionHeadKind,
        head: &SessionHeadMaterialization,
    ) -> Result<i64> {
        if disable_head_materialization_writes_for(kind) {
            return Ok(0);
        }
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
        let session_id = session_id.0.to_string();
        let head_kind = session_head_kind_to_str(kind);
        let write_bytes = bytes_str(&session_id)
            + bytes_str(head_kind)
            + I64_BYTES
            + bytes_str(&turns_json)
            + bytes_str(&tool_summaries_json)
            + bytes_str(&events_json)
            + bytes_str(&messages_json)
            + BOOL_BYTES
            + bytes_str(&head_window_json)
            + bytes_str(&now)
            + bytes_str(&now);

        let head_rev: i64 = self
            .query_scalar(
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
            .bind(&session_id)
            .bind(head_kind)
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
        record_write(
            WriteMetricTable::SessionHeadMaterializations,
            1,
            write_bytes,
        );

        Ok(head_rev)
    }

    pub(super) async fn session_head_kind_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<SessionHeadKind> {
        let archived_at: Option<Option<String>> = self
            .query_scalar(r#"SELECT archived_at FROM tasks WHERE id = ?"#)
            .bind(task_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(if archived_at.flatten().is_some() {
            SessionHeadKind::Archived
        } else {
            SessionHeadKind::Active
        })
    }

    pub(super) async fn materialize_session_head(
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

    pub(super) async fn delete_session_head_materializations_for_task(
        &self,
        task_id: TaskId,
        kind: SessionHeadKind,
    ) -> Result<()> {
        if disable_head_materialization_writes_for(kind) {
            return Ok(());
        }
        self.query(
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

    pub(super) async fn materialize_archived_heads_for_task(&self, task_id: TaskId) -> Result<()> {
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

    pub(super) async fn build_session_head(
        &self,
        session: &Session,
        limits: SessionHeadLimits,
        include_events: bool,
        last_event_seq: i64,
    ) -> Result<SessionHead> {
        let limit = limits.turn_limit as i64;
        let rows = self.query(
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
            tool_summaries.sort_by(compare_tool_summary_order);
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

        strip_snapshot_partials(&mut out, &mut events);
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
        crate::fault_injection::maybe_fail("ctx_store.get_session_head_snapshot")?;
        let head = self
            .get_session_head(session_id, limit, include_events)
            .await?;
        Ok(head.map(session_head_to_snapshot))
    }

    pub async fn get_active_snapshot_head(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionHeadSnapshot>> {
        crate::fault_injection::maybe_fail("ctx_store.get_active_snapshot_head")?;
        let session = match self.get_session(session_id).await? {
            Some(session) => session,
            None => return Ok(None),
        };
        if !matches!(
            self.session_head_kind_for_task(session.task_id).await?,
            SessionHeadKind::Active
        ) {
            return Ok(None);
        }
        let last_event_seq = self.session_last_event_seq(session_id).await?;
        let limits = session_head_limits(SessionHeadKind::Active, ACTIVE_SNAPSHOT_HEAD_LIMIT);
        let mut head = self
            .build_session_head(&session, limits, false, last_event_seq)
            .await?;
        strip_snapshot_partials(&mut head.turns, &mut head.events);
        Ok(Some(session_head_to_snapshot(head)))
    }

    pub(super) async fn get_session_head_with_kind(
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

        let turn_limit = match head_kind {
            SessionHeadKind::Active => SESSION_HEAD_MAX_TURNS,
            SessionHeadKind::Archived => SESSION_HEAD_ARCHIVED_TURN_LIMIT,
        };
        let materialize_limits = session_head_limits(head_kind, turn_limit);
        let head = self
            .build_session_head(&session, materialize_limits, true, last_event_seq)
            .await?;
        if !disable_head_materialization_writes_for(head_kind) {
            let store = self.clone();
            let session_id = session.id;
            let materialized = SessionHeadMaterialization::from_head(&head);
            tokio::spawn(async move {
                if let Err(err) = store
                    .upsert_session_head_materialization(session_id, head_kind, &materialized)
                    .await
                {
                    tracing::warn!(
                        session_id = %session_id.0,
                        "failed to persist session head materialization: {err:#}"
                    );
                }
            });
        }
        let limits = session_head_limits(head_kind, limit);
        Ok(Some(apply_session_head_limits(
            head,
            limits,
            include_events,
        )))
    }

    pub async fn refresh_active_session_head_projection(
        &self,
        session_id: SessionId,
    ) -> Result<bool> {
        if disable_head_materialization_writes_for(SessionHeadKind::Active) {
            return Ok(false);
        }
        let session = match self.get_session(session_id).await? {
            Some(session) => session,
            None => return Ok(false),
        };
        if !matches!(
            self.session_head_kind_for_task(session.task_id).await?,
            SessionHeadKind::Active
        ) {
            return Ok(false);
        }
        let last_event_seq = self.session_last_event_seq(session_id).await?;
        if let Some(materialized) = self
            .load_session_head_materialization(session_id, SessionHeadKind::Active)
            .await?
        {
            if materialized.last_event_seq == last_event_seq {
                return Ok(false);
            }
        }
        let _ = self
            .materialize_session_head(&session, SessionHeadKind::Active, last_event_seq)
            .await?;
        Ok(true)
    }

    pub async fn get_session_snapshot(
        &self,
        session_id: SessionId,
        _limit: u32,
        _include_events: bool,
    ) -> Result<Option<SessionSnapshot>> {
        let summary = match self.get_session_snapshot_summary(session_id).await? {
            Some(summary) => summary,
            None => return Ok(None),
        };
        let state = self.get_session_state(session_id).await?;
        Ok(Some(SessionSnapshot {
            summary,
            head: None,
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
            self.query(
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
            self.query(
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
        let rows = self
            .query(
                r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      output_text, first_event_seq, input_truncated, input_original_bytes,
                      output_truncated, output_original_bytes, created_at, updated_at
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
            out.sort_by(compare_tool_order);
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
        let mut sql = String::from(
            r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      output_text, first_event_seq, input_truncated, input_original_bytes,
                      output_truncated, output_original_bytes, created_at, updated_at
               FROM session_turn_tools
               WHERE session_id = ? AND turn_id IN ("#,
        );
        for i in 0..turn_ids.len() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push('?');
        }
        sql.push_str(") ORDER BY created_at ASC");
        let sql = self.rewrite_sql(&sql);
        let mut query = sqlx::query(sql.as_ref()).bind(session_id.0.to_string());
        for turn_id in turn_ids {
            query = query.bind(turn_id.0.to_string());
        }
        let rows = query.fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            if let Ok(tool) = build_session_turn_tool_summary_from_row(r) {
                out.push(tool);
            }
        }
        out.sort_by(compare_tool_summary_order);
        Ok(out)
    }

    pub async fn get_session_turn_tool(
        &self,
        session_id: SessionId,
        tool_call_id: &str,
    ) -> Result<Option<SessionTurnTool>> {
        let row = self
            .query(
                r#"SELECT session_id, tool_call_id, turn_id, tool_kind, title, status, input_json,
                      output_text, first_event_seq, input_truncated, input_original_bytes,
                      output_truncated, output_original_bytes, created_at, updated_at
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
        if disable_tool_summary_persistence() {
            return Ok(tool);
        }
        let input_json = tool
            .input_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serializing tool input")?;
        let input_truncated = tool.input_truncated.map(|value| if value { 1 } else { 0 });
        let output_truncated = tool.output_truncated.map(|value| if value { 1 } else { 0 });
        let session_id = tool.session_id.0.to_string();
        let turn_id = tool.turn_id.0.to_string();
        let created_at = tool.created_at.to_rfc3339();
        let updated_at = tool.updated_at.to_rfc3339();
        let write_bytes = bytes_str(&session_id)
            + bytes_str(&tool.tool_call_id)
            + bytes_str(&turn_id)
            + bytes_opt_str(tool.tool_kind.as_deref())
            + bytes_opt_str(tool.title.as_deref())
            + bytes_opt_str(tool.status.as_deref())
            + bytes_opt_str(input_json.as_deref())
            + bytes_opt_str(tool.output_text.as_deref())
            + bytes_opt_i64(tool.first_event_seq)
            + if input_truncated.is_some() {
                BOOL_BYTES
            } else {
                0
            }
            + bytes_opt_i64(tool.input_original_bytes)
            + if output_truncated.is_some() {
                BOOL_BYTES
            } else {
                0
            }
            + bytes_opt_i64(tool.output_original_bytes)
            + bytes_str(&created_at)
            + bytes_str(&updated_at);
        let result = self.query(
            r#"INSERT INTO session_turn_tools (
                    session_id, tool_call_id, turn_id, tool_kind, title, status,
                    input_json, output_text, first_event_seq, input_truncated, input_original_bytes,
                    output_truncated, output_original_bytes, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id, tool_call_id) DO UPDATE SET
                   turn_id = excluded.turn_id,
                   tool_kind = COALESCE(excluded.tool_kind, session_turn_tools.tool_kind),
                   title = COALESCE(excluded.title, session_turn_tools.title),
                   status = COALESCE(excluded.status, session_turn_tools.status),
                   input_json = COALESCE(excluded.input_json, session_turn_tools.input_json),
                   output_text = COALESCE(excluded.output_text, session_turn_tools.output_text),
                   first_event_seq = COALESCE(session_turn_tools.first_event_seq, excluded.first_event_seq),
                   input_truncated = COALESCE(excluded.input_truncated, session_turn_tools.input_truncated),
                   input_original_bytes = COALESCE(excluded.input_original_bytes, session_turn_tools.input_original_bytes),
                   output_truncated = COALESCE(excluded.output_truncated, session_turn_tools.output_truncated),
                   output_original_bytes = COALESCE(excluded.output_original_bytes, session_turn_tools.output_original_bytes),
                   updated_at = excluded.updated_at"#,
        )
        .bind(&session_id)
        .bind(&tool.tool_call_id)
        .bind(&turn_id)
        .bind(tool.tool_kind.as_deref())
        .bind(tool.title.as_deref())
        .bind(tool.status.as_deref())
        .bind(input_json)
        .bind(tool.output_text.as_deref())
        .bind(tool.first_event_seq)
        .bind(input_truncated)
        .bind(tool.input_original_bytes)
        .bind(output_truncated)
        .bind(tool.output_original_bytes)
        .bind(&created_at)
        .bind(&updated_at)
        .execute(&self.pool)
        .await?;
        record_write(
            WriteMetricTable::SessionTurnTools,
            result.rows_affected(),
            write_bytes,
        );
        Ok(tool)
    }
}
