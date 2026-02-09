use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::daemon::AppState;
use crate::harness_runtime;
use crate::settings as user_settings;

#[derive(Debug, Serialize)]
pub(super) struct PrefetchContainerImageResp {
    started: bool,
    image: String,
    present: bool,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) async fn prefetch_container_image(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PrefetchContainerImageResp>, StatusCode> {
    let settings = user_settings::load_settings(&state.core.data_root).await;
    let exec = settings.execution.unwrap_or_default();
    let image = harness_runtime::resolve_container_image(&exec.container);

    let st = harness_runtime::container_image_status(&state.core.data_root, &image)
        .await
        .unwrap_or(harness_runtime::ContainerImageStatus {
            present: false,
            available: false,
            error: Some("failed to determine container image status".to_string()),
        });
    if !st.available {
        return Ok(Json(PrefetchContainerImageResp {
            started: false,
            image,
            present: st.present,
            available: st.available,
            error: st.error,
        }));
    }
    if st.present {
        return Ok(Json(PrefetchContainerImageResp {
            started: false,
            image,
            present: true,
            available: true,
            error: None,
        }));
    }

    if !harness_runtime::is_default_container_image(&image) {
        return Ok(Json(PrefetchContainerImageResp {
            started: false,
            image,
            present: false,
            available: true,
            error: Some(
                "container image prefetch is only supported for the default ctx-harness image; custom images must already exist in podman".to_string(),
            ),
        }));
    }

    if harness_runtime::bundled_default_container_image_tar().is_none() {
        return Ok(Json(PrefetchContainerImageResp {
            started: false,
            image,
            present: false,
            available: true,
            error: Some(
                "default ctx-harness image is missing and no bundled image tar was found (CTX_BUNDLE_DIR not set?)".to_string(),
            ),
        }));
    }

    // Fire-and-forget: prefetch is opportunistic and should not block UI flows.
    let image_clone = image.clone();
    let data_root = state.core.data_root.clone();
    tokio::spawn(async move {
        if let Err(err) = harness_runtime::prefetch_container_image(&data_root, &image_clone).await
        {
            tracing::warn!(
                "container image prefetch failed for '{}': {err:#}",
                image_clone
            );
        }
    });

    Ok(Json(PrefetchContainerImageResp {
        started: true,
        image,
        present: false,
        available: true,
        error: None,
    }))
}

#[derive(Debug, Serialize)]
pub(super) struct ContainerImageStatusResp {
    image: String,
    present: bool,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) async fn container_image_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ContainerImageStatusResp>, StatusCode> {
    let settings = user_settings::load_settings(&state.core.data_root).await;
    let exec = settings.execution.unwrap_or_default();
    let image = harness_runtime::resolve_container_image(&exec.container);

    let st = harness_runtime::container_image_status(&state.core.data_root, &image)
        .await
        .unwrap_or(harness_runtime::ContainerImageStatus {
            present: false,
            available: false,
            error: Some("failed to determine container image status".to_string()),
        });
    Ok(Json(ContainerImageStatusResp {
        image,
        present: st.present,
        available: st.available,
        error: st.error,
    }))
}
