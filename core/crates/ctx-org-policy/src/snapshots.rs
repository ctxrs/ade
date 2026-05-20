use chrono::Utc;
use ctx_core::models::OrgPolicySnapshot;
use ctx_store::Store;

#[derive(Debug)]
pub enum CacheOrgPolicySnapshotError {
    EnrollmentMissing,
    EnrollmentLoad(anyhow::Error),
    InvalidSignature { message: String },
    SnapshotStore(anyhow::Error),
    EnrollmentActivation(anyhow::Error),
}

pub async fn upsert_org_policy_snapshot(
    store: &Store,
    snapshot: OrgPolicySnapshot,
) -> anyhow::Result<OrgPolicySnapshot> {
    store.upsert_org_policy_snapshot(snapshot).await
}

pub async fn cache_and_activate_org_policy_snapshot(
    store: &Store,
    snapshot: OrgPolicySnapshot,
) -> Result<OrgPolicySnapshot, CacheOrgPolicySnapshotError> {
    let Some(mut enrollment) = store
        .get_daemon_enrollment_by_org_id(snapshot.org_id)
        .await
        .map_err(CacheOrgPolicySnapshotError::EnrollmentLoad)?
    else {
        return Err(CacheOrgPolicySnapshotError::EnrollmentMissing);
    };

    crate::signature::verify_policy_snapshot_signature(&enrollment, &snapshot).map_err(
        |error| CacheOrgPolicySnapshotError::InvalidSignature {
            message: format!("{error:#}"),
        },
    )?;

    let stored = store
        .upsert_org_policy_snapshot(snapshot)
        .await
        .map_err(CacheOrgPolicySnapshotError::SnapshotStore)?;
    enrollment.active_policy_snapshot_id = Some(stored.id);
    enrollment.updated_at = Utc::now();
    store
        .upsert_daemon_enrollment(enrollment)
        .await
        .map_err(CacheOrgPolicySnapshotError::EnrollmentActivation)?;
    Ok(stored)
}
