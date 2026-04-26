use super::default_session_plan::select_default_provider_id;
use ctx_providers::adapters::{
    ProviderHealth, ProviderRecommendedAction, ProviderStatus, ProviderUsability,
    ProviderUsabilityStatus,
};
use std::collections::HashMap;

fn status(provider_id: &str) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: true,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ProviderHealth::Ok,
        diagnostics: Vec::new(),
        details: HashMap::new(),
        usability: ProviderUsability {
            usable: true,
            status: ProviderUsabilityStatus::Ready,
            reason_code: None,
            reason: None,
            blocking_provider_ids: Vec::new(),
            recommended_action: ProviderRecommendedAction::None,
        },
    }
}

#[test]
fn selects_canonical_codex_crp_for_default_session_creation() {
    let statuses = vec![status("codex")];

    assert_eq!(
        select_default_provider_id(&statuses),
        Some("codex-crp".to_string())
    );
}

#[test]
fn falls_back_to_stable_canonical_order_for_nonpreferred_providers() {
    let statuses = vec![status("zeta"), status("alpha")];

    assert_eq!(
        select_default_provider_id(&statuses),
        Some("alpha".to_string())
    );
}
