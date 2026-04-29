use anyhow::{Context, Result};
use ctx_core::models::{
    DaemonEnrollment, OrgPolicySnapshot, PolicySignatureAlgorithm, RequiredExecutionEnvironment,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
struct UnsignedPolicySnapshot<'a> {
    id: String,
    org_id: String,
    policy_version: &'a str,
    issued_at: String,
    expires_at: String,
    grace_expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_providers: Option<&'a Vec<String>>,
    allowed_models: &'a BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_execution_environment: Option<RequiredExecutionEnvironment>,
    allowed_network_profiles: &'a Vec<ctx_core::models::NetworkProfile>,
    route_policy: &'a ctx_core::models::RoutePolicy,
    archive_policy: &'a ctx_core::models::ArchivePolicy,
    features: &'a BTreeMap<String, ctx_core::models::PolicyFeatureState>,
}

#[derive(Debug, Deserialize)]
struct PolicySnapshotSignatureClaims {
    aud: String,
    exp: usize,
    org_id: String,
    policy_version: String,
    snapshot_sha256: String,
}

const POLICY_SNAPSHOT_AUDIENCE: &str = "ctx.org_policy_snapshot";

fn unsigned_policy_snapshot(snapshot: &OrgPolicySnapshot) -> UnsignedPolicySnapshot<'_> {
    UnsignedPolicySnapshot {
        id: snapshot.id.0.to_string(),
        org_id: snapshot.org_id.0.to_string(),
        policy_version: &snapshot.policy_version,
        issued_at: snapshot.issued_at.to_rfc3339(),
        expires_at: snapshot.expires_at.to_rfc3339(),
        grace_expires_at: snapshot.grace_expires_at.to_rfc3339(),
        allowed_providers: snapshot.allowed_providers.as_ref(),
        allowed_models: &snapshot.allowed_models,
        required_execution_environment: snapshot.required_execution_environment,
        allowed_network_profiles: &snapshot.allowed_network_profiles,
        route_policy: &snapshot.route_policy,
        archive_policy: &snapshot.archive_policy,
        features: &snapshot.features,
    }
}

pub(crate) fn policy_snapshot_digest_hex(snapshot: &OrgPolicySnapshot) -> Result<String> {
    let canonical = serde_json::to_vec(&unsigned_policy_snapshot(snapshot))
        .context("serialize unsigned policy snapshot")?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn jwt_algorithm(algorithm: PolicySignatureAlgorithm) -> Algorithm {
    match algorithm {
        PolicySignatureAlgorithm::Hs256 => Algorithm::HS256,
        PolicySignatureAlgorithm::Rs256 => Algorithm::RS256,
        PolicySignatureAlgorithm::EdDsa => Algorithm::EdDSA,
    }
}

fn decoding_key(enrollment: &DaemonEnrollment) -> Result<DecodingKey> {
    let key = enrollment.policy_signing_key.trim();
    match enrollment.policy_signature_algorithm {
        PolicySignatureAlgorithm::Hs256 => Ok(DecodingKey::from_secret(key.as_bytes())),
        PolicySignatureAlgorithm::Rs256 => {
            DecodingKey::from_rsa_pem(key.as_bytes()).context("parse RSA policy signing public key")
        }
        PolicySignatureAlgorithm::EdDsa => {
            if let Some(component) = key.strip_prefix("ed25519:") {
                return DecodingKey::from_ed_components(component.trim())
                    .context("parse Ed25519 policy signing public key component");
            }
            DecodingKey::from_ed_pem(key.as_bytes())
                .context("parse EdDSA policy signing public key")
        }
    }
}

pub(crate) fn verify_policy_snapshot_signature(
    enrollment: &DaemonEnrollment,
    snapshot: &OrgPolicySnapshot,
) -> Result<()> {
    let algorithm = jwt_algorithm(enrollment.policy_signature_algorithm);
    let key = decoding_key(enrollment)?;
    let mut validation = Validation::new(algorithm);
    validation.validate_aud = false;
    let token = decode::<PolicySnapshotSignatureClaims>(&snapshot.signature, &key, &validation)
        .context("verify policy snapshot signature")?;
    let expected_digest = policy_snapshot_digest_hex(snapshot)?;
    let claims = token.claims;
    if claims.aud != POLICY_SNAPSHOT_AUDIENCE {
        anyhow::bail!("policy snapshot signature audience mismatch");
    }
    if claims.exp == 0 {
        anyhow::bail!("policy snapshot signature expiration missing");
    }
    if claims.org_id != snapshot.org_id.0.to_string() {
        anyhow::bail!("policy snapshot signature org mismatch");
    }
    if claims.policy_version != snapshot.policy_version {
        anyhow::bail!("policy snapshot signature version mismatch");
    }
    if claims.snapshot_sha256 != expected_digest {
        anyhow::bail!("policy snapshot signature digest mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ctx_core::ids::{
        AccountId, DaemonEnrollmentId, OrgId, OrgMembershipId, OrgPolicySnapshotId,
    };
    use ctx_core::models::{
        ArchiveMode, ArchivePolicy, DaemonEnrollmentStatus, NetworkProfile, OrgMembershipRole,
        PlanType, PolicyFeatureState, RoutePolicy, RouteType,
    };
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn enrollment(org_id: OrgId) -> DaemonEnrollment {
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
            policy_signing_key: "test-secret".to_string(),
            active_policy_snapshot_id: None,
            enrolled_at: now,
            updated_at: now,
            revoked_at: None,
        }
    }

    fn snapshot(org_id: OrgId) -> OrgPolicySnapshot {
        let now = Utc::now();
        OrgPolicySnapshot {
            id: OrgPolicySnapshotId::new(),
            org_id,
            policy_version: "2026-04-28.1".to_string(),
            issued_at: now,
            expires_at: now + Duration::minutes(30),
            grace_expires_at: now + Duration::minutes(60),
            allowed_providers: Some(vec!["fake".to_string()]),
            allowed_models: BTreeMap::new(),
            required_execution_environment: None,
            allowed_network_profiles: vec![NetworkProfile::LlmOnly],
            route_policy: RoutePolicy {
                allowed_route_types: vec![RouteType::UserProviderAccount],
            },
            archive_policy: ArchivePolicy {
                mode: ArchiveMode::OrgSummary,
            },
            features: BTreeMap::from([("org_policy".to_string(), PolicyFeatureState::Enabled)]),
            signature: String::new(),
        }
    }

    fn sign(enrollment: &DaemonEnrollment, snapshot: &OrgPolicySnapshot) -> String {
        let claims = serde_json::json!({
            "aud": POLICY_SNAPSHOT_AUDIENCE,
            "exp": (Utc::now() + Duration::minutes(60)).timestamp(),
            "org_id": snapshot.org_id.0.to_string(),
            "policy_version": snapshot.policy_version,
            "snapshot_sha256": policy_snapshot_digest_hex(snapshot).unwrap(),
        });
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(enrollment.policy_signing_key.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn verifies_snapshot_signature_bound_to_digest() {
        let org_id = OrgId::new();
        let enrollment = enrollment(org_id);
        let mut snapshot = snapshot(org_id);
        snapshot.signature = sign(&enrollment, &snapshot);
        verify_policy_snapshot_signature(&enrollment, &snapshot).unwrap();

        snapshot.allowed_providers = Some(vec!["other".to_string()]);
        let err = verify_policy_snapshot_signature(&enrollment, &snapshot).unwrap_err();
        assert!(format!("{err:#}").contains("digest"));
    }
}
