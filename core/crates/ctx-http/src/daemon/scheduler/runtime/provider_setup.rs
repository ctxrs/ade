use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{ExecutionEnvironment, Session};
use ctx_org_policy::admission::{
    admit_runtime_turn, apply_turn_admission_env, RuntimeTurnAdmissionRequest,
};
use ctx_providers::adapters::ProviderAdapter;

use crate::daemon::AppState;

use super::execution_plan::prepare_turn_execution_plan;
use super::helpers::runtime_provider_id_for_session_provider;
use super::provider_env::{
    apply_runtime_source_env, build_base_provider_env, emit_provider_run_env_ready_event,
    prepare_provider_runtime_environment, BaseProviderEnvRequest, ProviderRunEnvReadyEvent,
    ProviderRuntimeEnvironmentRequest,
};
use super::provider_spawn::prepare_provider_adapter_for_turn;
use super::turn_failure::emit_turn_start_failed;
use super::turn_start::apply_crp_launch_policy_env_for_control_mode;

pub(super) struct ProviderTurnRuntimeSetupRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a ctx_store::Store,
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) message_id: MessageId,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) session_root_kind: &'a str,
}

pub(super) struct ProviderTurnRuntimeSetup {
    pub(super) provider_env: HashMap<String, String>,
    pub(super) runtime_provider_id: String,
    pub(super) adapter: Arc<dyn ProviderAdapter>,
}

pub(super) async fn prepare_provider_turn_runtime(
    request: ProviderTurnRuntimeSetupRequest<'_>,
) -> Result<ProviderTurnRuntimeSetup> {
    let ProviderTurnRuntimeSetupRequest {
        state,
        store,
        session,
        run_id,
        turn_id,
        message_id,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
    } = request;

    let settings = ctx_settings_service::load_settings(state.global_store()).await?;
    let provider_control_mode = settings
        .sandboxing
        .as_ref()
        .map(|s| s.provider_control_mode.clone())
        .unwrap_or_default();
    let mut provider_env = build_base_provider_env(BaseProviderEnvRequest {
        daemon_url: &state.core.daemon_url,
        data_root: &state.core.data_root,
        session,
        full_model_id,
        provider_control_mode: &provider_control_mode,
    });

    let execution_plan =
        match prepare_turn_execution_plan(state, store, session, execution_environment).await {
            Ok(execution_plan) => execution_plan,
            Err(err) => {
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                return Err(err);
            }
        };
    let execution_settings = execution_plan.execution_settings;
    let runtime_plan = execution_plan.runtime_plan;
    let is_linux_sandbox = runtime_plan.is_linux_sandbox();
    let source_env = match apply_runtime_source_env(
        &state.core.data_root,
        &session.provider_id,
        &runtime_plan,
        &mut provider_env,
    )
    .await
    {
        Ok(source_env) => source_env,
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let resolved_source = source_env.resolved_source;
    let runtime_source_mode = source_env.runtime_source_mode;
    let using_endpoint_source = source_env.using_endpoint_source;

    let admission = match admit_runtime_turn(
        state.global_store(),
        store,
        RuntimeTurnAdmissionRequest {
            session,
            run_id,
            provider_id: &session.provider_id,
            model_id: full_model_id,
            execution_environment,
            container_network_mode: execution_settings.container.network_mode.clone(),
            source_kind: resolved_source.source_kind,
        },
    )
    .await
    {
        Ok(admission) => admission,
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    apply_turn_admission_env(&mut provider_env, &admission);

    let runtime_provider_id =
        runtime_provider_id_for_session_provider(&session.provider_id, &resolved_source)
            .to_string();
    if runtime_provider_id != session.provider_id {
        provider_env.insert(
            "CTX_PROVIDER_RUNTIME_ID".to_string(),
            runtime_provider_id.clone(),
        );
    }
    let prepared_adapter = match prepare_provider_adapter_for_turn(
        state,
        &runtime_provider_id,
        is_linux_sandbox,
    )
    .await
    {
        Ok(prepared_adapter) => prepared_adapter,
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };

    prepare_provider_runtime_environment(ProviderRuntimeEnvironmentRequest {
        state,
        provider_env: &mut provider_env,
        runtime_provider_id: &runtime_provider_id,
        runtime_plan: &runtime_plan,
        is_linux_sandbox,
        runtime_source_mode,
        adapter_cfg: &prepared_adapter.adapter_cfg,
        install_target: prepared_adapter.install_target,
    })
    .await?;
    apply_crp_launch_policy_env_for_control_mode(&mut provider_env, &provider_control_mode);

    emit_provider_run_env_ready_event(ProviderRunEnvReadyEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str,
        full_model_id,
        execution_environment: execution_environment.as_str(),
        session_root_kind,
        runtime_provider_id: &runtime_provider_id,
        using_endpoint_source,
        is_linux_sandbox,
        runtime_plan: &runtime_plan,
        provider_env: &provider_env,
    });

    Ok(ProviderTurnRuntimeSetup {
        provider_env,
        runtime_provider_id,
        adapter: prepared_adapter.adapter,
    })
}
