use super::*;

#[derive(Debug, Clone, Default)]
pub struct WorkSearchQuery {
    pub text: Option<String>,
    pub path: Option<String>,
    pub pr_owner: Option<String>,
    pub pr_repo: Option<String>,
    pub pr_number: Option<i64>,
    pub commit_sha: Option<String>,
    pub freshness: Option<WorkEvidenceFreshness>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkSearchHit {
    pub doc: WorkSearchDoc,
    pub score: f64,
}

impl Store {
    pub async fn upsert_work_record(&self, record: &WorkRecord) -> Result<WorkRecord> {
        validate_work_observability_schema_version(record.schema_version, "work record")?;
        let mut record = record.clone();
        let now = Utc::now();
        if record.created_at.timestamp() == 0 {
            record.created_at = self
                .work_record_created_at(record.workspace_id, record.work_id.clone())
                .await?
                .unwrap_or(now);
        }
        if record.updated_at.timestamp() == 0 {
            record.updated_at = now;
        }
        insert_work_record_on_pool(&self.pool, &record).await?;
        Ok(record)
    }

    pub async fn get_workspace_work_record(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
    ) -> Result<Option<WorkRecord>> {
        let row = self
            .query(
                r#"SELECT record_json FROM work_records
                   WHERE workspace_id = ? AND work_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_optional(&self.pool)
            .await?;
        row.map(decode_work_record_row).transpose()
    }

    pub async fn list_workspace_work_records(
        &self,
        workspace_id: WorkspaceId,
        limit: Option<usize>,
    ) -> Result<Vec<WorkRecord>> {
        let limit = limit.unwrap_or(usize::MAX).min(5_000) as i64;
        let rows = self
            .query(
                r#"SELECT record_json FROM work_records
                   WHERE workspace_id = ?
                   ORDER BY updated_at DESC, work_id DESC
                   LIMIT ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(decode_work_record_row).collect()
    }

    pub async fn find_work_record_by_link(
        &self,
        workspace_id: WorkspaceId,
        target_kind: WorkLinkTargetKind,
        target_id: &str,
    ) -> Result<Option<WorkRecord>> {
        let row = self
            .query(
                r#"SELECT wr.record_json
                   FROM work_record_links wrl
                   JOIN work_records wr
                     ON wr.workspace_id = wrl.workspace_id
                    AND wr.work_id = wrl.work_id
                   WHERE wrl.workspace_id = ?
                     AND wrl.target_kind = ?
                     AND wrl.target_id = ?
                   ORDER BY wrl.updated_at DESC, wrl.link_id DESC
                   LIMIT 1"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(enum_db(&target_kind)?)
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(decode_work_record_row).transpose()
    }

    pub async fn upsert_work_record_link(&self, link: &WorkRecordLink) -> Result<WorkRecordLink> {
        validate_work_observability_schema_version(link.schema_version, "work record link")?;
        self.validate_work_record_workspace(link.workspace_id, &link.work_id)
            .await?;
        let mut link = link.clone();
        if let Some(existing_id) = self
            .existing_work_record_link_id(
                link.workspace_id,
                &link.work_id,
                link.target_kind,
                link.target_id.as_deref(),
                link.role,
            )
            .await?
        {
            link.link_id = existing_id;
            link.created_at = self
                .work_record_link_created_at(link.workspace_id, link.link_id.clone())
                .await?
                .unwrap_or(link.created_at);
        }
        insert_work_record_link_on_pool(&self.pool, &link).await?;
        self.touch_work_record(link.workspace_id, &link.work_id)
            .await?;
        Ok(link)
    }

    pub async fn list_work_record_links(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
    ) -> Result<Vec<WorkRecordLink>> {
        let rows = self
            .query(
                r#"SELECT record_json FROM work_record_links
                   WHERE workspace_id = ? AND work_id = ?
                   ORDER BY updated_at DESC, link_id DESC"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(decode_work_record_link_row).collect()
    }

    pub async fn append_work_event(&self, event: &WorkEvent) -> Result<WorkEvent> {
        validate_work_observability_schema_version(event.schema_version, "work event")?;
        self.validate_work_record_workspace(event.workspace_id, &event.work_id)
            .await?;
        let mut event = event.clone();
        if event.sequence <= 0 {
            event.sequence = self
                .query_scalar::<i64>(
                    r#"SELECT COALESCE(MAX(sequence), 0) + 1
                       FROM work_events
                       WHERE workspace_id = ? AND work_id = ?"#,
                )
                .bind(event.workspace_id.0.to_string())
                .bind(event.work_id.0.to_string())
                .fetch_one(&self.pool)
                .await?;
        }
        insert_work_event_on_pool(&self.pool, &event).await?;
        self.touch_work_record(event.workspace_id, &event.work_id)
            .await?;
        Ok(event)
    }

    pub async fn list_work_events(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
        limit: Option<usize>,
    ) -> Result<Vec<WorkEvent>> {
        let limit = limit.unwrap_or(usize::MAX).min(5_000) as i64;
        let rows = self
            .query(
                r#"SELECT record_json FROM work_events
                   WHERE workspace_id = ? AND work_id = ?
                   ORDER BY sequence ASC, event_id ASC
                   LIMIT ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(decode_work_event_row).collect()
    }

    pub async fn upsert_work_evidence(&self, evidence: &WorkEvidence) -> Result<WorkEvidence> {
        validate_work_observability_schema_version(evidence.schema_version, "work evidence")?;
        self.validate_work_record_workspace(evidence.workspace_id, &evidence.work_id)
            .await?;
        insert_work_evidence_on_pool(&self.pool, evidence).await?;
        self.touch_work_record(evidence.workspace_id, &evidence.work_id)
            .await?;
        Ok(evidence.clone())
    }

    pub async fn list_work_evidence(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
    ) -> Result<Vec<WorkEvidence>> {
        let rows = self
            .query(
                r#"SELECT record_json FROM work_evidence
                   WHERE workspace_id = ? AND work_id = ?
                   ORDER BY updated_at DESC, evidence_id DESC"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(decode_work_evidence_row).collect()
    }

    pub async fn upsert_work_summary(&self, summary: &WorkSummary) -> Result<WorkSummary> {
        validate_work_observability_schema_version(summary.schema_version, "work summary")?;
        self.validate_work_record_workspace(summary.workspace_id, &summary.work_id)
            .await?;
        insert_work_summary_on_pool(&self.pool, summary).await?;
        self.touch_work_record(summary.workspace_id, &summary.work_id)
            .await?;
        Ok(summary.clone())
    }

    pub async fn list_work_summaries(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
    ) -> Result<Vec<WorkSummary>> {
        let rows = self
            .query(
                r#"SELECT record_json FROM work_summaries
                   WHERE workspace_id = ? AND work_id = ?
                   ORDER BY updated_at DESC, summary_id DESC"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(decode_work_summary_row).collect()
    }

    pub async fn upsert_work_summary_claim(
        &self,
        claim: &WorkSummaryClaim,
    ) -> Result<WorkSummaryClaim> {
        validate_work_observability_schema_version(claim.schema_version, "work summary claim")?;
        self.validate_work_record_workspace(claim.workspace_id, &claim.work_id)
            .await?;
        insert_work_summary_claim_on_pool(&self.pool, claim).await?;
        Ok(claim.clone())
    }

    pub async fn list_work_summary_claims(
        &self,
        workspace_id: WorkspaceId,
        summary_id: Option<WorkSummaryId>,
        work_id: WorkRecordId,
    ) -> Result<Vec<WorkSummaryClaim>> {
        let rows = if let Some(summary_id) = summary_id {
            self.query(
                r#"SELECT record_json FROM work_summary_claims
                   WHERE workspace_id = ? AND work_id = ? AND summary_id = ?
                   ORDER BY claim_id ASC"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0.to_string())
            .bind(summary_id.0)
            .fetch_all(&self.pool)
            .await?
        } else {
            self.query(
                r#"SELECT record_json FROM work_summary_claims
                   WHERE workspace_id = ? AND work_id = ?
                   ORDER BY claim_id ASC"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter()
            .map(decode_work_summary_claim_row)
            .collect()
    }

    pub async fn upsert_work_search_doc(&self, doc: &WorkSearchDoc) -> Result<WorkSearchDoc> {
        validate_work_observability_schema_version(doc.schema_version, "work search doc")?;
        self.validate_work_record_workspace(doc.workspace_id, &doc.work_id)
            .await?;
        insert_work_search_doc_on_pool(&self.pool, doc).await?;
        Ok(doc.clone())
    }

    pub async fn delete_workspace_work_search_docs(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<u64> {
        let workspace = workspace_id.0.to_string();
        let affected = self
            .query(r#"DELETE FROM work_search_docs WHERE workspace_id = ?"#)
            .bind(&workspace)
            .execute(&self.pool)
            .await?
            .rows_affected();
        self.query(r#"DELETE FROM work_search_docs_fts WHERE workspace_id = ?"#)
            .bind(&workspace)
            .execute(&self.pool)
            .await?;
        Ok(affected)
    }

    pub async fn search_work_docs(
        &self,
        workspace_id: WorkspaceId,
        query: WorkSearchQuery,
    ) -> Result<Vec<WorkSearchHit>> {
        let limit = query.limit.unwrap_or(20).min(100) as i64;
        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let fts = fts_query(text);
            let rows = sqlx::query(
                r#"SELECT d.record_json, bm25(work_search_docs_fts) AS rank
                   FROM work_search_docs_fts
                   JOIN work_search_docs d ON d.doc_id = work_search_docs_fts.doc_id
                   WHERE work_search_docs_fts MATCH ?
                     AND d.workspace_id = ?
                     AND (? IS NULL OR d.path = ?)
                     AND (? IS NULL OR d.commit_sha = ?)
                     AND (? IS NULL OR d.pr_owner = ?)
                     AND (? IS NULL OR d.pr_repo = ?)
                     AND (? IS NULL OR d.pr_number = ?)
                     AND (? IS NULL OR d.freshness = ?)
                   ORDER BY rank ASC, d.updated_at DESC
                   LIMIT ?"#,
            )
            .bind(fts)
            .bind(workspace_id.0.to_string())
            .bind(query.path.as_deref())
            .bind(query.path.as_deref())
            .bind(query.commit_sha.as_deref())
            .bind(query.commit_sha.as_deref())
            .bind(query.pr_owner.as_deref())
            .bind(query.pr_owner.as_deref())
            .bind(query.pr_repo.as_deref())
            .bind(query.pr_repo.as_deref())
            .bind(query.pr_number)
            .bind(query.pr_number)
            .bind(query.freshness.as_ref().map(enum_db).transpose()?)
            .bind(query.freshness.as_ref().map(enum_db).transpose()?)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            return rows.into_iter().map(decode_work_search_hit_row).collect();
        }

        let rows = sqlx::query(
            r#"SELECT record_json, 0.0 AS rank
               FROM work_search_docs
               WHERE workspace_id = ?
                 AND (? IS NULL OR path = ?)
                 AND (? IS NULL OR commit_sha = ?)
                 AND (? IS NULL OR pr_owner = ?)
                 AND (? IS NULL OR pr_repo = ?)
                 AND (? IS NULL OR pr_number = ?)
                 AND (? IS NULL OR freshness = ?)
               ORDER BY updated_at DESC, doc_id DESC
               LIMIT ?"#,
        )
        .bind(workspace_id.0.to_string())
        .bind(query.path.as_deref())
        .bind(query.path.as_deref())
        .bind(query.commit_sha.as_deref())
        .bind(query.commit_sha.as_deref())
        .bind(query.pr_owner.as_deref())
        .bind(query.pr_owner.as_deref())
        .bind(query.pr_repo.as_deref())
        .bind(query.pr_repo.as_deref())
        .bind(query.pr_number)
        .bind(query.pr_number)
        .bind(query.freshness.as_ref().map(enum_db).transpose()?)
        .bind(query.freshness.as_ref().map(enum_db).transpose()?)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(decode_work_search_hit_row).collect()
    }

    async fn work_record_created_at(
        &self,
        workspace_id: WorkspaceId,
        work_id: WorkRecordId,
    ) -> Result<Option<DateTime<Utc>>> {
        let raw = self
            .query_scalar::<String>(
                r#"SELECT created_at FROM work_records
                   WHERE workspace_id = ? AND work_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0)
            .fetch_optional(&self.pool)
            .await?;
        raw.as_deref().map(parse_dt).transpose()
    }

    async fn work_record_link_created_at(
        &self,
        workspace_id: WorkspaceId,
        link_id: WorkRecordLinkId,
    ) -> Result<Option<DateTime<Utc>>> {
        let raw = self
            .query_scalar::<String>(
                r#"SELECT created_at FROM work_record_links
                   WHERE workspace_id = ? AND link_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(link_id.0)
            .fetch_optional(&self.pool)
            .await?;
        raw.as_deref().map(parse_dt).transpose()
    }

    async fn existing_work_record_link_id(
        &self,
        workspace_id: WorkspaceId,
        work_id: &WorkRecordId,
        target_kind: WorkLinkTargetKind,
        target_id: Option<&str>,
        role: WorkLinkRole,
    ) -> Result<Option<WorkRecordLinkId>> {
        let Some(target_id) = target_id else {
            return Ok(None);
        };
        let raw = self
            .query_scalar::<String>(
                r#"SELECT link_id FROM work_record_links
                   WHERE workspace_id = ?
                     AND work_id = ?
                     AND target_kind = ?
                     AND target_id = ?
                     AND role = ?
                   LIMIT 1"#,
            )
            .bind(workspace_id.0.to_string())
            .bind(work_id.0.to_string())
            .bind(enum_db(&target_kind)?)
            .bind(target_id)
            .bind(enum_db(&role)?)
            .fetch_optional(&self.pool)
            .await?;
        Ok(raw.map(WorkRecordLinkId))
    }

    async fn validate_work_record_workspace(
        &self,
        workspace_id: WorkspaceId,
        work_id: &WorkRecordId,
    ) -> Result<()> {
        let found = self
            .query_scalar::<String>(r#"SELECT workspace_id FROM work_records WHERE work_id = ?"#)
            .bind(work_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match found {
            Some(found) if found == workspace_id.0.to_string() => Ok(()),
            Some(_) => anyhow::bail!("work record belongs to a different workspace"),
            None => anyhow::bail!("work record does not exist"),
        }
    }

    async fn touch_work_record(
        &self,
        workspace_id: WorkspaceId,
        work_id: &WorkRecordId,
    ) -> Result<()> {
        let Some(mut record) = self
            .get_workspace_work_record(workspace_id, work_id.clone())
            .await?
        else {
            return Ok(());
        };
        record.updated_at = Utc::now();
        insert_work_record_on_pool(&self.pool, &record).await
    }
}

async fn insert_work_record_on_pool(pool: &Pool<Sqlite>, record: &WorkRecord) -> Result<()> {
    let record_json = serde_json::to_string(record).context("serializing work record")?;
    sqlx::query(
        r#"INSERT INTO work_records (
             work_id, workspace_id, title, objective, lifecycle, primary_repo_root,
             primary_branch, base_commit, head_commit, current_diff_fingerprint_json,
             trust_verdict, summary_freshness, metadata_json, record_json,
             created_at, updated_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(work_id) DO UPDATE SET
             title = excluded.title,
             objective = excluded.objective,
             lifecycle = excluded.lifecycle,
             primary_repo_root = excluded.primary_repo_root,
             primary_branch = excluded.primary_branch,
             base_commit = excluded.base_commit,
             head_commit = excluded.head_commit,
             current_diff_fingerprint_json = excluded.current_diff_fingerprint_json,
             trust_verdict = excluded.trust_verdict,
             summary_freshness = excluded.summary_freshness,
             metadata_json = excluded.metadata_json,
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             schema_version = excluded.schema_version
           WHERE work_records.workspace_id = excluded.workspace_id"#,
    )
    .bind(record.work_id.0.to_string())
    .bind(record.workspace_id.0.to_string())
    .bind(record.title.as_deref())
    .bind(record.objective.as_deref())
    .bind(enum_db(&record.lifecycle)?)
    .bind(record.primary_repo_root.as_deref())
    .bind(record.primary_branch.as_deref())
    .bind(record.base_commit.as_deref())
    .bind(record.head_commit.as_deref())
    .bind(json_opt(&record.current_diff_fingerprint)?)
    .bind(enum_db(&record.trust_verdict)?)
    .bind(enum_db(&record.summary_freshness)?)
    .bind(value_json_opt(&record.metadata_json)?)
    .bind(record_json)
    .bind(record.created_at.to_rfc3339())
    .bind(record.updated_at.to_rfc3339())
    .bind(record.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_record_link_on_pool(pool: &Pool<Sqlite>, link: &WorkRecordLink) -> Result<()> {
    let record_json = serde_json::to_string(link).context("serializing work record link")?;
    sqlx::query(
        r#"INSERT INTO work_record_links (
             link_id, work_id, workspace_id, target_kind, target_id, target_json, role,
             source, fidelity, trust, record_json, created_at, updated_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(link_id) DO UPDATE SET
             target_kind = excluded.target_kind,
             target_id = excluded.target_id,
             target_json = excluded.target_json,
             role = excluded.role,
             source = excluded.source,
             fidelity = excluded.fidelity,
             trust = excluded.trust,
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             schema_version = excluded.schema_version
           WHERE work_record_links.workspace_id = excluded.workspace_id
             AND work_record_links.work_id = excluded.work_id"#,
    )
    .bind(link.link_id.0.to_string())
    .bind(link.work_id.0.to_string())
    .bind(link.workspace_id.0.to_string())
    .bind(enum_db(&link.target_kind)?)
    .bind(link.target_id.as_deref())
    .bind(value_json_opt(&link.target_json)?)
    .bind(enum_db(&link.role)?)
    .bind(enum_db(&link.source)?)
    .bind(enum_db(&link.fidelity)?)
    .bind(enum_db(&link.trust)?)
    .bind(record_json)
    .bind(link.created_at.to_rfc3339())
    .bind(link.updated_at.to_rfc3339())
    .bind(link.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_event_on_pool(pool: &Pool<Sqlite>, event: &WorkEvent) -> Result<()> {
    let record_json = serde_json::to_string(event).context("serializing work event")?;
    sqlx::query(
        r#"INSERT INTO work_events (
             event_id, work_id, workspace_id, sequence, source_kind, source_id, event_type,
             event_time, actor_kind, provider, harness, model, redaction_class, source,
             fidelity, trust, payload_json, redacted_text, artifact_ref_json, record_json,
             created_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(event_id) DO UPDATE SET
             sequence = excluded.sequence,
             source_kind = excluded.source_kind,
             source_id = excluded.source_id,
             event_type = excluded.event_type,
             event_time = excluded.event_time,
             actor_kind = excluded.actor_kind,
             provider = excluded.provider,
             harness = excluded.harness,
             model = excluded.model,
             redaction_class = excluded.redaction_class,
             source = excluded.source,
             fidelity = excluded.fidelity,
             trust = excluded.trust,
             payload_json = excluded.payload_json,
             redacted_text = excluded.redacted_text,
             artifact_ref_json = excluded.artifact_ref_json,
             record_json = excluded.record_json,
             schema_version = excluded.schema_version
           WHERE work_events.workspace_id = excluded.workspace_id
             AND work_events.work_id = excluded.work_id"#,
    )
    .bind(event.event_id.0.to_string())
    .bind(event.work_id.0.to_string())
    .bind(event.workspace_id.0.to_string())
    .bind(event.sequence)
    .bind(event.source_kind.as_deref())
    .bind(event.source_id.as_deref())
    .bind(enum_db(&event.event_type)?)
    .bind(event.event_time.to_rfc3339())
    .bind(enum_db(&event.actor_kind)?)
    .bind(event.provider.as_deref())
    .bind(event.harness.as_deref())
    .bind(event.model.as_deref())
    .bind(enum_db(&event.redaction_class)?)
    .bind(enum_db(&event.source)?)
    .bind(enum_db(&event.fidelity)?)
    .bind(enum_db(&event.trust)?)
    .bind(value_json_opt(&event.payload_json)?)
    .bind(event.redacted_text.as_deref())
    .bind(value_json_opt(&event.artifact_ref)?)
    .bind(record_json)
    .bind(event.created_at.to_rfc3339())
    .bind(event.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_evidence_on_pool(pool: &Pool<Sqlite>, evidence: &WorkEvidence) -> Result<()> {
    let record_json = serde_json::to_string(evidence).context("serializing work evidence")?;
    sqlx::query(
        r#"INSERT INTO work_evidence (
             evidence_id, work_id, workspace_id, kind, status, freshness, claim, command,
             argv_json, cwd, exit_code, repo_root, head_sha, branch, fingerprint_json,
             current_fingerprint_json, output_ref_json, artifact_ref_json, source, fidelity,
             trust, record_json, started_at, finished_at, created_at, updated_at,
             schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(evidence_id) DO UPDATE SET
             kind = excluded.kind,
             status = excluded.status,
             freshness = excluded.freshness,
             claim = excluded.claim,
             command = excluded.command,
             argv_json = excluded.argv_json,
             cwd = excluded.cwd,
             exit_code = excluded.exit_code,
             repo_root = excluded.repo_root,
             head_sha = excluded.head_sha,
             branch = excluded.branch,
             fingerprint_json = excluded.fingerprint_json,
             current_fingerprint_json = excluded.current_fingerprint_json,
             output_ref_json = excluded.output_ref_json,
             artifact_ref_json = excluded.artifact_ref_json,
             source = excluded.source,
             fidelity = excluded.fidelity,
             trust = excluded.trust,
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             schema_version = excluded.schema_version
           WHERE work_evidence.workspace_id = excluded.workspace_id
             AND work_evidence.work_id = excluded.work_id"#,
    )
    .bind(evidence.evidence_id.0.to_string())
    .bind(evidence.work_id.0.to_string())
    .bind(evidence.workspace_id.0.to_string())
    .bind(enum_db(&evidence.kind)?)
    .bind(enum_db(&evidence.status)?)
    .bind(enum_db(&evidence.freshness)?)
    .bind(evidence.claim.as_deref())
    .bind(evidence.command.as_deref())
    .bind(serde_json::to_string(&evidence.argv)?)
    .bind(evidence.cwd.as_deref())
    .bind(evidence.exit_code)
    .bind(evidence.repo_root.as_deref())
    .bind(evidence.head_sha.as_deref())
    .bind(evidence.branch.as_deref())
    .bind(json_opt(&evidence.fingerprint)?)
    .bind(json_opt(&evidence.current_fingerprint)?)
    .bind(value_json_opt(&evidence.output_ref)?)
    .bind(value_json_opt(&evidence.artifact_ref)?)
    .bind(enum_db(&evidence.source)?)
    .bind(enum_db(&evidence.fidelity)?)
    .bind(enum_db(&evidence.trust)?)
    .bind(record_json)
    .bind(evidence.started_at.to_rfc3339())
    .bind(evidence.finished_at.to_rfc3339())
    .bind(evidence.created_at.to_rfc3339())
    .bind(evidence.updated_at.to_rfc3339())
    .bind(evidence.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_summary_on_pool(pool: &Pool<Sqlite>, summary: &WorkSummary) -> Result<()> {
    let record_json = serde_json::to_string(summary).context("serializing work summary")?;
    sqlx::query(
        r#"INSERT INTO work_summaries (
             summary_id, work_id, workspace_id, kind, audience, text, structured_json,
             generation_method, provider, model, template, source_material_left_machine,
             freshness, source_revision_key, record_json, generated_at, created_at,
             updated_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(summary_id) DO UPDATE SET
             kind = excluded.kind,
             audience = excluded.audience,
             text = excluded.text,
             structured_json = excluded.structured_json,
             generation_method = excluded.generation_method,
             provider = excluded.provider,
             model = excluded.model,
             template = excluded.template,
             source_material_left_machine = excluded.source_material_left_machine,
             freshness = excluded.freshness,
             source_revision_key = excluded.source_revision_key,
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             schema_version = excluded.schema_version
           WHERE work_summaries.workspace_id = excluded.workspace_id
             AND work_summaries.work_id = excluded.work_id"#,
    )
    .bind(summary.summary_id.0.to_string())
    .bind(summary.work_id.0.to_string())
    .bind(summary.workspace_id.0.to_string())
    .bind(enum_db(&summary.kind)?)
    .bind(enum_db(&summary.audience)?)
    .bind(&summary.text)
    .bind(value_json_opt(&summary.structured_json)?)
    .bind(enum_db(&summary.generation_method)?)
    .bind(summary.provider.as_deref())
    .bind(summary.model.as_deref())
    .bind(summary.template.as_deref())
    .bind(summary.source_material_left_machine)
    .bind(enum_db(&summary.freshness)?)
    .bind(summary.source_revision_key.as_deref())
    .bind(record_json)
    .bind(summary.generated_at.to_rfc3339())
    .bind(summary.created_at.to_rfc3339())
    .bind(summary.updated_at.to_rfc3339())
    .bind(summary.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_summary_claim_on_pool(
    pool: &Pool<Sqlite>,
    claim: &WorkSummaryClaim,
) -> Result<()> {
    let record_json = serde_json::to_string(claim).context("serializing work summary claim")?;
    sqlx::query(
        r#"INSERT INTO work_summary_claims (
             claim_id, summary_id, work_id, workspace_id, claim_text, claim_kind,
             source_kind, source_id, record_hash, freshness, redaction_class,
             record_json, created_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(claim_id) DO UPDATE SET
             claim_text = excluded.claim_text,
             claim_kind = excluded.claim_kind,
             source_kind = excluded.source_kind,
             source_id = excluded.source_id,
             record_hash = excluded.record_hash,
             freshness = excluded.freshness,
             redaction_class = excluded.redaction_class,
             record_json = excluded.record_json,
             schema_version = excluded.schema_version
           WHERE work_summary_claims.workspace_id = excluded.workspace_id
             AND work_summary_claims.work_id = excluded.work_id
             AND work_summary_claims.summary_id = excluded.summary_id"#,
    )
    .bind(claim.claim_id.0.to_string())
    .bind(claim.summary_id.0.to_string())
    .bind(claim.work_id.0.to_string())
    .bind(claim.workspace_id.0.to_string())
    .bind(&claim.claim_text)
    .bind(claim.claim_kind.as_deref())
    .bind(&claim.source_kind)
    .bind(&claim.source_id)
    .bind(claim.record_hash.as_deref())
    .bind(enum_db(&claim.freshness)?)
    .bind(enum_db(&claim.redaction_class)?)
    .bind(record_json)
    .bind(claim.created_at.to_rfc3339())
    .bind(claim.schema_version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_work_search_doc_on_pool(pool: &Pool<Sqlite>, doc: &WorkSearchDoc) -> Result<()> {
    let record_json = serde_json::to_string(doc).context("serializing work search doc")?;
    sqlx::query(
        r#"INSERT INTO work_search_docs (
             doc_id, workspace_id, work_id, doc_type, source_id, source_kind, event_time,
             repo_root, path, branch, commit_sha, pr_owner, pr_repo, pr_number,
             agent_provider, freshness, redaction_class, title, search_text_redacted,
             record_json, created_at, updated_at, schema_version
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(doc_id) DO UPDATE SET
             work_id = excluded.work_id,
             doc_type = excluded.doc_type,
             source_id = excluded.source_id,
             source_kind = excluded.source_kind,
             event_time = excluded.event_time,
             repo_root = excluded.repo_root,
             path = excluded.path,
             branch = excluded.branch,
             commit_sha = excluded.commit_sha,
             pr_owner = excluded.pr_owner,
             pr_repo = excluded.pr_repo,
             pr_number = excluded.pr_number,
             agent_provider = excluded.agent_provider,
             freshness = excluded.freshness,
             redaction_class = excluded.redaction_class,
             title = excluded.title,
             search_text_redacted = excluded.search_text_redacted,
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             schema_version = excluded.schema_version
           WHERE work_search_docs.workspace_id = excluded.workspace_id"#,
    )
    .bind(doc.doc_id.0.to_string())
    .bind(doc.workspace_id.0.to_string())
    .bind(doc.work_id.0.to_string())
    .bind(&doc.doc_type)
    .bind(&doc.source_id)
    .bind(&doc.source_kind)
    .bind(doc.event_time.to_rfc3339())
    .bind(doc.repo_root.as_deref())
    .bind(doc.path.as_deref())
    .bind(doc.branch.as_deref())
    .bind(doc.commit_sha.as_deref())
    .bind(doc.pr_owner.as_deref())
    .bind(doc.pr_repo.as_deref())
    .bind(doc.pr_number)
    .bind(doc.agent_provider.as_deref())
    .bind(enum_db(&doc.freshness)?)
    .bind(enum_db(&doc.redaction_class)?)
    .bind(doc.title.as_deref())
    .bind(&doc.search_text_redacted)
    .bind(record_json)
    .bind(doc.created_at.to_rfc3339())
    .bind(doc.updated_at.to_rfc3339())
    .bind(doc.schema_version)
    .execute(pool)
    .await?;

    sqlx::query(r#"DELETE FROM work_search_docs_fts WHERE doc_id = ?"#)
        .bind(doc.doc_id.0.to_string())
        .execute(pool)
        .await?;
    sqlx::query(
        r#"INSERT INTO work_search_docs_fts (
             doc_id, workspace_id, work_id, doc_type, source_id, source_kind, event_time,
             repo_root, path, branch, commit_sha, pr_owner, pr_repo, pr_number,
             agent_provider, freshness, redaction_class, title, search_text_redacted
           )
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(doc.doc_id.0.to_string())
    .bind(doc.workspace_id.0.to_string())
    .bind(doc.work_id.0.to_string())
    .bind(&doc.doc_type)
    .bind(&doc.source_id)
    .bind(&doc.source_kind)
    .bind(doc.event_time.to_rfc3339())
    .bind(doc.repo_root.as_deref())
    .bind(doc.path.as_deref())
    .bind(doc.branch.as_deref())
    .bind(doc.commit_sha.as_deref())
    .bind(doc.pr_owner.as_deref())
    .bind(doc.pr_repo.as_deref())
    .bind(doc.pr_number)
    .bind(doc.agent_provider.as_deref())
    .bind(enum_db(&doc.freshness)?)
    .bind(enum_db(&doc.redaction_class)?)
    .bind(doc.title.as_deref())
    .bind(&doc.search_text_redacted)
    .execute(pool)
    .await?;
    Ok(())
}

fn decode_work_record_row(row: SqliteRow) -> Result<WorkRecord> {
    decode_record_json(row, "work record")
}

fn decode_work_record_link_row(row: SqliteRow) -> Result<WorkRecordLink> {
    decode_record_json(row, "work record link")
}

fn decode_work_event_row(row: SqliteRow) -> Result<WorkEvent> {
    decode_record_json(row, "work event")
}

fn decode_work_evidence_row(row: SqliteRow) -> Result<WorkEvidence> {
    decode_record_json(row, "work evidence")
}

fn decode_work_summary_row(row: SqliteRow) -> Result<WorkSummary> {
    decode_record_json(row, "work summary")
}

fn decode_work_summary_claim_row(row: SqliteRow) -> Result<WorkSummaryClaim> {
    decode_record_json(row, "work summary claim")
}

fn decode_work_search_hit_row(row: SqliteRow) -> Result<WorkSearchHit> {
    let record_json: String = row.try_get("record_json")?;
    let doc: WorkSearchDoc =
        serde_json::from_str(&record_json).context("decoding work search doc")?;
    validate_work_observability_schema_version(doc.schema_version, "work search doc")?;
    let score = row.try_get::<f64, _>("rank").unwrap_or(0.0);
    Ok(WorkSearchHit { doc, score })
}

fn decode_record_json<T>(row: SqliteRow, label: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + WorkSchemaVersion,
{
    let record_json: String = row.try_get("record_json")?;
    let record: T =
        serde_json::from_str(&record_json).with_context(|| format!("decoding {label}"))?;
    validate_work_observability_schema_version(record.schema_version(), label)?;
    Ok(record)
}

trait WorkSchemaVersion {
    fn schema_version(&self) -> i64;
}

impl WorkSchemaVersion for WorkRecord {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

impl WorkSchemaVersion for WorkRecordLink {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

impl WorkSchemaVersion for WorkEvent {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

impl WorkSchemaVersion for WorkEvidence {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

impl WorkSchemaVersion for WorkSummary {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

impl WorkSchemaVersion for WorkSummaryClaim {
    fn schema_version(&self) -> i64 {
        self.schema_version
    }
}

fn validate_work_observability_schema_version(schema_version: i64, label: &str) -> Result<()> {
    if schema_version != WORK_OBSERVABILITY_SCHEMA_VERSION {
        anyhow::bail!(
            "{label} schema_version {} is not supported; expected {}",
            schema_version,
            WORK_OBSERVABILITY_SCHEMA_VERSION
        );
    }
    Ok(())
}

fn enum_db<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value).context("serializing enum value")?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .context("enum value did not serialize to a string")
}

fn json_opt<T: Serialize>(value: &Option<T>) -> Result<Option<String>> {
    value
        .as_ref()
        .map(|value| serde_json::to_string(value).context("serializing JSON column"))
        .transpose()
}

fn value_json_opt(value: &Option<serde_json::Value>) -> Result<Option<String>> {
    value
        .as_ref()
        .map(|value| serde_json::to_string(value).context("serializing JSON value column"))
        .transpose()
}

fn fts_query(query: &str) -> String {
    let terms = query
        .split_whitespace()
        .map(|term| term.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-'))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        "\"\"".to_string()
    } else {
        terms.join(" ")
    }
}
