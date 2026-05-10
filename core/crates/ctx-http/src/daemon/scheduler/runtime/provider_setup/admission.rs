use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::RunId;
use ctx_core::models::{ExecutionEnvironment, Session};
use ctx_harness_sources::HarnessSourceKind;
use ctx_org_policy::admission::{
    admit_runtime_turn, apply_turn_admission_env, RuntimeTurnAdmissionRequest,
};
use ctx_sandbox_contract::ContainerNetworkMode;

use crate::daemon::AppState;

pub(super) struct ProviderTurnAdmissionEnvRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a ctx_store::Store,
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) provider_id: &'a str,
    pub(super) model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) container_network_mode: ContainerNetworkMode,
    pub(super) source_kind: HarnessSourceKind,
}

pub(super) async fn apply_provider_turn_admission_env(
    provider_env: &mut HashMap<String, String>,
    request: ProviderTurnAdmissionEnvRequest<'_>,
) -> Result<()> {
    let admission = admit_runtime_turn(
        request.state.global_store(),
        request.store,
        RuntimeTurnAdmissionRequest {
            session: request.session,
            run_id: request.run_id,
            provider_id: request.provider_id,
            model_id: request.model_id,
            execution_environment: request.execution_environment,
            container_network_mode: request.container_network_mode,
            source_kind: request.source_kind,
        },
    )
    .await?;
    apply_turn_admission_env(provider_env, &admission);
    Ok(())
}
