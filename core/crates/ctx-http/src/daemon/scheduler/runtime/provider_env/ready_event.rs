use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::Session;
use ctx_observability::ops_events::OpsEvent;

use crate::daemon::AppState;

pub(in crate::daemon::scheduler::runtime) struct ProviderRunEnvReadyEvent<'a> {
    pub(in crate::daemon::scheduler::runtime) state: &'a Arc<AppState>,
    pub(in crate::daemon::scheduler::runtime) session: &'a Session,
    pub(in crate::daemon::scheduler::runtime) run_id: RunId,
    pub(in crate::daemon::scheduler::runtime) turn_id: TurnId,
    pub(in crate::daemon::scheduler::runtime) workdir_str: &'a str,
    pub(in crate::daemon::scheduler::runtime) full_model_id: &'a str,
    pub(in crate::daemon::scheduler::runtime) execution_environment: &'a str,
    pub(in crate::daemon::scheduler::runtime) session_root_kind: &'a str,
    pub(in crate::daemon::scheduler::runtime) runtime_provider_id: &'a str,
    pub(in crate::daemon::scheduler::runtime) using_endpoint_source: bool,
    pub(in crate::daemon::scheduler::runtime) is_linux_sandbox: bool,
    pub(in crate::daemon::scheduler::runtime) runtime_plan:
        &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(in crate::daemon::scheduler::runtime) provider_env: &'a HashMap<String, String>,
}

pub(in crate::daemon::scheduler::runtime) fn emit_provider_run_env_ready_event(
    event: ProviderRunEnvReadyEvent<'_>,
) {
    let ProviderRunEnvReadyEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
        runtime_provider_id,
        using_endpoint_source,
        is_linux_sandbox,
        runtime_plan,
        provider_env,
    } = event;
    let mut run_env_event = OpsEvent::new("info", "provider_run_env_ready");
    run_env_event.session_id = Some(session.id.0.to_string());
    run_env_event.worktree_id = Some(session.worktree_id.0.to_string());
    run_env_event.run_id = Some(run_id.0.to_string());
    run_env_event.turn_id = Some(turn_id.0.to_string());
    run_env_event.provider_id = Some(session.provider_id.clone());
    run_env_event.cwd = Some(workdir_str.to_string());
    run_env_event.worktree_root = Some(workdir_str.to_string());
    run_env_event.meta = Some(json!({
        "model_id": full_model_id,
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": execution_environment,
        "session_root_kind": session_root_kind,
        "runtime_provider_id": runtime_provider_id,
        "source_kind": if using_endpoint_source { "endpoint" } else { "subscription" },
        "is_container": is_linux_sandbox,
        "runtime_kind": runtime_plan
            .env_overrides
            .get(ctx_harness_runtime::CTX_HARNESS_RUNTIME_KIND_ENV)
            .cloned()
            .unwrap_or_else(|| "host".to_string()),
        "has_openai_api_key": provider_env
            .get("OPENAI_API_KEY")
            .is_some_and(|value| !value.trim().is_empty()),
        "has_codex_home": provider_env
            .get("CODEX_HOME")
            .is_some_and(|value| !value.trim().is_empty()),
        "openai_base_url_host": provider_env
            .get("OPENAI_BASE_URL")
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|parsed| parsed.host_str().map(|host| host.to_string())),
    }));
    state.telemetry.ops_events.emit(run_env_event);
}
