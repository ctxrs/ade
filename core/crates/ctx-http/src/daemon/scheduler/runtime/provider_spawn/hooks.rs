use std::sync::Arc;

use ctx_core::models::{ExecutionEnvironment, Session};
use ctx_providers::adapters::{ProviderRunHooks, ProviderSessionRefClaimHook};
use ctx_store::Store;

use crate::daemon::AppState;

pub(super) fn build_provider_run_hooks(
    state: &Arc<AppState>,
    store: &Store,
    session: &Session,
    execution_environment: ExecutionEnvironment,
    session_root_kind: &str,
) -> ProviderRunHooks {
    let claim_store = store.clone();
    let claim_session_id = session.id;
    let provider_session_ref_claim: ProviderSessionRefClaimHook = Arc::new(move |claim| {
        let claim_store = claim_store.clone();
        Box::pin(async move {
            if let Some(returned_ref) = claim.returned_provider_session_ref {
                claim_store
                    .claim_session_provider_session_ref(
                        claim_session_id,
                        returned_ref,
                        "provider.session_opened",
                    )
                    .await?;
            }
            Ok(())
        })
    });
    let provider_unknown_event =
        ctx_observability::provider_unknown_events::provider_unknown_event_hook(
            state.telemetry.provider_unknown_events.clone(),
            ctx_observability::provider_unknown_events::ProviderUnknownEventContext {
                provider_id: session.provider_id.clone(),
                execution_environment: Some(execution_environment.as_str().to_string()),
                session_root_kind: Some(session_root_kind.to_string()),
                operation: "turn".to_string(),
            },
        );
    ProviderRunHooks {
        provider_session_ref_claim: Some(provider_session_ref_claim),
        provider_unknown_event: Some(provider_unknown_event),
    }
}
