use chrono::Utc;
use ctx_core::ids::OrgId;
use ctx_core::models::{DaemonEnrollment, PlanType};
use ctx_store::Store;

#[derive(Debug)]
pub enum UpsertDaemonEnrollmentError {
    UnsupportedPlan,
    MissingSigningKey,
    Store(anyhow::Error),
}

pub async fn list_daemon_enrollments(store: &Store) -> anyhow::Result<Vec<DaemonEnrollment>> {
    store.list_daemon_enrollments().await
}

pub async fn upsert_daemon_enrollment_unchecked(
    store: &Store,
    enrollment: DaemonEnrollment,
) -> anyhow::Result<DaemonEnrollment> {
    store.upsert_daemon_enrollment(enrollment).await
}

pub async fn upsert_daemon_enrollment_checked(
    store: &Store,
    mut enrollment: DaemonEnrollment,
) -> Result<DaemonEnrollment, UpsertDaemonEnrollmentError> {
    if !matches!(enrollment.plan_type, PlanType::Team | PlanType::Enterprise) {
        return Err(UpsertDaemonEnrollmentError::UnsupportedPlan);
    }
    if enrollment.policy_signing_key.trim().is_empty() {
        return Err(UpsertDaemonEnrollmentError::MissingSigningKey);
    }
    enrollment.updated_at = Utc::now();
    upsert_daemon_enrollment_unchecked(store, enrollment)
        .await
        .map_err(UpsertDaemonEnrollmentError::Store)
}

pub async fn get_daemon_enrollment_by_org_id(
    store: &Store,
    org_id: OrgId,
) -> anyhow::Result<Option<DaemonEnrollment>> {
    store.get_daemon_enrollment_by_org_id(org_id).await
}
