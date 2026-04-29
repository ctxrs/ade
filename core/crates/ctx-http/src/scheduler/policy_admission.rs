use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::json;

use ctx_core::ids::{PolicyDecisionEventId, RunGrantId, RunId};
use ctx_core::models::{
    ArchiveMode, ArchiveVisibility, AuditActor, AuditActorKind, AuditEvent, AuditEventKind,
    DaemonEnrollment, DaemonEnrollmentStatus, ExecutionEnvironment, NetworkProfile,
    OrgPolicySnapshot, PolicyDecisionEvent, PolicyDecisionOutcome, PolicyDecisionSource,
    PolicyDenyReason, RouteType, RunArchiveState, RunGrant, RunRecord, RunStatus, Session,
};
use ctx_harness_sources::HarnessSourceKind;
use ctx_sandbox_contract::ContainerNetworkMode;
use ctx_store::Store;

use crate::daemon::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdmissionRouteSource {
    Subscription,
    Endpoint,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TurnAdmissionRequest<'a> {
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) provider_id: &'a str,
    pub(super) model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) network_profile: NetworkProfile,
    pub(super) route_type: RouteType,
}

#[derive(Clone, Debug)]
pub(super) enum TurnAdmission {
    Local,
    OrgManaged { run_grant: RunGrant },
}

impl From<HarnessSourceKind> for AdmissionRouteSource {
    fn from(value: HarnessSourceKind) -> Self {
        match value {
            HarnessSourceKind::Subscription => Self::Subscription,
            HarnessSourceKind::Endpoint => Self::Endpoint,
        }
    }
}

pub(super) fn route_type_for_source(source: AdmissionRouteSource) -> RouteType {
    match source {
        AdmissionRouteSource::Subscription => RouteType::UserProviderAccount,
        AdmissionRouteSource::Endpoint => RouteType::UserApiKey,
    }
}

pub(super) fn network_profile_for_container_mode(mode: ContainerNetworkMode) -> NetworkProfile {
    match mode {
        ContainerNetworkMode::LlmOnly => NetworkProfile::LlmOnly,
        ContainerNetworkMode::Allowlist => NetworkProfile::Allowlist,
        ContainerNetworkMode::All => NetworkProfile::All,
    }
}

fn archive_visibility_for_mode(mode: ArchiveMode) -> ArchiveVisibility {
    match mode {
        ArchiveMode::LocalOnly => ArchiveVisibility::LocalOnly,
        ArchiveMode::AccountPrivate => ArchiveVisibility::AccountPrivate,
        ArchiveMode::OrgSummary => ArchiveVisibility::OrgSummary,
        ArchiveMode::OrgTranscript => ArchiveVisibility::OrgTranscript,
        ArchiveMode::OrgEvidence => ArchiveVisibility::OrgEvidence,
    }
}

async fn snapshot_for_enrollment(
    global_store: &Store,
    enrollment: &DaemonEnrollment,
) -> Result<Option<OrgPolicySnapshot>> {
    if let Some(snapshot_id) = enrollment.active_policy_snapshot_id {
        return global_store.get_org_policy_snapshot(snapshot_id).await;
    }
    global_store
        .get_latest_org_policy_snapshot(enrollment.org_id)
        .await
}

fn decision_event(
    request: &TurnAdmissionRequest<'_>,
    enrollment: Option<&DaemonEnrollment>,
    snapshot: Option<&OrgPolicySnapshot>,
    run_grant_id: Option<RunGrantId>,
    outcome: PolicyDecisionOutcome,
    deny_reason: Option<PolicyDenyReason>,
    detail: Option<String>,
) -> PolicyDecisionEvent {
    PolicyDecisionEvent {
        id: PolicyDecisionEventId::new(),
        run_grant_id,
        run_id: Some(request.run_id),
        session_id: Some(request.session.id),
        workspace_id: Some(request.session.workspace_id),
        account_id: enrollment.map(|value| value.account_id),
        org_id: enrollment
            .map(|value| value.org_id)
            .or_else(|| snapshot.map(|value| value.org_id)),
        policy_snapshot_id: snapshot.map(|value| value.id),
        policy_version: snapshot.map(|value| value.policy_version.clone()),
        decision_source: PolicyDecisionSource::CachedPolicy,
        outcome,
        deny_reason,
        requested_provider_id: Some(request.provider_id.to_string()),
        requested_model_id: Some(request.model_id.to_string()),
        requested_execution_environment: Some(request.execution_environment),
        requested_network_profile: Some(request.network_profile),
        requested_route_type: Some(request.route_type),
        detail,
        created_at: Utc::now(),
    }
}

