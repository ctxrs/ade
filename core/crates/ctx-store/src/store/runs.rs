use super::*;

impl Store {
    pub async fn upsert_run(&self, run: RunRecord) -> Result<RunRecord> {
        let (retention_policy_key, retention_legal_hold_key) = run
            .retention_policy
            .as_ref()
            .map(|retention| {
                (
                    Some(retention.policy_key.clone()),
                    retention.legal_hold_key.clone(),
                )
            })
            .unwrap_or((None, None));

        self.query(
            r#"INSERT INTO runs (
                   id,
                   session_id,
                   task_id,
                   workspace_id,
                   worktree_id,
                   parent_run_id,
                   account_id,
                   org_id,
                   run_grant_id,
                   status,
                   archive_state,
                   archive_visibility,
                   retention_policy_key,
                   retention_legal_hold_key,
                   created_at,
                   started_at,
                   completed_at,
                   archived_at,
                   updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   session_id = excluded.session_id,
                   task_id = excluded.task_id,
                   workspace_id = excluded.workspace_id,
                   worktree_id = excluded.worktree_id,
                   parent_run_id = excluded.parent_run_id,
                   account_id = excluded.account_id,
                   org_id = excluded.org_id,
                   run_grant_id = excluded.run_grant_id,
                   status = excluded.status,
                   archive_state = excluded.archive_state,
                   archive_visibility = excluded.archive_visibility,
                   retention_policy_key = excluded.retention_policy_key,
                   retention_legal_hold_key = excluded.retention_legal_hold_key,
                   created_at = excluded.created_at,
                   started_at = excluded.started_at,
                   completed_at = excluded.completed_at,
                   archived_at = excluded.archived_at,
                   updated_at = excluded.updated_at"#,
        )
        .bind(run.id.0.to_string())
        .bind(run.session_id.0.to_string())
        .bind(run.task_id.0.to_string())
        .bind(run.workspace_id.0.to_string())
        .bind(run.worktree_id.0.to_string())
        .bind(run.parent_run_id.map(|id| id.0.to_string()))
        .bind(run.account_id.map(|id| id.0.to_string()))
        .bind(run.org_id.map(|id| id.0.to_string()))
        .bind(run.run_grant_id.map(|id| id.0.to_string()))
        .bind(run.status.as_str())
        .bind(run.archive_state.as_str())
        .bind(run.archive_visibility.as_str())
        .bind(retention_policy_key)
        .bind(retention_legal_hold_key)
        .bind(run.created_at.to_rfc3339())
        .bind(run.started_at.map(|dt| dt.to_rfc3339()))
        .bind(run.completed_at.map(|dt| dt.to_rfc3339()))
        .bind(run.archived_at.map(|dt| dt.to_rfc3339()))
        .bind(run.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(run)
    }

    pub async fn get_run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
        let row = self
            .query(
                r#"SELECT id, session_id, task_id, workspace_id, worktree_id, parent_run_id,
                          account_id, org_id, run_grant_id, status, archive_state,
                          archive_visibility, retention_policy_key, retention_legal_hold_key,
                          created_at, started_at, completed_at, archived_at, updated_at
                   FROM runs
                   WHERE id = ?"#,
            )
            .bind(run_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(build_run_record_from_row).transpose()
    }

    pub async fn update_run_status(
        &self,
        run_id: RunId,
        status: RunStatus,
        completed_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        let now = chrono::Utc::now();
        self.query(
            r#"UPDATE runs
               SET status = ?,
                   completed_at = COALESCE(?, completed_at),
                   updated_at = ?
               WHERE id = ?"#,
        )
        .bind(status.as_str())
        .bind(completed_at.map(|value| value.to_rfc3339()))
        .bind(now.to_rfc3339())
        .bind(run_id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn append_run_audit_event(&self, event: AuditEvent) -> Result<AuditEvent> {
        let (retention_policy_key, retention_legal_hold_key) = event
            .retention_policy
            .as_ref()
            .map(|retention| {
                (
                    Some(retention.policy_key.clone()),
                    retention.legal_hold_key.clone(),
                )
            })
            .unwrap_or((None, None));

        let _write_guard = self.write_gate.lock().await;
        let mut tx = self.pool.begin().await?;
        let ingest_seq: i64 = self
            .query_scalar(
                r#"SELECT next_seq
                   FROM run_archive_ingest_sequence
                   WHERE id = 1"#,
            )
            .fetch_one(&mut *tx)
            .await?;
        self.query(
            r#"UPDATE run_archive_ingest_sequence
               SET next_seq = ?
               WHERE id = 1"#,
        )
        .bind(ingest_seq + 1)
        .execute(&mut *tx)
        .await?;

        self.query(
            r#"INSERT INTO run_audit_events (
                   id,
                   workspace_id,
                   task_id,
                   session_id,
                   run_id,
                   account_id,
                   org_id,
                   actor_kind,
                   actor_account_id,
                   actor_org_id,
                   actor_membership_role,
                   event_kind,
                   archive_visibility,
                   retention_policy_key,
                   retention_legal_hold_key,
                   payload_json,
                   created_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&event.id)
        .bind(event.workspace_id.0.to_string())
        .bind(event.task_id.map(|id| id.0.to_string()))
        .bind(event.session_id.map(|id| id.0.to_string()))
        .bind(event.run_id.map(|id| id.0.to_string()))
        .bind(event.account_id.map(|id| id.0.to_string()))
        .bind(event.org_id.map(|id| id.0.to_string()))
        .bind(event.actor.kind.as_str())
        .bind(event.actor.account_id.map(|id| id.0.to_string()))
        .bind(event.actor.org_id.map(|id| id.0.to_string()))
        .bind(event.actor.membership_role.clone())
        .bind(event.event_kind.as_str())
        .bind(
            event
                .archive_visibility
                .map(|visibility| visibility.as_str()),
        )
        .bind(retention_policy_key)
        .bind(retention_legal_hold_key)
        .bind(event.payload_json.to_string())
        .bind(event.created_at.to_rfc3339())
        .execute(&mut *tx)
        .await?;

        self.query(
            r#"INSERT INTO run_audit_event_ingest_sequences (
                   audit_event_id,
                   run_id,
                   ingest_seq,
                   created_at
               )
               VALUES (?, ?, ?, ?)"#,
        )
        .bind(&event.id)
        .bind(event.run_id.map(|id| id.0.to_string()))
        .bind(ingest_seq)
        .bind(event.created_at.to_rfc3339())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(event)
    }

    pub async fn list_run_audit_events(&self, run_id: RunId) -> Result<Vec<AuditEvent>> {
        let rows = self
            .query(
                r#"SELECT id, workspace_id, task_id, session_id, run_id, account_id, org_id,
                          actor_kind, actor_account_id, actor_org_id, actor_membership_role,
                          event_kind, archive_visibility, retention_policy_key,
                          retention_legal_hold_key, payload_json, created_at
                   FROM run_audit_events
                   WHERE run_id = ?
                   ORDER BY created_at ASC, id ASC"#,
            )
            .bind(run_id.0.to_string())
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(build_audit_event_from_row).collect()
    }

    pub async fn get_run_archive_ingest_cursor(
        &self,
        run_id: RunId,
    ) -> Result<Option<RunArchiveIngestCursor>> {
        let row = self
            .query(
                r#"SELECT run_id, workspace_id, org_id, archive_visibility,
                          retention_policy_key, retention_legal_hold_key,
                          last_session_event_seq, last_audit_event_seq, last_batch_id,
                          last_synced_at, updated_at
                   FROM run_archive_ingest_cursors
                   WHERE run_id = ?"#,
            )
            .bind(run_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(build_run_archive_ingest_cursor_from_row)
            .transpose()
    }

    pub async fn build_run_archive_ingest_batch(
        &self,
        run_id: RunId,
        max_items: u32,
    ) -> Result<Option<RunArchiveIngestBatch>> {
        let cursor = self.get_run_archive_ingest_cursor(run_id).await?;
        let from = cursor
            .as_ref()
            .map(|cursor| cursor.watermark)
            .unwrap_or_default();
        self.build_run_archive_ingest_batch_after(run_id, from, max_items, cursor.is_none())
            .await
    }

    pub async fn build_run_archive_ingest_batch_after(
        &self,
        run_id: RunId,
        from: RunArchiveIngestWatermark,
        max_items: u32,
        force_run_snapshot: bool,
    ) -> Result<Option<RunArchiveIngestBatch>> {
        self.flush_event_log_for_reads().await;

        let Some(run) = self.get_run(run_id).await? else {
            return Ok(None);
        };
        let scope = RunArchiveIngestScope::from_visibility(run.archive_visibility);
        if !scope.is_cloud_visible() || run.org_id.is_none() {
            return Ok(None);
        }
        let Some(session) = self.get_session(run.session_id).await? else {
            return Ok(None);
        };

        let limit = i64::from(max_items.max(1));
        let raw_events = self
            .list_run_session_events_after(run.session_id, run.id, from.session_event_seq, limit)
            .await?;
        let raw_audit_events = self
            .list_run_audit_events_after(run.id, from.audit_event_seq, limit)
            .await?;

        if !force_run_snapshot && raw_events.is_empty() && raw_audit_events.is_empty() {
            return Ok(None);
        }

        let mut normalization = RunArchiveNormalizationStats::default();
        let mut to = from;
        let mut session_events = Vec::with_capacity(raw_events.len());
        for event in raw_events {
            to.session_event_seq = to.session_event_seq.max(event.seq);
            if let Some((normalized, stats)) = normalize_session_event_for_archive(&event, scope) {
                normalization.merge(stats);
                session_events.push(normalized);
            }
        }

        let mut audit_events = Vec::with_capacity(raw_audit_events.len());
        for sequenced in raw_audit_events {
            to.audit_event_seq = to.audit_event_seq.max(sequenced.ingest_seq);
            let normalized_payload = normalize_archive_json(&sequenced.event.payload_json);
            normalization.merge(normalized_payload.stats);
            audit_events.push(RunArchiveIngestAuditEvent {
                ingest_seq: sequenced.ingest_seq,
                id: sequenced.event.id,
                workspace_id: sequenced.event.workspace_id,
                task_id: sequenced.event.task_id,
                session_id: sequenced.event.session_id,
                run_id: sequenced.event.run_id,
                account_id: sequenced.event.account_id,
                org_id: sequenced.event.org_id,
                actor: sequenced.event.actor,
                event_kind: sequenced.event.event_kind,
                archive_visibility: sequenced.event.archive_visibility,
                retention_policy: sequenced.event.retention_policy,
                payload_json: normalized_payload.value,
                created_at: sequenced.event.created_at,
            });
        }

        let messages = if scope.includes_transcript() {
            let raw_messages = self.list_run_messages(run.session_id, run.id).await?;
            let mut out = Vec::with_capacity(raw_messages.len());
            for message in raw_messages {
                let normalized = normalize_archive_text(&message.content);
                normalization.merge(normalized.stats);
                out.push(RunArchiveIngestMessage {
                    id: message.id,
                    session_id: message.session_id,
                    task_id: message.task_id,
                    run_id: run.id,
                    turn_id: message.turn_id,
                    turn_sequence: message.turn_sequence,
                    role: message.role,
                    content: normalized.text,
                    created_at: message.created_at,
                });
            }
            out
        } else {
            Vec::new()
        };

        let idempotency_key = format!(
            "run-archive:{}:{}-{}:{}-{}:{}",
            run.id.0,
            from.session_event_seq + 1,
            to.session_event_seq,
            from.audit_event_seq + 1,
            to.audit_event_seq,
            scope.as_str()
        );

        Ok(Some(RunArchiveIngestBatch {
            idempotency_key,
            run: RunArchiveIngestRun {
                id: run.id,
                session_id: run.session_id,
                task_id: run.task_id,
                workspace_id: run.workspace_id,
                worktree_id: run.worktree_id,
                parent_run_id: run.parent_run_id,
                account_id: run.account_id,
                org_id: run.org_id,
                run_grant_id: run.run_grant_id,
                status: run.status,
                archive_state: run.archive_state,
                archive_visibility: run.archive_visibility,
                retention_policy: run.retention_policy,
                provider_id: session.provider_id,
                model_id: session.model_id,
                execution_environment: session.execution_environment.as_str().to_string(),
                created_at: run.created_at,
                started_at: run.started_at,
                completed_at: run.completed_at,
                archived_at: run.archived_at,
                updated_at: run.updated_at,
            },
            scope,
            from,
            to,
            messages,
            session_events,
            audit_events,
            normalization,
            created_at: Utc::now(),
        }))
    }

    pub async fn acknowledge_run_archive_ingest_batch(
        &self,
        batch: &RunArchiveIngestBatch,
    ) -> Result<RunArchiveIngestCursor> {
        let now = Utc::now();
        let retention_policy = batch.run.retention_policy.as_ref();
        self.query(
            r#"INSERT INTO run_archive_ingest_cursors (
                   run_id,
                   workspace_id,
                   org_id,
                   archive_visibility,
                   retention_policy_key,
                   retention_legal_hold_key,
                   last_session_event_seq,
                   last_audit_event_seq,
                   last_batch_id,
                   last_synced_at,
                   updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(run_id) DO UPDATE SET
                   workspace_id = excluded.workspace_id,
                   org_id = excluded.org_id,
                   archive_visibility = excluded.archive_visibility,
                   retention_policy_key = excluded.retention_policy_key,
                   retention_legal_hold_key = excluded.retention_legal_hold_key,
                   last_session_event_seq = MAX(
                       run_archive_ingest_cursors.last_session_event_seq,
                       excluded.last_session_event_seq
                   ),
                   last_audit_event_seq = MAX(
                       run_archive_ingest_cursors.last_audit_event_seq,
                       excluded.last_audit_event_seq
                   ),
                   last_batch_id = excluded.last_batch_id,
                   last_synced_at = excluded.last_synced_at,
                   updated_at = excluded.updated_at"#,
        )
        .bind(batch.run.id.0.to_string())
        .bind(batch.run.workspace_id.0.to_string())
        .bind(batch.run.org_id.map(|id| id.0.to_string()))
        .bind(batch.run.archive_visibility.as_str())
        .bind(retention_policy.map(|policy| policy.policy_key.as_str()))
        .bind(retention_policy.and_then(|policy| policy.legal_hold_key.as_deref()))
        .bind(batch.to.session_event_seq)
        .bind(batch.to.audit_event_seq)
        .bind(&batch.idempotency_key)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.get_run_archive_ingest_cursor(batch.run.id)
            .await?
            .with_context(|| "run archive ingest cursor missing after acknowledgement".to_string())
    }

    async fn list_run_session_events_after(
        &self,
        session_id: SessionId,
        run_id: RunId,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<SessionEvent>> {
        let rows = self
            .query(
                r#"SELECT seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
                   FROM session_events
                   WHERE session_id = ?
                     AND run_id = ?
                     AND transient = 0
                     AND seq > ?
                   ORDER BY seq ASC
                   LIMIT ?"#,
            )
            .bind(session_id.0.to_string())
            .bind(run_id.0.to_string())
            .bind(after_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(build_session_event_from_row).collect()
    }

    async fn list_run_messages(
        &self,
        session_id: SessionId,
        run_id: RunId,
    ) -> Result<Vec<Message>> {
        let rows = self
            .query(
                r#"SELECT id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq,
                          role, content, attachments_json, delivery, delivered_at, created_at
                   FROM messages
                   WHERE session_id = ? AND run_id = ?
                   ORDER BY created_at ASC, turn_sequence ASC, order_seq ASC"#,
            )
            .bind(session_id.0.to_string())
            .bind(run_id.0.to_string())
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(build_message_from_row).collect()
    }

    async fn list_run_audit_events_after(
        &self,
        run_id: RunId,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<SequencedAuditEvent>> {
        let rows = self
            .query(
                r#"SELECT s.ingest_seq, e.id, e.workspace_id, e.task_id, e.session_id, e.run_id,
                          e.account_id, e.org_id, e.actor_kind, e.actor_account_id,
                          e.actor_org_id, e.actor_membership_role, e.event_kind,
                          e.archive_visibility, e.retention_policy_key,
                          e.retention_legal_hold_key, e.payload_json, e.created_at
                   FROM run_audit_events AS e
                   JOIN run_audit_event_ingest_sequences AS s
                     ON s.audit_event_id = e.id
                   WHERE e.run_id = ?
                     AND s.ingest_seq > ?
                   ORDER BY s.ingest_seq ASC
                   LIMIT ?"#,
            )
            .bind(run_id.0.to_string())
            .bind(after_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(build_sequenced_audit_event_from_row)
            .collect()
    }
}

struct SequencedAuditEvent {
    ingest_seq: i64,
    event: AuditEvent,
}

fn build_run_archive_ingest_cursor_from_row(row: SqliteRow) -> Result<RunArchiveIngestCursor> {
    let run_id: String = row.try_get("run_id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let org_id: Option<String> = row.try_get("org_id")?;
    let archive_visibility: String = row.try_get("archive_visibility")?;
    let retention_policy_key: Option<String> = row.try_get("retention_policy_key")?;
    let retention_legal_hold_key: Option<String> = row.try_get("retention_legal_hold_key")?;
    let last_session_event_seq: i64 = row.try_get("last_session_event_seq")?;
    let last_audit_event_seq: i64 = row.try_get("last_audit_event_seq")?;
    let last_batch_id: Option<String> = row.try_get("last_batch_id")?;
    let last_synced_at: Option<String> = row.try_get("last_synced_at")?;
    let updated_at: String = row.try_get("updated_at")?;

    Ok(RunArchiveIngestCursor {
        run_id: parse_uuid_id(run_id, "run_archive_ingest_cursors.run_id", RunId)?,
        workspace_id: parse_uuid_id(
            workspace_id,
            "run_archive_ingest_cursors.workspace_id",
            WorkspaceId,
        )?,
        org_id: parse_opt_uuid_id(org_id, "run_archive_ingest_cursors.org_id", OrgId)?,
        archive_visibility: ArchiveVisibility::parse(&archive_visibility).with_context(|| {
            format!("invalid run_archive_ingest_cursors.archive_visibility: {archive_visibility}")
        })?,
        retention_policy: retention_policy_key.map(|policy_key| RetentionPolicyRef {
            policy_key,
            legal_hold_key: retention_legal_hold_key,
        }),
        watermark: RunArchiveIngestWatermark {
            session_event_seq: last_session_event_seq,
            audit_event_seq: last_audit_event_seq,
        },
        last_batch_id,
        last_synced_at: last_synced_at.as_deref().map(parse_dt).transpose()?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn build_session_event_from_row(row: SqliteRow) -> Result<SessionEvent> {
    let id: String = row.try_get("id")?;
    let session_id: String = row.try_get("session_id")?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let turn_id: Option<String> = row.try_get("turn_id")?;
    let event_type: String = row.try_get("event_type")?;
    let payload_json: String = row.try_get("payload_json")?;
    let transient: i64 = row.try_get("transient")?;
    let created_at: String = row.try_get("created_at")?;

    Ok(SessionEvent {
        seq: row.try_get("seq")?,
        id: parse_uuid_id(id, "session_events.id", SessionEventId)?,
        session_id: parse_uuid_id(session_id, "session_events.session_id", SessionId)?,
        run_id: parse_opt_uuid_id(run_id, "session_events.run_id", RunId)?,
        turn_id: parse_opt_uuid_id(turn_id, "session_events.turn_id", TurnId)?,
        event_type: parse_session_event_type(&event_type),
        payload_json: serde_json::from_str(&payload_json)
            .with_context(|| "failed to parse session_events.payload_json".to_string())?,
        transient: transient != 0,
        created_at: parse_dt(&created_at)?,
    })
}

fn build_message_from_row(row: SqliteRow) -> Result<Message> {
    let id: String = row.try_get("id")?;
    let session_id: String = row.try_get("session_id")?;
    let task_id: String = row.try_get("task_id")?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let turn_id: Option<String> = row.try_get("turn_id")?;
    let turn_sequence: Option<i64> = row.try_get("turn_sequence")?;
    let order_seq: Option<i64> = row.try_get("order_seq")?;
    let role: String = row.try_get("role")?;
    let content: String = row.try_get("content")?;
    let attachments_json: Option<String> = row.try_get("attachments_json")?;
    let delivery: String = row.try_get("delivery")?;
    let delivered_at: Option<String> = row.try_get("delivered_at")?;
    let created_at: String = row.try_get("created_at")?;
    let attachments = attachments_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<MessageAttachment>>(value).ok())
        .unwrap_or_default();

    Ok(Message {
        id: parse_uuid_id(id, "messages.id", MessageId)?,
        session_id: parse_uuid_id(session_id, "messages.session_id", SessionId)?,
        task_id: parse_uuid_id(task_id, "messages.task_id", TaskId)?,
        run_id: parse_opt_uuid_id(run_id, "messages.run_id", RunId)?,
        turn_id: parse_opt_uuid_id(turn_id, "messages.turn_id", TurnId)?,
        turn_sequence,
        order_seq,
        role: parse_message_role(&role),
        content,
        attachments,
        delivery: parse_message_delivery(&delivery),
        delivered_at: delivered_at.as_deref().map(parse_dt).transpose()?,
        created_at: parse_dt(&created_at)?,
    })
}

fn build_sequenced_audit_event_from_row(row: SqliteRow) -> Result<SequencedAuditEvent> {
    let ingest_seq: i64 = row.try_get("ingest_seq")?;
    Ok(SequencedAuditEvent {
        ingest_seq,
        event: build_audit_event_from_row(row)?,
    })
}

fn build_run_record_from_row(row: SqliteRow) -> Result<RunRecord> {
    let id: String = row.try_get("id")?;
    let session_id: String = row.try_get("session_id")?;
    let task_id: String = row.try_get("task_id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let worktree_id: String = row.try_get("worktree_id")?;
    let parent_run_id: Option<String> = row.try_get("parent_run_id")?;
    let account_id: Option<String> = row.try_get("account_id")?;
    let org_id: Option<String> = row.try_get("org_id")?;
    let run_grant_id: Option<String> = row.try_get("run_grant_id")?;
    let status: String = row.try_get("status")?;
    let archive_state: String = row.try_get("archive_state")?;
    let archive_visibility: String = row.try_get("archive_visibility")?;
    let retention_policy_key: Option<String> = row.try_get("retention_policy_key")?;
    let retention_legal_hold_key: Option<String> = row.try_get("retention_legal_hold_key")?;
    let created_at: String = row.try_get("created_at")?;
    let started_at: Option<String> = row.try_get("started_at")?;
    let completed_at: Option<String> = row.try_get("completed_at")?;
    let archived_at: Option<String> = row.try_get("archived_at")?;
    let updated_at: String = row.try_get("updated_at")?;

    Ok(RunRecord {
        id: parse_uuid_id(id, "runs.id", RunId)?,
        session_id: parse_uuid_id(session_id, "runs.session_id", SessionId)?,
        task_id: parse_uuid_id(task_id, "runs.task_id", TaskId)?,
        workspace_id: parse_uuid_id(workspace_id, "runs.workspace_id", WorkspaceId)?,
        worktree_id: parse_uuid_id(worktree_id, "runs.worktree_id", WorktreeId)?,
        parent_run_id: parse_opt_uuid_id(parent_run_id, "runs.parent_run_id", RunId)?,
        account_id: parse_opt_uuid_id(account_id, "runs.account_id", AccountId)?,
        org_id: parse_opt_uuid_id(org_id, "runs.org_id", OrgId)?,
        run_grant_id: parse_opt_uuid_id(run_grant_id, "runs.run_grant_id", RunGrantId)?,
        status: RunStatus::parse(&status)
            .with_context(|| format!("invalid runs.status value: {status}"))?,
        archive_state: RunArchiveState::parse(&archive_state)
            .with_context(|| format!("invalid runs.archive_state value: {archive_state}"))?,
        archive_visibility: ArchiveVisibility::parse(&archive_visibility).with_context(|| {
            format!("invalid runs.archive_visibility value: {archive_visibility}")
        })?,
        retention_policy: retention_policy_key.map(|policy_key| RetentionPolicyRef {
            policy_key,
            legal_hold_key: retention_legal_hold_key,
        }),
        created_at: parse_dt(&created_at)?,
        started_at: started_at.as_deref().map(parse_dt).transpose()?,
        completed_at: completed_at.as_deref().map(parse_dt).transpose()?,
        archived_at: archived_at.as_deref().map(parse_dt).transpose()?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn build_audit_event_from_row(row: SqliteRow) -> Result<AuditEvent> {
    let id: String = row.try_get("id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let task_id: Option<String> = row.try_get("task_id")?;
    let session_id: Option<String> = row.try_get("session_id")?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let account_id: Option<String> = row.try_get("account_id")?;
    let org_id: Option<String> = row.try_get("org_id")?;
    let actor_kind: String = row.try_get("actor_kind")?;
    let actor_account_id: Option<String> = row.try_get("actor_account_id")?;
    let actor_org_id: Option<String> = row.try_get("actor_org_id")?;
    let actor_membership_role: Option<String> = row.try_get("actor_membership_role")?;
    let event_kind: String = row.try_get("event_kind")?;
    let archive_visibility: Option<String> = row.try_get("archive_visibility")?;
    let retention_policy_key: Option<String> = row.try_get("retention_policy_key")?;
    let retention_legal_hold_key: Option<String> = row.try_get("retention_legal_hold_key")?;
    let payload_json: String = row.try_get("payload_json")?;
    let created_at: String = row.try_get("created_at")?;

    Ok(AuditEvent {
        id,
        workspace_id: parse_uuid_id(workspace_id, "run_audit_events.workspace_id", WorkspaceId)?,
        task_id: parse_opt_uuid_id(task_id, "run_audit_events.task_id", TaskId)?,
        session_id: parse_opt_uuid_id(session_id, "run_audit_events.session_id", SessionId)?,
        run_id: parse_opt_uuid_id(run_id, "run_audit_events.run_id", RunId)?,
        account_id: parse_opt_uuid_id(account_id, "run_audit_events.account_id", AccountId)?,
        org_id: parse_opt_uuid_id(org_id, "run_audit_events.org_id", OrgId)?,
        actor: AuditActor {
            kind: AuditActorKind::parse(&actor_kind)
                .with_context(|| format!("invalid run_audit_events.actor_kind: {actor_kind}"))?,
            account_id: parse_opt_uuid_id(
                actor_account_id,
                "run_audit_events.actor_account_id",
                AccountId,
            )?,
            org_id: parse_opt_uuid_id(actor_org_id, "run_audit_events.actor_org_id", OrgId)?,
            membership_role: actor_membership_role,
        },
        event_kind: AuditEventKind::parse(&event_kind)
            .with_context(|| format!("invalid run_audit_events.event_kind: {event_kind}"))?,
        archive_visibility: archive_visibility
            .map(|value| {
                ArchiveVisibility::parse(&value).with_context(|| {
                    format!("invalid run_audit_events.archive_visibility: {value}")
                })
            })
            .transpose()?,
        retention_policy: retention_policy_key.map(|policy_key| RetentionPolicyRef {
            policy_key,
            legal_hold_key: retention_legal_hold_key,
        }),
        payload_json: serde_json::from_str(&payload_json)
            .with_context(|| "failed to parse run_audit_events.payload_json".to_string())?,
        created_at: parse_dt(&created_at)?,
    })
}

fn parse_uuid_id<T>(value: String, field_name: &str, wrap: fn(uuid::Uuid) -> T) -> Result<T> {
    Ok(wrap(uuid::Uuid::parse_str(&value).with_context(|| {
        format!("invalid UUID in {field_name}: {value}")
    })?))
}

fn parse_opt_uuid_id<T>(
    value: Option<String>,
    field_name: &str,
    wrap: fn(uuid::Uuid) -> T,
) -> Result<Option<T>> {
    value
        .map(|value| parse_uuid_id(value, field_name, wrap))
        .transpose()
}
