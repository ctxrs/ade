use super::*;

pub(super) async fn startup_runtime_state(
    coordinator: &ExecutionSetupCoordinator,
    settings: &crate::settings::ContainerExecutionSettings,
) -> Result<(bool, bool)> {
    match settings.runtime {
        crate::settings::ContainerRuntimeKind::NativeContainer => {
            let target = ctx_harness_runtime::resolve_container_image(settings);
            let machine_ready = normalize_container_engine_ready_for_gate(
                ctx_harness_runtime::sandbox_engine_ready(&coordinator.data_root).await,
            )?;
            let image_present = if machine_ready {
                ctx_harness_runtime::container_image_present(&coordinator.data_root, &target)
                    .await?
            } else {
                false
            };
            Ok((machine_ready, image_present))
        }
        crate::settings::ContainerRuntimeKind::SharedVmContainer => {
            ctx_harness_runtime::selected_runtime_launch_readiness_state(
                &coordinator.data_root,
                settings,
            )
            .await
        }
    }
}

pub(super) async fn startup_prewarm_runtime(
    coordinator: &ExecutionSetupCoordinator,
    exec: &ExecutionSettings,
) -> Result<()> {
    let _artifact_warmup = coordinator.harness.begin_prewarm_artifact_activity();
    let scope = RuntimePrewarmScope::Runtime;
    coordinator.prewarm.ensure_scope(exec, scope, None).await
}