async fn upsert_failed_org_run(
    store: &Store,
    request: &TurnAdmissionRequest<'_>,
    enrollment: Option<&DaemonEnrollment>,
) -> Result<()> {
    let now = Utc::now();
    store
        .upsert_run(RunRecord {
            id: request.run_id,
            session_id: request.session.id,
            task_id: request.session.task_id,
            workspace_id: request.session.workspace_id,
            worktree_id: request.session.worktree_id,
            parent_run_id: None,
            account_id: enrollment.map(|value| value.account_id),
            org_id: enrollment.map(|value| value.org_id),
            run_grant_id: None,
            status: RunStatus::Failed,
            archive_state: RunArchiveState::Active,
            archive_visibility: ArchiveVisibility::LocalOnly,
            retention_policy: None,
            created_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            archived_at: None,
            updated_at: now,
        })
        .await?;
    Ok(())
}

async fn deny_org_run<T>(
    store: &Store,
    request: &TurnAdmissionRequest<'_>,
    enrollment: Option<&DaemonEnrollment>,
    snapshot: Option<&OrgPolicySnapshot>,
    deny_reason: PolicyDenyReason,
    detail: impl Into<String>,
) -> Result<T> {
    let detail = detail.into();
    let event = decision_event(
        request,
        enrollment,
        snapshot,
        None,
        PolicyDecisionOutcome::Denied,
        Some(deny_reason),
        Some(detail.clone()),
    );
    store.append_policy_decision_event(event).await?;
    upsert_failed_org_run(store, request, enrollment).await?;
    Err(anyhow!("org policy denied run: {deny_reason:?}: {detail}"))
}

