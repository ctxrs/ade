use super::*;
use ctx_store::Store;

async fn configured_startup_target(coordinator: &ExecutionSetupCoordinator) -> Result<String> {
    let db_path = coordinator.data_root.join("db").join("db.sqlite");
    let store = Store::open_sqlite(&db_path, Some(1)).await?;
    let settings = crate::settings::load_settings(&store).await?;
    store.close().await;
    Ok(ctx_harness_runtime::runtime_prewarm_target(
        &settings.execution.unwrap_or_default().container,
    ))
}

pub(super) async fn ensure_ready_startup_prewarm_metadata(
    coordinator: &ExecutionSetupCoordinator,
    target: &str,
    gate: &PrewarmGate,
    attempted_at: &str,
    previous_last_success_at: Option<String>,
) -> Option<String> {
    let existing = match read_prewarm_metadata(&coordinator.data_root).await {
        Ok(metadata) => metadata,
        Err(err) => {
            tracing::warn!(
                image = target,
                error = %format_error_chain(&err),
                "failed to read startup prewarm metadata while reporting reused readiness"
            );
            return previous_last_success_at;
        }
    };
    if let Some(metadata) = existing.as_ref() {
        if metadata.image_ref == target
            && !gate.image_ref_changed
            && !gate.bundled_image_digest_changed
        {
            return Some(metadata.ready_at.clone());
        }
    }

    let metadata = StartupPrewarmMetadata {
        image_ref: target.to_string(),
        bundled_image_fingerprint: gate.bundled_image_fingerprint.clone(),
        ready_at: attempted_at.to_string(),
    };
    if let Err(err) = write_prewarm_metadata(&coordinator.data_root, &metadata).await {
        let message = format_error_chain(&err);
        tracing::warn!(
            image = target,
            error = %message,
            "failed to backfill startup prewarm metadata for an already-ready runtime"
        );
        let mut event = OpsEvent::new("warn", "execution.startup_prewarm_metadata_error");
        event.meta = Some(json!({"image": target, "error": message}));
        coordinator.ops_events.emit(event);
        return previous_last_success_at;
    }

    Some(metadata.ready_at)
}

pub(super) async fn refresh_startup_prewarm_metadata_after_successful_container_launch(
    coordinator: &ExecutionSetupCoordinator,
    settings: &crate::settings::ContainerExecutionSettings,
) {
    let target = ctx_harness_runtime::runtime_prewarm_target(settings);
    let startup_target = match configured_startup_target(coordinator).await {
        Ok(startup_target) => startup_target,
        Err(err) => {
            tracing::warn!(
                target,
                error = %format_error_chain(&err),
                "failed to resolve configured startup target after successful launch; leaving startup prewarm metadata unchanged"
            );
            return;
        }
    };
    if startup_target != target {
        return;
    }

    let (metadata_missing, metadata_image_ref_changed) = match read_prewarm_metadata(
        &coordinator.data_root,
    )
    .await
    {
        Ok(metadata) => {
            let image_ref_changed = metadata
                .as_ref()
                .is_some_and(|metadata| metadata.image_ref != target);
            (metadata.is_none(), image_ref_changed)
        }
        Err(err) => {
            tracing::warn!(
                target,
                error = %format_error_chain(&err),
                "failed to read startup prewarm metadata after successful launch; leaving metadata unchanged"
            );
            return;
        }
    };
    let should_refresh = {
        let inner = coordinator.inner.lock().await;
        metadata_missing
            || metadata_image_ref_changed
            || inner.startup.image_ref_changed
            || inner.startup.bundled_image_digest_changed
    };
    if !should_refresh {
        return;
    }

    let bundled_image_fingerprint = match settings.runtime {
        crate::settings::ContainerRuntimeKind::NativeContainer => {
            match bundled_image_fingerprint(&target).await {
                Ok(fingerprint) => fingerprint,
                Err(err) => {
                    tracing::warn!(
                        target,
                        error = %format_error_chain(&err),
                        "failed to compute bundled image fingerprint after successful launch; leaving startup prewarm metadata unchanged"
                    );
                    return;
                }
            }
        }
        crate::settings::ContainerRuntimeKind::SharedVmContainer => None,
    };

    let ready_at = format_ts(Utc::now());
    let metadata = StartupPrewarmMetadata {
        image_ref: target.clone(),
        bundled_image_fingerprint,
        ready_at: ready_at.clone(),
    };
    if let Err(err) = write_prewarm_metadata(&coordinator.data_root, &metadata).await {
        tracing::warn!(
            target,
            error = %format_error_chain(&err),
            "failed to persist refreshed startup prewarm metadata after successful launch"
        );
        return;
    }

    let mut inner = coordinator.inner.lock().await;
    inner.startup.target_image = target;
    if inner.startup.state == StartupPrewarmState::Running {
        return;
    }
    inner.startup.state = StartupPrewarmState::Ready;
    inner.startup.needs_prewarm = false;
    inner.startup.machine_ready = true;
    inner.startup.image_present = true;
    inner.startup.image_ref_changed = false;
    inner.startup.bundled_image_digest_changed = false;
    inner.startup.last_success_at = Some(ready_at);
    inner.startup.error = None;
}
