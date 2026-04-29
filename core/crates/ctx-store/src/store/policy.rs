use super::*;
use serde::de::DeserializeOwned;

impl Store {
    pub async fn upsert_daemon_enrollment(
        &self,
        enrollment: DaemonEnrollment,
    ) -> Result<DaemonEnrollment> {
        self.query(
            r#"INSERT INTO daemon_enrollments (
                   id,
                   account_id,
                   org_id,
                   org_membership_id,
                   membership_role,
                   plan_type,
                   status,
                   policy_signature_algorithm,
                   policy_signing_key,
                   active_policy_snapshot_id,
                   enrolled_at,
                   updated_at,
                   revoked_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(org_id) DO UPDATE SET
                   id = excluded.id,
                   account_id = excluded.account_id,
                   org_membership_id = excluded.org_membership_id,
                   membership_role = excluded.membership_role,
                   plan_type = excluded.plan_type,
                   status = excluded.status,
                   policy_signature_algorithm = excluded.policy_signature_algorithm,
                   policy_signing_key = excluded.policy_signing_key,
                   active_policy_snapshot_id = excluded.active_policy_snapshot_id,
                   enrolled_at = excluded.enrolled_at,
                   updated_at = excluded.updated_at,
                   revoked_at = excluded.revoked_at"#,
        )
        .bind(enrollment.id.0.to_string())
        .bind(enrollment.account_id.0.to_string())
        .bind(enrollment.org_id.0.to_string())
        .bind(enrollment.org_membership_id.0.to_string())
        .bind(enum_str(&enrollment.membership_role)?)
        .bind(enum_str(&enrollment.plan_type)?)
        .bind(enum_str(&enrollment.status)?)
        .bind(enum_str(&enrollment.policy_signature_algorithm)?)
        .bind(&enrollment.policy_signing_key)
        .bind(
            enrollment
                .active_policy_snapshot_id
                .map(|value| value.0.to_string()),
        )
        .bind(enrollment.enrolled_at.to_rfc3339())
        .bind(enrollment.updated_at.to_rfc3339())
        .bind(enrollment.revoked_at.map(|value| value.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        Ok(enrollment)
    }

    pub async fn get_daemon_enrollment_by_org_id(
        &self,
        org_id: OrgId,
    ) -> Result<Option<DaemonEnrollment>> {
        let row = self
            .query(
                r#"SELECT id, account_id, org_id, org_membership_id, membership_role, plan_type,
                          status, policy_signature_algorithm, policy_signing_key,
                          active_policy_snapshot_id, enrolled_at, updated_at, revoked_at
                   FROM daemon_enrollments
                   WHERE org_id = ?"#,
            )
            .bind(org_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_daemon_enrollment).transpose()
    }

    pub async fn list_daemon_enrollments(&self) -> Result<Vec<DaemonEnrollment>> {
        let rows = self
            .query(
                r#"SELECT id, account_id, org_id, org_membership_id, membership_role, plan_type,
                          status, policy_signature_algorithm, policy_signing_key,
                          active_policy_snapshot_id, enrolled_at, updated_at, revoked_at
                   FROM daemon_enrollments
                   ORDER BY updated_at DESC"#,
            )
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(map_daemon_enrollment).collect()
    }

    pub async fn upsert_org_policy_snapshot(
        &self,
        snapshot: OrgPolicySnapshot,
    ) -> Result<OrgPolicySnapshot> {
        self.query(
            r#"INSERT INTO org_policy_snapshots (
                   id,
                   org_id,
                   policy_version,
                   issued_at,
                   expires_at,
                   grace_expires_at,
                   allowed_providers_json,
                   allowed_models_json,
                   required_execution_environment,
                   allowed_network_profiles_json,
                   route_policy_json,
                   archive_policy_json,
                   features_json,
                   signature,
                   cached_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(org_id, policy_version) DO UPDATE SET
                   id = excluded.id,
                   issued_at = excluded.issued_at,
                   expires_at = excluded.expires_at,
                   grace_expires_at = excluded.grace_expires_at,
                   allowed_providers_json = excluded.allowed_providers_json,
                   allowed_models_json = excluded.allowed_models_json,
                   required_execution_environment = excluded.required_execution_environment,
                   allowed_network_profiles_json = excluded.allowed_network_profiles_json,
                   route_policy_json = excluded.route_policy_json,
                   archive_policy_json = excluded.archive_policy_json,
                   features_json = excluded.features_json,
                   signature = excluded.signature,
                   cached_at = excluded.cached_at"#,
        )
        .bind(snapshot.id.0.to_string())
        .bind(snapshot.org_id.0.to_string())
        .bind(&snapshot.policy_version)
        .bind(snapshot.issued_at.to_rfc3339())
        .bind(snapshot.expires_at.to_rfc3339())
        .bind(snapshot.grace_expires_at.to_rfc3339())
        .bind(serialize_optional_json(
            snapshot.allowed_providers.as_ref(),
        )?)
        .bind(serialize_json(&snapshot.allowed_models)?)
        .bind(
            snapshot
                .required_execution_environment
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(serialize_json(&snapshot.allowed_network_profiles)?)
        .bind(serialize_json(&snapshot.route_policy)?)
        .bind(serialize_json(&snapshot.archive_policy)?)
        .bind(serialize_json(&snapshot.features)?)
        .bind(&snapshot.signature)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(snapshot)
    }

    pub async fn upsert_workspace_policy_overlay(
        &self,
        overlay: WorkspacePolicyOverlay,
    ) -> Result<WorkspacePolicyOverlay> {
        self.query(
            r#"INSERT INTO workspace_policy_overlays (
                   workspace_id,
                   org_id,
                   allowed_providers_json,
                   allowed_models_json,
                   required_execution_environment,
                   allowed_network_profiles_json,
                   allowed_route_types_json,
                   features_json,
                   updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(workspace_id) DO UPDATE SET
                   org_id = excluded.org_id,
                   allowed_providers_json = excluded.allowed_providers_json,
                   allowed_models_json = excluded.allowed_models_json,
                   required_execution_environment = excluded.required_execution_environment,
                   allowed_network_profiles_json = excluded.allowed_network_profiles_json,
                   allowed_route_types_json = excluded.allowed_route_types_json,
                   features_json = excluded.features_json,
                   updated_at = excluded.updated_at"#,
        )
        .bind(overlay.workspace_id.0.to_string())
        .bind(overlay.org_id.0.to_string())
        .bind(serialize_optional_json(overlay.allowed_providers.as_ref())?)
        .bind(serialize_json(&overlay.allowed_models)?)
        .bind(
            overlay
                .required_execution_environment
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(serialize_optional_json(
            overlay.allowed_network_profiles.as_ref(),
        )?)
        .bind(serialize_optional_json(
            overlay.allowed_route_types.as_ref(),
        )?)
        .bind(serialize_json(&overlay.features)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(overlay)
    }

    pub async fn get_workspace_policy_overlay(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspacePolicyOverlay>> {
        let row = self
            .query(
                r#"SELECT workspace_id, org_id, allowed_providers_json, allowed_models_json,
                          required_execution_environment, allowed_network_profiles_json,
                          allowed_route_types_json, features_json
                   FROM workspace_policy_overlays
                   WHERE workspace_id = ?"#,
            )
            .bind(workspace_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_workspace_policy_overlay).transpose()
    }

    pub async fn get_org_policy_snapshot(
        &self,
        id: OrgPolicySnapshotId,
    ) -> Result<Option<OrgPolicySnapshot>> {
        let row = self
            .query(
                r#"SELECT id, org_id, policy_version, issued_at, expires_at, grace_expires_at,
                          allowed_providers_json, allowed_models_json,
                          required_execution_environment, allowed_network_profiles_json,
                          route_policy_json, archive_policy_json, features_json, signature
                   FROM org_policy_snapshots
                   WHERE id = ?"#,
            )
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_org_policy_snapshot).transpose()
    }

    pub async fn get_latest_org_policy_snapshot(
        &self,
        org_id: OrgId,
    ) -> Result<Option<OrgPolicySnapshot>> {
        let row = self
            .query(
                r#"SELECT id, org_id, policy_version, issued_at, expires_at, grace_expires_at,
                          allowed_providers_json, allowed_models_json,
                          required_execution_environment, allowed_network_profiles_json,
                          route_policy_json, archive_policy_json, features_json, signature
                   FROM org_policy_snapshots
                   WHERE org_id = ?
                   ORDER BY issued_at DESC, cached_at DESC
                   LIMIT 1"#,
            )
            .bind(org_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_org_policy_snapshot).transpose()
    }

    pub async fn create_run_grant(&self, run_grant: RunGrant) -> Result<RunGrant> {
        self.query(
            r#"INSERT INTO run_grants (
                   id,
                   run_id,
                   session_id,
                   workspace_id,
                   account_id,
                   org_id,
                   membership_role,
                   policy_version,
                   provider_id,
                   model_id,
                   execution_environment,
                   network_profile,
                   route_type,
                   archive_mode,
                   issued_at,
                   expires_at,
                   decision_source
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(run_grant.id.0.to_string())
        .bind(run_grant.run_id.0.to_string())
        .bind(run_grant.session_id.0.to_string())
        .bind(run_grant.workspace_id.0.to_string())
        .bind(run_grant.account_id.0.to_string())
        .bind(run_grant.org_id.0.to_string())
        .bind(
            run_grant
                .membership_role
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(&run_grant.policy_version)
        .bind(&run_grant.provider_id)
        .bind(&run_grant.model_id)
        .bind(enum_str(&run_grant.execution_environment)?)
        .bind(enum_str(&run_grant.network_profile)?)
        .bind(
            run_grant
                .route_type
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(enum_str(&run_grant.archive_mode)?)
        .bind(run_grant.issued_at.to_rfc3339())
        .bind(run_grant.expires_at.map(|value| value.to_rfc3339()))
        .bind(enum_str(&run_grant.decision_source)?)
        .execute(&self.pool)
        .await?;

        Ok(run_grant)
    }

    pub async fn get_run_grant(&self, id: RunGrantId) -> Result<Option<RunGrant>> {
        let row = self
            .query(
                r#"SELECT id, run_id, session_id, workspace_id, account_id, org_id,
                          membership_role, policy_version, provider_id, model_id,
                          execution_environment, network_profile, route_type, archive_mode,
                          issued_at, expires_at, decision_source
                   FROM run_grants
                   WHERE id = ?"#,
            )
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_run_grant).transpose()
    }

    pub async fn get_run_grant_by_run_id(&self, run_id: RunId) -> Result<Option<RunGrant>> {
        let row = self
            .query(
                r#"SELECT id, run_id, session_id, workspace_id, account_id, org_id,
                          membership_role, policy_version, provider_id, model_id,
                          execution_environment, network_profile, route_type, archive_mode,
                          issued_at, expires_at, decision_source
                   FROM run_grants
                   WHERE run_id = ?"#,
            )
            .bind(run_id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(map_run_grant).transpose()
    }

    pub async fn append_policy_decision_event(
        &self,
        event: PolicyDecisionEvent,
    ) -> Result<PolicyDecisionEvent> {
        self.query(
            r#"INSERT INTO policy_decision_events (
                   id,
                   run_grant_id,
                   run_id,
                   session_id,
                   workspace_id,
                   account_id,
                   org_id,
                   policy_snapshot_id,
                   policy_version,
                   decision_source,
                   outcome,
                   deny_reason,
                   requested_provider_id,
                   requested_model_id,
                   requested_execution_environment,
                   requested_network_profile,
                   requested_route_type,
                   detail,
                   created_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(event.id.0.to_string())
        .bind(event.run_grant_id.map(|value| value.0.to_string()))
        .bind(event.run_id.map(|value| value.0.to_string()))
        .bind(event.session_id.map(|value| value.0.to_string()))
        .bind(event.workspace_id.map(|value| value.0.to_string()))
        .bind(event.account_id.map(|value| value.0.to_string()))
        .bind(event.org_id.map(|value| value.0.to_string()))
        .bind(event.policy_snapshot_id.map(|value| value.0.to_string()))
        .bind(&event.policy_version)
        .bind(enum_str(&event.decision_source)?)
        .bind(enum_str(&event.outcome)?)
        .bind(
            event
                .deny_reason
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(&event.requested_provider_id)
        .bind(&event.requested_model_id)
        .bind(
            event
                .requested_execution_environment
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(
            event
                .requested_network_profile
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(
            event
                .requested_route_type
                .map(|value| enum_str(&value))
                .transpose()?,
        )
        .bind(&event.detail)
        .bind(event.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(event)
    }

    pub async fn list_policy_decision_events_for_run(
        &self,
        run_id: RunId,
    ) -> Result<Vec<PolicyDecisionEvent>> {
        let rows = self
            .query(
                r#"SELECT id, run_grant_id, run_id, session_id, workspace_id, account_id,
                          org_id, policy_snapshot_id, policy_version, decision_source, outcome,
                          deny_reason, requested_provider_id, requested_model_id,
                          requested_execution_environment, requested_network_profile,
                          requested_route_type, detail, created_at
                   FROM policy_decision_events
                   WHERE run_id = ?
                   ORDER BY created_at ASC, id ASC"#,
            )
            .bind(run_id.0.to_string())
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(map_policy_decision_event).collect()
    }
}

fn map_daemon_enrollment(row: SqliteRow) -> Result<DaemonEnrollment> {
    Ok(DaemonEnrollment {
        id: parse_uuid_id(&row.try_get::<String, _>("id")?, DaemonEnrollmentId)?,
        account_id: parse_uuid_id(&row.try_get::<String, _>("account_id")?, AccountId)?,
        org_id: parse_uuid_id(&row.try_get::<String, _>("org_id")?, OrgId)?,
        org_membership_id: parse_uuid_id(
            &row.try_get::<String, _>("org_membership_id")?,
            OrgMembershipId,
        )?,
        membership_role: parse_string_enum(&row.try_get::<String, _>("membership_role")?)?,
        plan_type: parse_string_enum(&row.try_get::<String, _>("plan_type")?)?,
        status: parse_string_enum(&row.try_get::<String, _>("status")?)?,
        policy_signature_algorithm: parse_string_enum(
            &row.try_get::<String, _>("policy_signature_algorithm")?,
        )?,
        policy_signing_key: row.try_get("policy_signing_key")?,
        active_policy_snapshot_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("active_policy_snapshot_id")?,
            OrgPolicySnapshotId,
        )?,
        enrolled_at: parse_dt(&row.try_get::<String, _>("enrolled_at")?)?,
        updated_at: parse_dt(&row.try_get::<String, _>("updated_at")?)?,
        revoked_at: row
            .try_get::<Option<String>, _>("revoked_at")?
            .map(|value| parse_dt(&value))
            .transpose()?,
    })
}

fn map_workspace_policy_overlay(row: SqliteRow) -> Result<WorkspacePolicyOverlay> {
    let allowed_providers_json: Option<String> = row.try_get("allowed_providers_json")?;
    let required_execution_environment: Option<String> =
        row.try_get("required_execution_environment")?;
    let allowed_network_profiles_json: Option<String> =
        row.try_get("allowed_network_profiles_json")?;
    let allowed_route_types_json: Option<String> = row.try_get("allowed_route_types_json")?;

    Ok(WorkspacePolicyOverlay {
        workspace_id: parse_uuid_id(&row.try_get::<String, _>("workspace_id")?, WorkspaceId)?,
        org_id: parse_uuid_id(&row.try_get::<String, _>("org_id")?, OrgId)?,
        allowed_providers: allowed_providers_json
            .as_deref()
            .map(parse_json)
            .transpose()?,
        allowed_models: parse_json(&row.try_get::<String, _>("allowed_models_json")?)?,
        required_execution_environment: required_execution_environment
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        allowed_network_profiles: allowed_network_profiles_json
            .as_deref()
            .map(parse_json)
            .transpose()?,
        allowed_route_types: allowed_route_types_json
            .as_deref()
            .map(parse_json)
            .transpose()?,
        features: parse_json(&row.try_get::<String, _>("features_json")?)?,
    })
}

fn map_org_policy_snapshot(row: SqliteRow) -> Result<OrgPolicySnapshot> {
    let allowed_providers_json: Option<String> = row.try_get("allowed_providers_json")?;
    let required_execution_environment: Option<String> =
        row.try_get("required_execution_environment")?;

    Ok(OrgPolicySnapshot {
        id: parse_uuid_id(&row.try_get::<String, _>("id")?, OrgPolicySnapshotId)?,
        org_id: parse_uuid_id(&row.try_get::<String, _>("org_id")?, OrgId)?,
        policy_version: row.try_get("policy_version")?,
        issued_at: parse_dt(&row.try_get::<String, _>("issued_at")?)?,
        expires_at: parse_dt(&row.try_get::<String, _>("expires_at")?)?,
        grace_expires_at: parse_dt(&row.try_get::<String, _>("grace_expires_at")?)?,
        allowed_providers: allowed_providers_json
            .as_deref()
            .map(parse_json)
            .transpose()?,
        allowed_models: parse_json(&row.try_get::<String, _>("allowed_models_json")?)?,
        required_execution_environment: required_execution_environment
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        allowed_network_profiles: parse_json(
            &row.try_get::<String, _>("allowed_network_profiles_json")?,
        )?,
        route_policy: parse_json(&row.try_get::<String, _>("route_policy_json")?)?,
        archive_policy: parse_json(&row.try_get::<String, _>("archive_policy_json")?)?,
        features: parse_json(&row.try_get::<String, _>("features_json")?)?,
        signature: row.try_get("signature")?,
    })
}

fn map_run_grant(row: SqliteRow) -> Result<RunGrant> {
    Ok(RunGrant {
        id: parse_uuid_id(&row.try_get::<String, _>("id")?, RunGrantId)?,
        run_id: parse_uuid_id(&row.try_get::<String, _>("run_id")?, RunId)?,
        session_id: parse_uuid_id(&row.try_get::<String, _>("session_id")?, SessionId)?,
        workspace_id: parse_uuid_id(&row.try_get::<String, _>("workspace_id")?, WorkspaceId)?,
        account_id: parse_uuid_id(&row.try_get::<String, _>("account_id")?, AccountId)?,
        org_id: parse_uuid_id(&row.try_get::<String, _>("org_id")?, OrgId)?,
        membership_role: row
            .try_get::<Option<String>, _>("membership_role")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        policy_version: row.try_get("policy_version")?,
        provider_id: row.try_get("provider_id")?,
        model_id: row.try_get("model_id")?,
        execution_environment: parse_string_enum(
            &row.try_get::<String, _>("execution_environment")?,
        )?,
        network_profile: parse_string_enum(&row.try_get::<String, _>("network_profile")?)?,
        route_type: row
            .try_get::<Option<String>, _>("route_type")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        archive_mode: parse_string_enum(&row.try_get::<String, _>("archive_mode")?)?,
        issued_at: parse_dt(&row.try_get::<String, _>("issued_at")?)?,
        expires_at: row
            .try_get::<Option<String>, _>("expires_at")?
            .map(|value| parse_dt(&value))
            .transpose()?,
        decision_source: parse_string_enum(&row.try_get::<String, _>("decision_source")?)?,
    })
}

fn map_policy_decision_event(row: SqliteRow) -> Result<PolicyDecisionEvent> {
    Ok(PolicyDecisionEvent {
        id: parse_uuid_id(&row.try_get::<String, _>("id")?, PolicyDecisionEventId)?,
        run_grant_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("run_grant_id")?,
            RunGrantId,
        )?,
        run_id: parse_optional_uuid_id(row.try_get::<Option<String>, _>("run_id")?, RunId)?,
        session_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("session_id")?,
            SessionId,
        )?,
        workspace_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("workspace_id")?,
            WorkspaceId,
        )?,
        account_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("account_id")?,
            AccountId,
        )?,
        org_id: parse_optional_uuid_id(row.try_get::<Option<String>, _>("org_id")?, OrgId)?,
        policy_snapshot_id: parse_optional_uuid_id(
            row.try_get::<Option<String>, _>("policy_snapshot_id")?,
            OrgPolicySnapshotId,
        )?,
        policy_version: row.try_get("policy_version")?,
        decision_source: parse_string_enum(&row.try_get::<String, _>("decision_source")?)?,
        outcome: parse_string_enum(&row.try_get::<String, _>("outcome")?)?,
        deny_reason: row
            .try_get::<Option<String>, _>("deny_reason")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        requested_provider_id: row.try_get("requested_provider_id")?,
        requested_model_id: row.try_get("requested_model_id")?,
        requested_execution_environment: row
            .try_get::<Option<String>, _>("requested_execution_environment")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        requested_network_profile: row
            .try_get::<Option<String>, _>("requested_network_profile")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        requested_route_type: row
            .try_get::<Option<String>, _>("requested_route_type")?
            .as_deref()
            .map(parse_string_enum)
            .transpose()?,
        detail: row.try_get("detail")?,
        created_at: parse_dt(&row.try_get::<String, _>("created_at")?)?,
    })
}

fn serialize_json<T>(value: &T) -> Result<String>
where
    T: Serialize,
{
    serde_json::to_string(value).context("serialize json column")
}

fn serialize_optional_json<T>(value: Option<&T>) -> Result<Option<String>>
where
    T: Serialize,
{
    value.map(serialize_json).transpose()
}

fn parse_json<T>(value: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(value).context("parse json column")
}

fn enum_str<T>(value: &T) -> Result<String>
where
    T: Serialize,
{
    match serde_json::to_value(value).context("serialize enum")? {
        serde_json::Value::String(value) => Ok(value),
        _ => anyhow::bail!("expected enum to serialize as string"),
    }
}

fn parse_string_enum<T>(value: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .context("parse enum from string column")
}

fn parse_uuid_id<T>(value: &str, build: fn(uuid::Uuid) -> T) -> Result<T> {
    Ok(build(uuid::Uuid::parse_str(value)?))
}

fn parse_optional_uuid_id<T>(
    value: Option<String>,
    build: fn(uuid::Uuid) -> T,
) -> Result<Option<T>> {
    value
        .as_deref()
        .map(|value| parse_uuid_id(value, build))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::collections::BTreeMap;

    fn sample_enrollment(org_id: OrgId, snapshot_id: OrgPolicySnapshotId) -> DaemonEnrollment {
        let now = Utc::now();
        DaemonEnrollment {
            id: DaemonEnrollmentId::new(),
            account_id: AccountId::new(),
            org_id,
            org_membership_id: OrgMembershipId::new(),
            membership_role: OrgMembershipRole::Admin,
            plan_type: PlanType::Team,
            status: DaemonEnrollmentStatus::Active,
            policy_signature_algorithm: PolicySignatureAlgorithm::Hs256,
            policy_signing_key: "test-policy-signing-key".to_string(),
            active_policy_snapshot_id: Some(snapshot_id),
            enrolled_at: now,
            updated_at: now,
            revoked_at: None,
        }
    }

    fn sample_snapshot(org_id: OrgId) -> OrgPolicySnapshot {
        let now = Utc::now();
        let mut allowed_models = BTreeMap::new();
        allowed_models.insert("anthropic".to_string(), vec!["claude-sonnet-4".to_string()]);

        let mut features = BTreeMap::new();
        features.insert("mobile_relay".to_string(), PolicyFeatureState::Enabled);

        OrgPolicySnapshot {
            id: OrgPolicySnapshotId::new(),
            org_id,
            policy_version: "2026-04-28.1".to_string(),
            issued_at: now,
            expires_at: now + Duration::minutes(30),
            grace_expires_at: now + Duration::minutes(60),
            allowed_providers: Some(vec!["anthropic".to_string()]),
            allowed_models,
            required_execution_environment: Some(RequiredExecutionEnvironment::Sandbox),
            allowed_network_profiles: vec![NetworkProfile::LlmOnly],
            route_policy: RoutePolicy {
                allowed_route_types: vec![RouteType::CtxManaged],
            },
            archive_policy: ArchivePolicy {
                mode: ArchiveMode::OrgTranscript,
            },
            features,
            signature: "signed".to_string(),
        }
    }

    #[tokio::test]
    async fn daemon_enrollment_and_policy_snapshot_roundtrip() {
        let (_dir, store) = crate::store::tests::setup_store().await;
        let org_id = OrgId::new();
        let snapshot = sample_snapshot(org_id);
        store
            .upsert_org_policy_snapshot(snapshot.clone())
            .await
            .unwrap();

        let enrollment = sample_enrollment(org_id, snapshot.id);
        let stored_enrollment = store
            .upsert_daemon_enrollment(enrollment.clone())
            .await
            .unwrap();
        assert_eq!(stored_enrollment, enrollment);

        let loaded_enrollment = store
            .get_daemon_enrollment_by_org_id(org_id)
            .await
            .unwrap()
            .expect("enrollment");
        assert_eq!(loaded_enrollment, enrollment);

        let loaded_snapshot = store
            .get_latest_org_policy_snapshot(org_id)
            .await
            .unwrap()
            .expect("snapshot");
        assert_eq!(loaded_snapshot, snapshot);

        let overlay = WorkspacePolicyOverlay {
            workspace_id: crate::store::tests::create_session_with_turn(&store, None)
                .await
                .0
                .workspace_id,
            org_id,
            allowed_providers: Some(vec!["anthropic".to_string()]),
            allowed_models: BTreeMap::new(),
            required_execution_environment: Some(RequiredExecutionEnvironment::Sandbox),
            allowed_network_profiles: Some(vec![NetworkProfile::LlmOnly]),
            allowed_route_types: Some(vec![RouteType::CtxManaged]),
            features: BTreeMap::new(),
        };
        store
            .upsert_workspace_policy_overlay(overlay.clone())
            .await
            .unwrap();
        let loaded_overlay = store
            .get_workspace_policy_overlay(overlay.workspace_id)
            .await
            .unwrap()
            .expect("overlay");
        assert_eq!(loaded_overlay, overlay);
    }

    #[tokio::test]
    async fn run_grant_and_policy_decision_event_roundtrip() {
        let (_dir, store) = crate::store::tests::setup_store().await;
        let (session, _turn_id) = crate::store::tests::create_session_with_turn(&store, None).await;
        let run_id = RunId::new();
        let issued_at = Utc::now();

        let run_grant = RunGrant {
            id: RunGrantId::new(),
            run_id,
            session_id: session.id,
            workspace_id: session.workspace_id,
            account_id: AccountId::new(),
            org_id: OrgId::new(),
            membership_role: Some(OrgMembershipRole::Member),
            policy_version: "2026-04-28.1".to_string(),
            provider_id: "anthropic".to_string(),
            model_id: "claude-sonnet-4".to_string(),
            execution_environment: ExecutionEnvironment::Sandbox,
            network_profile: NetworkProfile::LlmOnly,
            route_type: Some(RouteType::CtxManaged),
            archive_mode: ArchiveMode::OrgTranscript,
            issued_at,
            expires_at: Some(issued_at + Duration::minutes(60)),
            decision_source: PolicyDecisionSource::CachedPolicy,
        };

        store.create_run_grant(run_grant.clone()).await.unwrap();
        let loaded_grant = store
            .get_run_grant_by_run_id(run_id)
            .await
            .unwrap()
            .expect("run grant");
        assert_eq!(loaded_grant, run_grant);

        let event = PolicyDecisionEvent {
            id: PolicyDecisionEventId::new(),
            run_grant_id: Some(run_grant.id),
            run_id: Some(run_id),
            session_id: Some(session.id),
            workspace_id: Some(session.workspace_id),
            account_id: Some(run_grant.account_id),
            org_id: Some(run_grant.org_id),
            policy_snapshot_id: Some(OrgPolicySnapshotId::new()),
            policy_version: Some(run_grant.policy_version.clone()),
            decision_source: PolicyDecisionSource::CachedPolicy,
            outcome: PolicyDecisionOutcome::Denied,
            deny_reason: Some(PolicyDenyReason::PersonalRouteNotAllowed),
            requested_provider_id: Some(run_grant.provider_id.clone()),
            requested_model_id: Some(run_grant.model_id.clone()),
            requested_execution_environment: Some(ExecutionEnvironment::Sandbox),
            requested_network_profile: Some(NetworkProfile::LlmOnly),
            requested_route_type: Some(RouteType::UserApiKey),
            detail: Some("personal route blocked by org policy".to_string()),
            created_at: issued_at,
        };

        store
            .append_policy_decision_event(event.clone())
            .await
            .unwrap();
        let events = store
            .list_policy_decision_events_for_run(run_id)
            .await
            .unwrap();
        assert_eq!(events, vec![event]);
    }
}
