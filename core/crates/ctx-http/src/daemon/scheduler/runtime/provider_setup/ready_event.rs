use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::{ExecutionEnvironment, Session};

use crate::daemon::AppState;

use super::super::provider_env::{emit_provider_run_env_ready_event, ProviderRunEnvReadyEvent};

pub(super) struct ProviderSetupReadyEvent<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) session_root_kind: &'a str,
    pub(super) runtime_provider_id: &'a str,
    pub(super) using_endpoint_source: bool,
    pub(super) is_linux_sandbox: bool,
    pub(super) runtime_plan: &'a ctx_harness_runtime::HarnessExecutionPlan,
    pub(super) provider_env: &'a HashMap<String, String>,
}

pub(super) fn emit_provider_setup_ready_event(request: ProviderSetupReadyEvent<'_>) {
    emit_provider_run_env_ready_event(ProviderRunEnvReadyEvent {
        state: request.state,
        session: request.session,
        run_id: request.run_id,
        turn_id: request.turn_id,
        workdir_str: request.workdir_str,
        full_model_id: request.full_model_id,
        execution_environment: request.execution_environment.as_str(),
        session_root_kind: request.session_root_kind,
        runtime_provider_id: request.runtime_provider_id,
        using_endpoint_source: request.using_endpoint_source,
        is_linux_sandbox: request.is_linux_sandbox,
        runtime_plan: request.runtime_plan,
        provider_env: request.provider_env,
    });
}
