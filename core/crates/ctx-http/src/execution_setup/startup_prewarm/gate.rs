use super::*;

pub(super) async fn compute_prewarm_gate(
    coordinator: &ExecutionSetupCoordinator,
    settings: &crate::settings::ContainerExecutionSettings,
) -> Result<PrewarmGate> {
    let (machine_ready, image_present) = coordinator.startup_runtime_state(settings).await?;
    compute_prewarm_gate_with_runtime_state(coordinator, settings, machine_ready, image_present)
        .await
}

pub(super) async fn compute_prewarm_gate_with_runtime_state(
    coordinator: &ExecutionSetupCoordinator,
    settings: &crate::settings::ContainerExecutionSettings,
    machine_ready: bool,
    image_present: bool,
) -> Result<PrewarmGate> {
    let target = ctx_harness_runtime::runtime_prewarm_target(settings);
    let metadata = read_prewarm_metadata(&coordinator.data_root).await?;
    let bundled_image_fingerprint = match settings.runtime {
        crate::settings::ContainerRuntimeKind::NativeContainer => {
            bundled_image_fingerprint(&target).await?
        }
        crate::settings::ContainerRuntimeKind::SharedVmContainer => None,
    };

    let image_ref_changed = metadata
        .as_ref()
        .map(|meta| meta.image_ref != target)
        .unwrap_or(false);
    let bundled_image_digest_changed = match metadata.as_ref() {
        Some(meta) => meta.bundled_image_fingerprint != bundled_image_fingerprint,
        None => image_present && bundled_image_fingerprint.is_some(),
    };

    let needs_prewarm = needs_prewarm(
        machine_ready,
        image_present,
        image_ref_changed,
        bundled_image_digest_changed,
    );

    Ok(PrewarmGate {
        machine_ready,
        image_present,
        image_ref_changed,
        bundled_image_digest_changed,
        needs_prewarm,
        bundled_image_fingerprint,
    })
}

pub(super) async fn force_reload_stale_default_container_image_if_needed(
    coordinator: &ExecutionSetupCoordinator,
    settings: &crate::settings::ContainerExecutionSettings,
    gate: &PrewarmGate,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<bool> {
    if !gate.machine_ready || !gate.image_present || !gate.bundled_image_digest_changed {
        return Ok(false);
    }
    if !matches!(
        settings.runtime,
        crate::settings::ContainerRuntimeKind::NativeContainer
    ) {
        return Ok(false);
    }
    let target = ctx_harness_runtime::runtime_prewarm_target(settings);
    if !ctx_sandbox_container_runtime::is_default_container_image(&target) {
        return Ok(false);
    }
    ctx_sandbox_container_runtime::force_reload_default_container_image(
        &coordinator.data_root,
        &ctx_sandbox_container_runtime::SandboxCommandMode::NativeContainer,
        observer,
    )
    .await?;
    Ok(true)
}
