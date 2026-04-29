use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::ids::*;

use super::session::ExecutionEnvironment;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanType {
    FreeLocal,
    Pro,
    Team,
    Enterprise,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrgMembershipRole {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyFeatureState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProfile {
    LlmOnly,
    Allowlist,
    All,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RouteType {
    CtxManaged,
    CustomerGateway,
    UserOauth,
    UserApiKey,
    UserProviderAccount,
}

impl RouteType {
    pub fn is_personal(self) -> bool {
        matches!(
            self,
            Self::UserOauth | Self::UserApiKey | Self::UserProviderAccount
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveMode {
    LocalOnly,
    AccountPrivate,
    OrgSummary,
    OrgTranscript,
    OrgEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequiredExecutionEnvironment {
    Sandbox,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecisionSource {
    Local,
    CachedPolicy,
    LivePolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicySignatureAlgorithm {
    Hs256,
    Rs256,
    EdDsa,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEnrollmentStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecisionOutcome {
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDenyReason {
    PolicyHardExpired,
    ProviderNotAllowed,
    ModelNotAllowed,
    ExecutionEnvironmentNotAllowed,
    NetworkProfileNotAllowed,
    RouteTypeNotAllowed,
    PersonalRouteNotAllowed,
    DaemonEnrollmentMissing,
    DaemonEnrollmentRevoked,
    PolicySnapshotMissing,
    PolicySignatureInvalid,
    WorkspaceOrgMismatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyWindowState {
    Fresh,
    Grace,
    Expired,
}

impl PolicyWindowState {
    pub fn permits_org_run(self) -> bool {
        !matches!(self, Self::Expired)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutePolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_route_types: Vec<RouteType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchivePolicy {
    pub mode: ArchiveMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonEnrollment {
    pub id: DaemonEnrollmentId,
    pub account_id: AccountId,
    pub org_id: OrgId,
    pub org_membership_id: OrgMembershipId,
    pub membership_role: OrgMembershipRole,
    pub plan_type: PlanType,
    pub status: DaemonEnrollmentStatus,
    pub policy_signature_algorithm: PolicySignatureAlgorithm,
    pub policy_signing_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_policy_snapshot_id: Option<OrgPolicySnapshotId>,
    pub enrolled_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgPolicySnapshot {
    pub id: OrgPolicySnapshotId,
    pub org_id: OrgId,
    pub policy_version: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub grace_expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub allowed_models: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_execution_environment: Option<RequiredExecutionEnvironment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_network_profiles: Vec<NetworkProfile>,
    pub route_policy: RoutePolicy,
    pub archive_policy: ArchivePolicy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, PolicyFeatureState>,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspacePolicyOverlay {
    pub workspace_id: WorkspaceId,
    pub org_id: OrgId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub allowed_models: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_execution_environment: Option<RequiredExecutionEnvironment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_network_profiles: Option<Vec<NetworkProfile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_route_types: Option<Vec<RouteType>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, PolicyFeatureState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveWorkspacePolicy {
    pub org_id: OrgId,
    pub policy_snapshot_id: OrgPolicySnapshotId,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub allowed_models: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_execution_environment: Option<RequiredExecutionEnvironment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_network_profiles: Vec<NetworkProfile>,
    pub route_policy: RoutePolicy,
    pub archive_policy: ArchivePolicy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, PolicyFeatureState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunGrant {
    pub id: RunGrantId,
    pub run_id: RunId,
    pub session_id: SessionId,
    pub workspace_id: WorkspaceId,
    pub account_id: AccountId,
    pub org_id: OrgId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership_role: Option<OrgMembershipRole>,
    pub policy_version: String,
    pub provider_id: String,
    pub model_id: String,
    pub execution_environment: ExecutionEnvironment,
    pub network_profile: NetworkProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_type: Option<RouteType>,
    pub archive_mode: ArchiveMode,
    pub issued_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub decision_source: PolicyDecisionSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDecisionEvent {
    pub id: PolicyDecisionEventId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_grant_id: Option<RunGrantId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<OrgId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_snapshot_id: Option<OrgPolicySnapshotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    pub decision_source: PolicyDecisionSource,
    pub outcome: PolicyDecisionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_reason: Option<PolicyDenyReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_execution_environment: Option<ExecutionEnvironment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_network_profile: Option<NetworkProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_route_type: Option<RouteType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn is_provider_allowed(allowed_providers: Option<&[String]>, provider_id: &str) -> bool {
    match allowed_providers {
        Some(providers) => providers.iter().any(|candidate| candidate == provider_id),
        None => true,
    }
}

pub fn is_provider_model_allowed(
    allowed_providers: Option<&[String]>,
    allowed_models: &BTreeMap<String, Vec<String>>,
    provider_id: &str,
    model_id: &str,
) -> bool {
    if !is_provider_allowed(allowed_providers, provider_id) {
        return false;
    }

    match allowed_models.get(provider_id) {
        Some(models) => models.iter().any(|candidate| candidate == model_id),
        None => true,
    }
}

pub fn execution_environment_satisfies_requirement(
    required_execution_environment: Option<RequiredExecutionEnvironment>,
    execution_environment: ExecutionEnvironment,
) -> bool {
    match required_execution_environment {
        Some(RequiredExecutionEnvironment::Sandbox) => {
            matches!(execution_environment, ExecutionEnvironment::Sandbox)
        }
        None => true,
    }
}

pub fn intersect_network_profiles(
    allowed_network_profiles: &[NetworkProfile],
    overlay_network_profiles: Option<&[NetworkProfile]>,
) -> Vec<NetworkProfile> {
    let allowed = canonicalize_copy_list(allowed_network_profiles);
    let Some(overlay) = overlay_network_profiles else {
        return allowed;
    };

    let overlay_set: BTreeSet<_> = overlay.iter().copied().collect();
    allowed
        .into_iter()
        .filter(|profile| overlay_set.contains(profile))
        .collect()
}

pub fn is_network_profile_allowed(
    allowed_network_profiles: &[NetworkProfile],
    requested_network_profile: NetworkProfile,
) -> bool {
    allowed_network_profiles
        .iter()
        .any(|profile| *profile == requested_network_profile)
}

pub fn is_route_allowed(route_policy: &RoutePolicy, route_type: RouteType) -> bool {
    route_policy
        .allowed_route_types
        .iter()
        .any(|candidate| *candidate == route_type)
}

pub fn is_personal_route_allowed(route_policy: &RoutePolicy, route_type: RouteType) -> bool {
    route_type.is_personal() && is_route_allowed(route_policy, route_type)
}

pub fn is_personal_route_blocked(route_policy: &RoutePolicy, route_type: RouteType) -> bool {
    route_type.is_personal() && !is_route_allowed(route_policy, route_type)
}

pub fn policy_window_state(snapshot: &OrgPolicySnapshot, now: DateTime<Utc>) -> PolicyWindowState {
    if now <= snapshot.expires_at {
        PolicyWindowState::Fresh
    } else if now <= snapshot.grace_expires_at {
        PolicyWindowState::Grace
    } else {
        PolicyWindowState::Expired
    }
}

pub fn merge_org_policy_with_overlay(
    snapshot: &OrgPolicySnapshot,
    overlay: Option<&WorkspacePolicyOverlay>,
) -> EffectiveWorkspacePolicy {
    debug_assert!(
        overlay.is_none_or(|value| value.org_id == snapshot.org_id),
        "workspace policy overlay org_id must match snapshot org_id"
    );
    let overlay_allowed_providers = overlay.and_then(|value| value.allowed_providers.as_deref());
    let merged_allowed_providers = intersect_optional_string_lists(
        snapshot.allowed_providers.as_deref(),
        overlay_allowed_providers,
    );

    let merged_allowed_models = merge_allowed_models(
        &snapshot.allowed_models,
        overlay.map(|value| &value.allowed_models),
        merged_allowed_providers.as_deref(),
    );

    let merged_required_execution_environment = if snapshot.required_execution_environment.is_some()
        || overlay
            .and_then(|value| value.required_execution_environment)
            .is_some()
    {
        Some(RequiredExecutionEnvironment::Sandbox)
    } else {
        None
    };

    let merged_network_profiles = intersect_network_profiles(
        &snapshot.allowed_network_profiles,
        overlay.and_then(|value| value.allowed_network_profiles.as_deref()),
    );

    let merged_route_types = intersect_optional_copy_lists(
        Some(snapshot.route_policy.allowed_route_types.as_slice()),
        overlay.and_then(|value| value.allowed_route_types.as_deref()),
    )
    .unwrap_or_default();

    EffectiveWorkspacePolicy {
        org_id: snapshot.org_id,
        policy_snapshot_id: snapshot.id,
        policy_version: snapshot.policy_version.clone(),
        workspace_id: overlay.map(|value| value.workspace_id),
        allowed_providers: merged_allowed_providers,
        allowed_models: merged_allowed_models,
        required_execution_environment: merged_required_execution_environment,
        allowed_network_profiles: merged_network_profiles,
        route_policy: RoutePolicy {
            allowed_route_types: merged_route_types,
        },
        archive_policy: snapshot.archive_policy.clone(),
        features: merge_feature_states(&snapshot.features, overlay.map(|value| &value.features)),
    }
}

pub fn org_policy_allows_run(
    snapshot: &OrgPolicySnapshot,
    overlay: Option<&WorkspacePolicyOverlay>,
    provider_id: &str,
    model_id: &str,
    execution_environment: ExecutionEnvironment,
    network_profile: NetworkProfile,
    route_type: Option<RouteType>,
    now: DateTime<Utc>,
) -> Result<PolicyWindowState, PolicyDenyReason> {
    let window_state = policy_window_state(snapshot, now);
    if !window_state.permits_org_run() {
        return Err(PolicyDenyReason::PolicyHardExpired);
    }

    let effective_policy = merge_org_policy_with_overlay(snapshot, overlay);
    if !is_provider_allowed(effective_policy.allowed_providers.as_deref(), provider_id) {
        return Err(PolicyDenyReason::ProviderNotAllowed);
    }
    if !is_provider_model_allowed(
        effective_policy.allowed_providers.as_deref(),
        &effective_policy.allowed_models,
        provider_id,
        model_id,
    ) {
        return Err(PolicyDenyReason::ModelNotAllowed);
    }
    if !execution_environment_satisfies_requirement(
        effective_policy.required_execution_environment,
        execution_environment,
    ) {
        return Err(PolicyDenyReason::ExecutionEnvironmentNotAllowed);
    }
    if !is_network_profile_allowed(&effective_policy.allowed_network_profiles, network_profile) {
        return Err(PolicyDenyReason::NetworkProfileNotAllowed);
    }
    if let Some(route_type) = route_type {
        if !is_route_allowed(&effective_policy.route_policy, route_type) {
            if route_type.is_personal() {
                return Err(PolicyDenyReason::PersonalRouteNotAllowed);
            }
            return Err(PolicyDenyReason::RouteTypeNotAllowed);
        }
    }

    Ok(window_state)
}

fn merge_allowed_models(
    allowed_models: &BTreeMap<String, Vec<String>>,
    overlay_allowed_models: Option<&BTreeMap<String, Vec<String>>>,
    merged_allowed_providers: Option<&[String]>,
) -> BTreeMap<String, Vec<String>> {
    let mut provider_ids = BTreeSet::new();
    provider_ids.extend(allowed_models.keys().cloned());
    if let Some(overlay_allowed_models) = overlay_allowed_models {
        provider_ids.extend(overlay_allowed_models.keys().cloned());
    }

    let mut out = BTreeMap::new();
    for provider_id in provider_ids {
        if !is_provider_allowed(merged_allowed_providers, &provider_id) {
            continue;
        }

        let merged = match (
            allowed_models.get(&provider_id),
            overlay_allowed_models.and_then(|value| value.get(&provider_id)),
        ) {
            (Some(org_models), Some(overlay_models)) => {
                let overlay_set: BTreeSet<_> = overlay_models.iter().cloned().collect();
                org_models
                    .iter()
                    .filter(|candidate| overlay_set.contains(*candidate))
                    .cloned()
                    .collect::<Vec<_>>()
            }
            (Some(org_models), None) => canonicalize_string_list(org_models),
            (None, Some(overlay_models)) => canonicalize_string_list(overlay_models),
            (None, None) => continue,
        };

        out.insert(provider_id, canonicalize_string_list(&merged));
    }
    out
}

fn merge_feature_states(
    feature_states: &BTreeMap<String, PolicyFeatureState>,
    overlay_feature_states: Option<&BTreeMap<String, PolicyFeatureState>>,
) -> BTreeMap<String, PolicyFeatureState> {
    let mut out = feature_states.clone();
    if let Some(overlay_feature_states) = overlay_feature_states {
        for (feature, state) in overlay_feature_states {
            if matches!(state, PolicyFeatureState::Disabled) {
                out.insert(feature.clone(), PolicyFeatureState::Disabled);
            }
        }
    }
    out
}

fn intersect_optional_string_lists(
    values: Option<&[String]>,
    overlays: Option<&[String]>,
) -> Option<Vec<String>> {
    match (values, overlays) {
        (Some(values), Some(overlays)) => {
            let overlay_set: BTreeSet<_> = overlays.iter().cloned().collect();
            Some(
                values
                    .iter()
                    .filter(|value| overlay_set.contains(*value))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
        (Some(values), None) => Some(canonicalize_string_list(values)),
        (None, Some(overlays)) => Some(canonicalize_string_list(overlays)),
        (None, None) => None,
    }
}

fn intersect_optional_copy_lists<T>(values: Option<&[T]>, overlays: Option<&[T]>) -> Option<Vec<T>>
where
    T: Copy + Ord,
{
    match (values, overlays) {
        (Some(values), Some(overlays)) => {
            let overlay_set: BTreeSet<_> = overlays.iter().copied().collect();
            Some(
                values
                    .iter()
                    .copied()
                    .filter(|value| overlay_set.contains(value))
                    .collect::<Vec<_>>(),
            )
        }
        (Some(values), None) => Some(canonicalize_copy_list(values)),
        (None, Some(overlays)) => Some(canonicalize_copy_list(overlays)),
        (None, None) => None,
    }
}

fn canonicalize_string_list(values: &[String]) -> Vec<String> {
    let unique: BTreeSet<_> = values.iter().cloned().collect();
    unique.into_iter().collect()
}

fn canonicalize_copy_list<T>(values: &[T]) -> Vec<T>
where
    T: Copy + Ord,
{
    let unique: BTreeSet<_> = values.iter().copied().collect();
    unique.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_snapshot(now: DateTime<Utc>) -> OrgPolicySnapshot {
        let mut allowed_models = BTreeMap::new();
        allowed_models.insert(
            "anthropic".to_string(),
            vec!["claude-sonnet-4".to_string(), "claude-opus-4".to_string()],
        );
        allowed_models.insert("openai".to_string(), vec!["gpt-4.1".to_string()]);

        let mut features = BTreeMap::new();
        features.insert("mobile_relay".to_string(), PolicyFeatureState::Enabled);
        features.insert("llm_token_relay".to_string(), PolicyFeatureState::Enabled);

        OrgPolicySnapshot {
            id: OrgPolicySnapshotId::new(),
            org_id: OrgId::new(),
            policy_version: "2026-04-28.1".to_string(),
            issued_at: now,
            expires_at: now + Duration::minutes(30),
            grace_expires_at: now + Duration::minutes(60),
            allowed_providers: Some(vec!["anthropic".to_string(), "openai".to_string()]),
            allowed_models,
            required_execution_environment: None,
            allowed_network_profiles: vec![NetworkProfile::LlmOnly, NetworkProfile::All],
            route_policy: RoutePolicy {
                allowed_route_types: vec![
                    RouteType::CtxManaged,
                    RouteType::CustomerGateway,
                    RouteType::UserOauth,
                    RouteType::UserApiKey,
                ],
            },
            archive_policy: ArchivePolicy {
                mode: ArchiveMode::OrgTranscript,
            },
            features,
            signature: "signed".to_string(),
        }
    }

    #[test]
    fn workspace_overlay_only_narrows_org_policy() {
        let now = Utc::now();
        let snapshot = sample_snapshot(now);
        let mut overlay_models = BTreeMap::new();
        overlay_models.insert("anthropic".to_string(), vec!["claude-sonnet-4".to_string()]);

        let mut overlay_features = BTreeMap::new();
        overlay_features.insert("llm_token_relay".to_string(), PolicyFeatureState::Disabled);

        let overlay = WorkspacePolicyOverlay {
            workspace_id: WorkspaceId::new(),
            org_id: snapshot.org_id,
            allowed_providers: Some(vec!["anthropic".to_string()]),
            allowed_models: overlay_models,
            required_execution_environment: Some(RequiredExecutionEnvironment::Sandbox),
            allowed_network_profiles: Some(vec![NetworkProfile::LlmOnly]),
            allowed_route_types: Some(vec![RouteType::CtxManaged]),
            features: overlay_features,
        };

        let merged = merge_org_policy_with_overlay(&snapshot, Some(&overlay));
        assert_eq!(merged.workspace_id, Some(overlay.workspace_id));
        assert_eq!(
            merged.allowed_providers,
            Some(vec!["anthropic".to_string()])
        );
        assert_eq!(
            merged.allowed_models.get("anthropic"),
            Some(&vec!["claude-sonnet-4".to_string()])
        );
        assert!(merged.allowed_models.get("openai").is_none());
        assert_eq!(
            merged.required_execution_environment,
            Some(RequiredExecutionEnvironment::Sandbox)
        );
        assert_eq!(
            merged.allowed_network_profiles,
            vec![NetworkProfile::LlmOnly]
        );
        assert_eq!(
            merged.route_policy.allowed_route_types,
            vec![RouteType::CtxManaged]
        );
        assert_eq!(
            merged.features.get("mobile_relay"),
            Some(&PolicyFeatureState::Enabled)
        );
        assert_eq!(
            merged.features.get("llm_token_relay"),
            Some(&PolicyFeatureState::Disabled)
        );
    }

    #[test]
    fn org_policy_window_allows_grace_and_denies_hard_expiry() {
        let now = Utc::now();
        let snapshot = sample_snapshot(now);

        assert_eq!(
            policy_window_state(&snapshot, now),
            PolicyWindowState::Fresh
        );
        assert_eq!(
            policy_window_state(&snapshot, now + Duration::minutes(45)),
            PolicyWindowState::Grace
        );
        assert_eq!(
            policy_window_state(&snapshot, now + Duration::minutes(61)),
            PolicyWindowState::Expired
        );

        let grace_result = org_policy_allows_run(
            &snapshot,
            None,
            "anthropic",
            "claude-sonnet-4",
            ExecutionEnvironment::Sandbox,
            NetworkProfile::LlmOnly,
            Some(RouteType::CtxManaged),
            now + Duration::minutes(45),
        );
        assert_eq!(grace_result, Ok(PolicyWindowState::Grace));

        let expired_result = org_policy_allows_run(
            &snapshot,
            None,
            "anthropic",
            "claude-sonnet-4",
            ExecutionEnvironment::Sandbox,
            NetworkProfile::LlmOnly,
            Some(RouteType::CtxManaged),
            now + Duration::minutes(61),
        );
        assert_eq!(expired_result, Err(PolicyDenyReason::PolicyHardExpired));
    }

    #[test]
    fn required_sandbox_denies_org_managed_host_mode() {
        let now = Utc::now();
        let mut snapshot = sample_snapshot(now);
        snapshot.required_execution_environment = Some(RequiredExecutionEnvironment::Sandbox);

        let result = org_policy_allows_run(
            &snapshot,
            None,
            "anthropic",
            "claude-sonnet-4",
            ExecutionEnvironment::Host,
            NetworkProfile::LlmOnly,
            Some(RouteType::CtxManaged),
            now,
        );

        assert_eq!(
            result,
            Err(PolicyDenyReason::ExecutionEnvironmentNotAllowed)
        );
    }

    #[test]
    fn personal_routes_are_blocked_when_policy_disallows_them() {
        let now = Utc::now();
        let mut snapshot = sample_snapshot(now);
        snapshot.route_policy = RoutePolicy {
            allowed_route_types: vec![RouteType::CtxManaged],
        };

        assert!(is_personal_route_blocked(
            &snapshot.route_policy,
            RouteType::UserApiKey
        ));
        assert!(!is_personal_route_allowed(
            &snapshot.route_policy,
            RouteType::UserApiKey
        ));

        let result = org_policy_allows_run(
            &snapshot,
            None,
            "anthropic",
            "claude-sonnet-4",
            ExecutionEnvironment::Sandbox,
            NetworkProfile::LlmOnly,
            Some(RouteType::UserApiKey),
            now,
        );
        assert_eq!(result, Err(PolicyDenyReason::PersonalRouteNotAllowed));
    }
}
