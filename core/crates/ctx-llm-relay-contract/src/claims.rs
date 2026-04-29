use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    subject::{AccessContext, AccessContextError, AccessContextKind},
    RouteAuthMethod, RouteType, CONTROL_PLANE_ISSUER, RELAY_AUDIENCE,
    V1_MAX_DELEGATION_TTL_SECONDS,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProviderModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayDelegationClaims {
    pub jti: String,
    pub access_context_kind: AccessContextKind,
    pub billing_subject_id: String,
    pub ctx_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_membership_id: Option<String>,
    pub daemon_id: String,
    pub daemon_public_key_thumbprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_public_key_jwk: Option<serde_json::Value>,
    pub allowed_route_ids: Vec<String>,
    pub allowed_provider_model_pairs: Vec<ProviderModelRef>,
    pub allowed_auth_methods: Vec<RouteAuthMethod>,
    pub policy_version: String,
    pub pricing_version: String,
    pub max_per_request_cents: u64,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_requests: Option<u32>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub issuer: String,
    pub audience: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunGrantClaims {
    pub jti: String,
    pub request_id: String,
    pub access_context_kind: AccessContextKind,
    pub billing_subject_id: String,
    pub ctx_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_membership_id: Option<String>,
    pub daemon_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub route_id: String,
    #[serde(rename = "route_type")]
    pub route_type: RouteType,
    pub provider_id: String,
    pub model_id: String,
    pub policy_version: String,
    pub pricing_version: String,
    pub max_estimated_cents: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub delegation_jti: String,
    pub delegation_hash: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub issuer: String,
    pub audience: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GrantValidationError {
    #[error("relay delegation access context is invalid: {0}")]
    InvalidDelegationAccessContext(AccessContextError),
    #[error("run grant access context is invalid: {0}")]
    InvalidRunGrantAccessContext(AccessContextError),
    #[error("relay delegation issuer must be {expected}, found {actual}")]
    UnexpectedDelegationIssuer {
        expected: &'static str,
        actual: String,
    },
    #[error("relay delegation audience must be {expected}, found {actual}")]
    UnexpectedDelegationAudience {
        expected: &'static str,
        actual: String,
    },
    #[error("run grant audience must be {expected}, found {actual}")]
    UnexpectedGrantAudience {
        expected: &'static str,
        actual: String,
    },
    #[error("run grant issuer is required")]
    MissingGrantIssuer,
    #[error("run grant delegation hash is required")]
    MissingDelegationHash,
    #[error("relay delegation expires before or at issuance")]
    InvalidDelegationValidityWindow,
    #[error("run grant expires before or at issuance")]
    InvalidGrantValidityWindow,
    #[error("relay delegation ttl exceeds the v1 maximum")]
    DelegationTtlExceeded,
    #[error("relay delegation has expired")]
    DelegationExpired,
    #[error("run grant has expired")]
    GrantExpired,
    #[error("run grant was issued before the relay delegation")]
    GrantIssuedBeforeDelegation,
    #[error("run grant expires after relay delegation expiry")]
    GrantExpiresAfterDelegation,
    #[error("run grant delegation_jti does not match relay delegation jti")]
    DelegationJtiMismatch,
    #[error("run grant field {field} does not match relay delegation")]
    ClaimMismatch { field: &'static str },
    #[error("run grant route type must be ctx_managed")]
    InvalidGrantRouteType,
    #[error("relay delegation must allow only ctx_provider_key auth")]
    InvalidAllowedAuthMethods,
    #[error("route {route_id} is not allowed by relay delegation")]
    RouteNotAllowed { route_id: String },
    #[error("provider/model pair {provider_id}/{model_id} is not allowed by relay delegation")]
    ProviderModelNotAllowed {
        provider_id: String,
        model_id: String,
    },
    #[error("run grant max_estimated_cents exceeds relay delegation limit")]
    MaxEstimatedCentsExceeded,
    #[error("run grant max_input_tokens exceeds relay delegation limit")]
    MaxInputTokensExceeded,
    #[error("run grant max_output_tokens exceeds relay delegation limit")]
    MaxOutputTokensExceeded,
}

impl RelayDelegationClaims {
    pub fn access_context(&self) -> AccessContext {
        AccessContext {
            kind: self.access_context_kind.clone(),
            billing_subject_id: self.billing_subject_id.clone(),
            ctx_user_id: self.ctx_user_id.clone(),
            ctx_account_id: self.ctx_account_id.clone(),
            ctx_org_id: self.ctx_org_id.clone(),
            ctx_membership_id: self.ctx_membership_id.clone(),
        }
    }

    pub fn allows_route(&self, route_id: &str) -> bool {
        self.allowed_route_ids
            .iter()
            .any(|candidate| candidate == route_id)
    }

    pub fn allows_provider_model(&self, provider_id: &str, model_id: &str) -> bool {
        self.allowed_provider_model_pairs
            .iter()
            .any(|candidate| candidate.provider_id == provider_id && candidate.model_id == model_id)
    }

    pub fn validate_standard_claims(&self, now: DateTime<Utc>) -> Result<(), GrantValidationError> {
        self.access_context()
            .validate()
            .map_err(GrantValidationError::InvalidDelegationAccessContext)?;

        if self.issuer != CONTROL_PLANE_ISSUER {
            return Err(GrantValidationError::UnexpectedDelegationIssuer {
                expected: CONTROL_PLANE_ISSUER,
                actual: self.issuer.clone(),
            });
        }
        if self.audience != RELAY_AUDIENCE {
            return Err(GrantValidationError::UnexpectedDelegationAudience {
                expected: RELAY_AUDIENCE,
                actual: self.audience.clone(),
            });
        }
        if self.expires_at <= self.issued_at {
            return Err(GrantValidationError::InvalidDelegationValidityWindow);
        }
        if self
            .expires_at
            .signed_duration_since(self.issued_at)
            .num_seconds()
            > V1_MAX_DELEGATION_TTL_SECONDS
        {
            return Err(GrantValidationError::DelegationTtlExceeded);
        }
        if now >= self.expires_at {
            return Err(GrantValidationError::DelegationExpired);
        }

        Ok(())
    }
}

impl RunGrantClaims {
    pub fn access_context(&self) -> AccessContext {
        AccessContext {
            kind: self.access_context_kind.clone(),
            billing_subject_id: self.billing_subject_id.clone(),
            ctx_user_id: self.ctx_user_id.clone(),
            ctx_account_id: self.ctx_account_id.clone(),
            ctx_org_id: self.ctx_org_id.clone(),
            ctx_membership_id: self.ctx_membership_id.clone(),
        }
    }

    pub fn validate_against_delegation(
        &self,
        delegation: &RelayDelegationClaims,
        now: DateTime<Utc>,
    ) -> Result<(), GrantValidationError> {
        delegation.validate_standard_claims(now)?;
        self.access_context()
            .validate()
            .map_err(GrantValidationError::InvalidRunGrantAccessContext)?;

        if self.audience != RELAY_AUDIENCE {
            return Err(GrantValidationError::UnexpectedGrantAudience {
                expected: RELAY_AUDIENCE,
                actual: self.audience.clone(),
            });
        }
        if self.issuer.trim().is_empty() {
            return Err(GrantValidationError::MissingGrantIssuer);
        }
        if self.delegation_hash.trim().is_empty() {
            return Err(GrantValidationError::MissingDelegationHash);
        }
        if self.expires_at <= self.issued_at {
            return Err(GrantValidationError::InvalidGrantValidityWindow);
        }
        if now >= self.expires_at {
            return Err(GrantValidationError::GrantExpired);
        }
        if self.issued_at < delegation.issued_at {
            return Err(GrantValidationError::GrantIssuedBeforeDelegation);
        }
        if self.expires_at > delegation.expires_at {
            return Err(GrantValidationError::GrantExpiresAfterDelegation);
        }
        if self.delegation_jti != delegation.jti {
            return Err(GrantValidationError::DelegationJtiMismatch);
        }
        if self.route_type != RouteType::CtxManaged {
            return Err(GrantValidationError::InvalidGrantRouteType);
        }
        if delegation.allowed_auth_methods.is_empty()
            || delegation
                .allowed_auth_methods
                .iter()
                .any(|method| *method != RouteAuthMethod::CtxProviderKey)
        {
            return Err(GrantValidationError::InvalidAllowedAuthMethods);
        }
        if !delegation.allows_route(&self.route_id) {
            return Err(GrantValidationError::RouteNotAllowed {
                route_id: self.route_id.clone(),
            });
        }
        if !delegation.allows_provider_model(&self.provider_id, &self.model_id) {
            return Err(GrantValidationError::ProviderModelNotAllowed {
                provider_id: self.provider_id.clone(),
                model_id: self.model_id.clone(),
            });
        }

        assert_same_string(
            "access_context_kind",
            &format!("{:?}", self.access_context_kind),
            &format!("{:?}", delegation.access_context_kind),
        )?;
        assert_same_string(
            "billing_subject_id",
            &self.billing_subject_id,
            &delegation.billing_subject_id,
        )?;
        assert_same_string("ctx_user_id", &self.ctx_user_id, &delegation.ctx_user_id)?;
        assert_same_optional_string(
            "ctx_account_id",
            &self.ctx_account_id,
            &delegation.ctx_account_id,
        )?;
        assert_same_optional_string("ctx_org_id", &self.ctx_org_id, &delegation.ctx_org_id)?;
        assert_same_optional_string(
            "ctx_membership_id",
            &self.ctx_membership_id,
            &delegation.ctx_membership_id,
        )?;
        assert_same_string("daemon_id", &self.daemon_id, &delegation.daemon_id)?;
        assert_same_string(
            "policy_version",
            &self.policy_version,
            &delegation.policy_version,
        )?;
        assert_same_string(
            "pricing_version",
            &self.pricing_version,
            &delegation.pricing_version,
        )?;

        if self.max_estimated_cents > delegation.max_per_request_cents {
            return Err(GrantValidationError::MaxEstimatedCentsExceeded);
        }
        if self.max_input_tokens.unwrap_or(0) > delegation.max_input_tokens {
            return Err(GrantValidationError::MaxInputTokensExceeded);
        }
        if self.max_output_tokens.unwrap_or(0) > delegation.max_output_tokens {
            return Err(GrantValidationError::MaxOutputTokensExceeded);
        }

        Ok(())
    }
}

fn assert_same_string(
    field: &'static str,
    left: &str,
    right: &str,
) -> Result<(), GrantValidationError> {
    if left == right {
        Ok(())
    } else {
        Err(GrantValidationError::ClaimMismatch { field })
    }
}

fn assert_same_optional_string(
    field: &'static str,
    left: &Option<String>,
    right: &Option<String>,
) -> Result<(), GrantValidationError> {
    if left == right {
        Ok(())
    } else {
        Err(GrantValidationError::ClaimMismatch { field })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{GrantValidationError, ProviderModelRef, RelayDelegationClaims, RunGrantClaims};
    use crate::{AccessContextKind, RouteAuthMethod, RouteType};

    fn base_delegation() -> RelayDelegationClaims {
        let issued_at = Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
        RelayDelegationClaims {
            jti: "delegation_1".to_string(),
            access_context_kind: AccessContextKind::Org,
            billing_subject_id: "bill_org_1".to_string(),
            ctx_user_id: "user_1".to_string(),
            ctx_account_id: Some("account_1".to_string()),
            ctx_org_id: Some("org_1".to_string()),
            ctx_membership_id: Some("membership_1".to_string()),
            daemon_id: "daemon_1".to_string(),
            daemon_public_key_thumbprint: "thumb_1".to_string(),
            daemon_public_key_jwk: None,
            allowed_route_ids: vec!["route_ctx".to_string()],
            allowed_provider_model_pairs: vec![ProviderModelRef {
                provider_id: "openai".to_string(),
                model_id: "gpt-5".to_string(),
            }],
            allowed_auth_methods: vec![RouteAuthMethod::CtxProviderKey],
            policy_version: "policy_v1".to_string(),
            pricing_version: "pricing_v1".to_string(),
            max_per_request_cents: 250,
            max_input_tokens: 32_000,
            max_output_tokens: 4_096,
            max_concurrent_requests: Some(4),
            issued_at,
            expires_at: issued_at + Duration::minutes(5),
            issuer: "ctx-control-plane".to_string(),
            audience: "ctx-llm-relay".to_string(),
        }
    }

    fn base_grant() -> RunGrantClaims {
        let issued_at = Utc.with_ymd_and_hms(2026, 4, 28, 12, 1, 0).unwrap();
        RunGrantClaims {
            jti: "grant_1".to_string(),
            request_id: "request_1".to_string(),
            access_context_kind: AccessContextKind::Org,
            billing_subject_id: "bill_org_1".to_string(),
            ctx_user_id: "user_1".to_string(),
            ctx_account_id: Some("account_1".to_string()),
            ctx_org_id: Some("org_1".to_string()),
            ctx_membership_id: Some("membership_1".to_string()),
            daemon_id: "daemon_1".to_string(),
            workspace_id: Some("workspace_1".to_string()),
            task_id: Some("task_1".to_string()),
            session_id: Some("session_1".to_string()),
            run_id: Some("run_1".to_string()),
            turn_id: Some("turn_1".to_string()),
            route_id: "route_ctx".to_string(),
            route_type: RouteType::CtxManaged,
            provider_id: "openai".to_string(),
            model_id: "gpt-5".to_string(),
            policy_version: "policy_v1".to_string(),
            pricing_version: "pricing_v1".to_string(),
            max_estimated_cents: 125,
            max_input_tokens: Some(24_000),
            max_output_tokens: Some(2_048),
            delegation_jti: "delegation_1".to_string(),
            delegation_hash: "hash_1".to_string(),
            issued_at,
            expires_at: issued_at + Duration::minutes(2),
            issuer: "ctx-daemon:daemon_1".to_string(),
            audience: "ctx-llm-relay".to_string(),
        }
    }

    #[test]
    fn run_grant_subset_validation_accepts_valid_subset() {
        let delegation = base_delegation();
        let grant = base_grant();
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 2, 0).unwrap();

        assert_eq!(grant.validate_against_delegation(&delegation, now), Ok(()));
    }

    #[test]
    fn run_grant_subset_validation_rejects_disallowed_provider_model() {
        let delegation = base_delegation();
        let mut grant = base_grant();
        grant.model_id = "gpt-5-mini".to_string();
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 2, 0).unwrap();

        assert!(matches!(
            grant.validate_against_delegation(&delegation, now),
            Err(GrantValidationError::ProviderModelNotAllowed { .. })
        ));
    }

    #[test]
    fn run_grant_subset_validation_rejects_excess_output_limit() {
        let delegation = base_delegation();
        let mut grant = base_grant();
        grant.max_output_tokens = Some(10_000);
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 2, 0).unwrap();

        assert_eq!(
            grant.validate_against_delegation(&delegation, now),
            Err(GrantValidationError::MaxOutputTokensExceeded)
        );
    }
}
