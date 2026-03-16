use super::*;

impl Store {
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
                r#"SELECT head_rev, last_event_seq, turns_json, tool_summaries_json, events_json,
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
            head_rev: row.try_get("head_rev")?,
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
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(session_id, head_kind) DO UPDATE SET
                   head_rev = excluded.head_rev,
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
            .bind(head.head_rev)
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
        let projection_rev = self.get_session_projection_rev(session.id).await?;
        let head = self
            .build_session_head(session, limits, true, last_event_seq, projection_rev)
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
        projection_rev: i64,
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
            projection_rev,
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
        let projection_rev = self.get_session_projection_rev(session_id).await?;
        let limits = session_head_limits(SessionHeadKind::Active, ACTIVE_SNAPSHOT_HEAD_LIMIT);
        let mut head = self
            .build_session_head(&session, limits, false, last_event_seq, projection_rev)
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
        let projection_rev = self.get_session_projection_rev(session_id).await?;

        if let Some(materialized) = self
            .load_session_head_materialization(session_id, head_kind)
            .await?
        {
            if materialized.last_event_seq == last_event_seq
                && materialized.head_rev == projection_rev
            {
                let summary_checkpoint = self.get_session_summary_checkpoint(session_id).await?;
                let head =
                    materialized.into_session_head(session, projection_rev, summary_checkpoint);
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
            .build_session_head(
                &session,
                materialize_limits,
                true,
                last_event_seq,
                projection_rev,
            )
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
        let projection_rev = self.get_session_projection_rev(session_id).await?;
        if let Some(materialized) = self
            .load_session_head_materialization(session_id, SessionHeadKind::Active)
            .await?
        {
            if materialized.last_event_seq == last_event_seq
                && materialized.head_rev == projection_rev
            {
                return Ok(false);
            }
        }
        let _ = self
            .materialize_session_head(&session, SessionHeadKind::Active, last_event_seq)
            .await?;
        Ok(true)
    }
}
