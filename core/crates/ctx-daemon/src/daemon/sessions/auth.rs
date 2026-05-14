use std::sync::Arc;

use crate::daemon::DaemonState;
use ctx_core::ids::SessionId;
use ctx_core::models::Session;
use ctx_observability::logs;
use ctx_providers::adapters::{ProviderRunHooks, ProviderSessionRefClaimHook};

mod events;
mod runtime;

use events::{append_auth_notice, spawn_session_auth_event_sink};
use runtime::prepare_session_auth_runtime;

#[derive(Debug)]
pub enum SessionAuthError {
    NotFound(&'static str),
    BadRequest(String),
    Forbidden(String),
    Internal(String),
    AuthenticationFailed { redacted_message: String },
}

fn provider_session_claim_hook(
    store: ctx_store::Store,
    session_id: SessionId,
) -> ProviderSessionRefClaimHook {
    Arc::new(move |claim| {
        let store = store.clone();
        Box::pin(async move {
            if let Some(returned_ref) = claim.returned_provider_session_ref {
                store
                    .claim_session_provider_session_ref(
                        session_id,
                        returned_ref,
                        "provider.session_opened.auth",
                    )
                    .await?;
            }
            Ok(())
        })
    })
}

pub async fn run_session_authentication(
    state: &Arc<DaemonState>,
    store: &ctx_store::Store,
    session: &Session,
    method_id: Option<String>,
) -> Result<(), SessionAuthError> {
    let prepared = prepare_session_auth_runtime(state, store, session).await?;
    let event_sender = spawn_session_auth_event_sink(Arc::clone(state), store.clone(), session.id);

    append_auth_notice(
        state,
        store,
        session.id,
        serde_json::json!({
            "kind": "auth_started",
            "provider": session.provider_id,
            "method_id": method_id,
        }),
    )
    .await?;

    let provider_unknown_event =
        ctx_observability::provider_unknown_events::provider_unknown_event_hook(
            state.telemetry.provider_unknown_events.clone(),
            ctx_observability::provider_unknown_events::ProviderUnknownEventContext {
                provider_id: session.provider_id.clone(),
                execution_environment: Some(session.execution_environment.as_str().to_string()),
                session_root_kind: None,
                operation: "auth".to_string(),
            },
        );
    let result = prepared
        .adapter
        .authenticate_session(
            session.id.0.to_string(),
            prepared.workdir,
            prepared.provider_env,
            method_id,
            event_sender,
            ProviderRunHooks {
                provider_session_ref_claim: Some(provider_session_claim_hook(
                    store.clone(),
                    session.id,
                )),
                provider_unknown_event: Some(provider_unknown_event),
            },
        )
        .await;

    match result {
        Ok(()) => {
            append_auth_notice(
                state,
                store,
                session.id,
                serde_json::json!({
                    "kind": "auth_finished",
                    "provider": session.provider_id,
                }),
            )
            .await?;
            Ok(())
        }
        Err(error) => {
            let redacted_message = logs::redact_sensitive(&error.to_string());
            append_auth_notice(
                state,
                store,
                session.id,
                serde_json::json!({
                    "kind": "auth_failed",
                    "provider": session.provider_id,
                    "message": redacted_message,
                }),
            )
            .await?;
            Err(SessionAuthError::AuthenticationFailed { redacted_message })
        }
    }
}
