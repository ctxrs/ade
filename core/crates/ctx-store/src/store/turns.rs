use super::*;

pub struct SessionTurnToolCountDeltas {
    pub total: i64,
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}

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
        self.schedule_active_snapshot_head_refresh(turn.session_id, None)
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
        self.schedule_active_snapshot_head_refresh(session_id, None)
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
        self.schedule_active_snapshot_head_refresh(session_id, None)
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
        self.schedule_active_snapshot_head_refresh(session_id, None)
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
                r#"SELECT session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle, status, input_json,
                      output_text, order_seq, first_event_seq, input_truncated, input_original_bytes,
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
            r#"SELECT session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle, status, input_json,
                      output_text, order_seq, first_event_seq, input_truncated, input_original_bytes,
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
                r#"SELECT session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle, status, input_json,
                      output_text, order_seq, first_event_seq, input_truncated, input_original_bytes,
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
            + bytes_opt_str(tool.provider_tool_name.as_deref())
            + bytes_opt_str(tool.title.as_deref())
            + bytes_opt_str(tool.subtitle.as_deref())
            + bytes_opt_str(tool.status.as_deref())
            + bytes_opt_str(input_json.as_deref())
            + bytes_opt_str(tool.output_text.as_deref())
            + I64_BYTES
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
                    session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle, status, input_json,
                    output_text, order_seq, first_event_seq, input_truncated, input_original_bytes, output_truncated,
                    output_original_bytes, created_at, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id, tool_call_id) DO UPDATE SET
                   turn_id = excluded.turn_id,
                   tool_kind = COALESCE(excluded.tool_kind, session_turn_tools.tool_kind),
                   provider_tool_name = COALESCE(excluded.provider_tool_name, session_turn_tools.provider_tool_name),
                   title = COALESCE(excluded.title, session_turn_tools.title),
                   subtitle = COALESCE(excluded.subtitle, session_turn_tools.subtitle),
                   status = CASE WHEN excluded.status IS NULL THEN session_turn_tools.status
                       WHEN session_turn_tools.status IN ('completed', 'failed')
                            AND excluded.status IN ('pending', 'in_progress') THEN session_turn_tools.status
                       ELSE excluded.status END,
                   input_json = COALESCE(excluded.input_json, session_turn_tools.input_json),
                   output_text = COALESCE(excluded.output_text, session_turn_tools.output_text),
                   order_seq = COALESCE(MIN(session_turn_tools.order_seq, excluded.order_seq), excluded.order_seq),
                   first_event_seq = COALESCE(session_turn_tools.first_event_seq, excluded.first_event_seq),
                   input_truncated = COALESCE(excluded.input_truncated, session_turn_tools.input_truncated), input_original_bytes = COALESCE(excluded.input_original_bytes, session_turn_tools.input_original_bytes),
                   output_truncated = COALESCE(excluded.output_truncated, session_turn_tools.output_truncated), output_original_bytes = COALESCE(excluded.output_original_bytes, session_turn_tools.output_original_bytes),
                   updated_at = excluded.updated_at"#,
        )
        .bind(&session_id)
        .bind(&tool.tool_call_id)
        .bind(&turn_id)
        .bind(tool.tool_kind.as_deref())
        .bind(tool.provider_tool_name.as_deref())
        .bind(tool.title.as_deref())
        .bind(tool.subtitle.as_deref())
        .bind(tool.status.as_deref())
        .bind(input_json)
        .bind(tool.output_text.as_deref())
        .bind(tool.order_seq)
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