pub(super) async fn admit_turn(
    state: &Arc<AppState>,
    store: &Store,
    request: TurnAdmissionRequest<'_>,
) -> Result<TurnAdmission> {
    let Some(overlay) = store
        .get_workspace_policy_overlay(request.session.workspace_id)
        .await
        .context("load workspace org policy overlay")?
    else {
        return Ok(TurnAdmission::Local);
    };

    let Some(enrollment) = state
        .global_store()
        .get_daemon_enrollment_by_org_id(overlay.org_id)
        .await
        .context("load daemon enrollment")?
    else {
        return deny_org_run(
            store,
            &request,
            None,
            None,
            PolicyDenyReason::DaemonEnrollmentMissing,
            "workspace is bound to an org but this daemon has no active enrollment for it",
        )
        .await;
    };

    if !matches!(enrollment.status, DaemonEnrollmentStatus::Active) {
        return deny_org_run(
            store,
            &request,
            Some(&enrollment),
            None,
            PolicyDenyReason::DaemonEnrollmentRevoked,
            "daemon enrollment is not active",
        )
        .await;
    }

    let Some(snapshot) = snapshot_for_enrollment(state.global_store(), &enrollment)
        .await
        .context("load active org policy snapshot")?
    else {
        return deny_org_run(
            store,
            &request,
            Some(&enrollment),
            None,
            PolicyDenyReason::PolicySnapshotMissing,
            "daemon enrollment has no cached org policy snapshot",
        )
        .await;
    };

    if snapshot.org_id != overlay.org_id || snapshot.org_id != enrollment.org_id {
        return deny_org_run(
            store,
            &request,
            Some(&enrollment),
            Some(&snapshot),
            PolicyDenyReason::WorkspaceOrgMismatch,
            "workspace org binding does not match enrollment or policy snapshot",
        )
        .await;
    }

    if let Err(err) =
        crate::policy_signature::verify_policy_snapshot_signature(&enrollment, &snapshot)
    {
        return deny_org_run(
            store,
            &request,
            Some(&enrollment),
            Some(&snapshot),
            PolicyDenyReason::PolicySignatureInvalid,
            format!("policy snapshot signature verification failed: {err:#}"),
        )
        .await;
    }

    if let Err(reason) = ctx_core::models::org_policy_allows_run(
        &snapshot,
        Some(&overlay),
        request.provider_id,
        request.model_id,
        request.execution_environment,
        request.network_profile,
        Some(request.route_type),
        Utc::now(),
    ) {
        return deny_org_run(
            store,
            &request,
            Some(&enrollment),
            Some(&snapshot),
            reason,
            "run request does not satisfy cached org policy",
        )
        .await;
    }

    let effective_policy =
        ctx_core::models::merge_org_policy_with_overlay(&snapshot, Some(&overlay));
    let now = Utc::now();
    let archive_mode = effective_policy.archive_policy.mode;
    let run_grant = RunGrant {
        id: RunGrantId::new(),
        run_id: request.run_id,
        session_id: request.session.id,
        workspace_id: request.session.workspace_id,
        account_id: enrollment.account_id,
        org_id: enrollment.org_id,
        membership_role: Some(enrollment.membership_role),
        policy_version: snapshot.policy_version.clone(),
        provider_id: request.provider_id.to_string(),
        model_id: request.model_id.to_string(),
        execution_environment: request.execution_environment,
        network_profile: request.network_profile,
        route_type: Some(request.route_type),
        archive_mode,
        issued_at: now,
        expires_at: Some(snapshot.grace_expires_at),
        decision_source: PolicyDecisionSource::CachedPolicy,
    };
    store.create_run_grant(run_grant.clone()).await?;
    let event = decision_event(
        &request,
        Some(&enrollment),
        Some(&snapshot),
        Some(run_grant.id),
        PolicyDecisionOutcome::Granted,
        None,
        Some("run admitted by cached org policy snapshot".to_string()),
    );
    store.append_policy_decision_event(event).await?;
    store
        .upsert_run(RunRecord {
            id: request.run_id,
            session_id: request.session.id,
            task_id: request.session.task_id,
            workspace_id: request.session.workspace_id,
            worktree_id: request.session.worktree_id,
            parent_run_id: None,
            account_id: Some(enrollment.account_id),
            org_id: Some(enrollment.org_id),
            run_grant_id: Some(run_grant.id),
            status: RunStatus::Running,
            archive_state: RunArchiveState::Active,
            archive_visibility: archive_visibility_for_mode(archive_mode),
            retention_policy: None,
            created_at: now,
            started_at: Some(now),
            completed_at: None,
            archived_at: None,
            updated_at: now,
        })
        .await?;
    store
        .append_run_audit_event(AuditEvent {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: request.session.workspace_id,
            task_id: Some(request.session.task_id),
            session_id: Some(request.session.id),
            run_id: Some(request.run_id),
            account_id: Some(enrollment.account_id),
            org_id: Some(enrollment.org_id),
            actor: AuditActor {
                kind: AuditActorKind::Account,
                account_id: Some(enrollment.account_id),
                org_id: Some(enrollment.org_id),
                membership_role: Some(format!("{:?}", enrollment.membership_role)),
            },
            event_kind: AuditEventKind::RunCreated,
            archive_visibility: Some(archive_visibility_for_mode(archive_mode)),
            retention_policy: None,
            payload_json: json!({
                "run_grant_id": run_grant.id.0,
                "policy_version": snapshot.policy_version,
                "decision_source": "cached_policy",
            }),
            created_at: now,
        })
        .await?;

    Ok(TurnAdmission::OrgManaged { run_grant })
}

pub(super) async fn update_run_terminal_status(
    store: &Store,
    run_id: Option<RunId>,
    status: RunStatus,
) {
    let Some(run_id) = run_id else {
        return;
    };
    if let Err(err) = store
        .update_run_status(run_id, status, Some(Utc::now()))
        .await
    {
        tracing::warn!(
            run_id = %run_id.0,
            "failed to update durable run terminal status: {err:#}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_runtime_source_to_personal_route_type() {
        assert_eq!(
            route_type_for_source(AdmissionRouteSource::Subscription),
            RouteType::UserProviderAccount
        );
        assert_eq!(
            route_type_for_source(AdmissionRouteSource::Endpoint),
            RouteType::UserApiKey
        );
    }

    #[test]
    fn maps_container_network_mode_to_policy_profile() {
        assert_eq!(
            network_profile_for_container_mode(ContainerNetworkMode::LlmOnly),
            NetworkProfile::LlmOnly
        );
        assert_eq!(
            network_profile_for_container_mode(ContainerNetworkMode::Allowlist),
            NetworkProfile::Allowlist
        );
        assert_eq!(
            network_profile_for_container_mode(ContainerNetworkMode::All),
            NetworkProfile::All
        );
    }
}
