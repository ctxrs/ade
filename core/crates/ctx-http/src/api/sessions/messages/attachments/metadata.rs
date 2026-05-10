use super::validation::{ensure_image_attachment_mime_type, ensure_image_attachment_size};
use super::*;

pub(super) struct ImageBlobMetadata {
    pub(super) sha256: String,
    pub(super) mime_type: String,
}

pub(super) async fn load_image_blob_metadata(
    state: &Arc<AppState>,
    blob_id: &str,
) -> Result<ImageBlobMetadata, ApiErr> {
    let blob = state.global_store().get_blob(blob_id).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to inspect image attachment.",
        )
    })?;
    let Some((sha256, stored_mime_type, bytes, _stored_name, _created_at)) = blob else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Image attachment blob was not found.",
        ));
    };
    ensure_image_attachment_mime_type(&stored_mime_type)?;
    let bytes = usize::try_from(bytes).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid image attachment metadata.",
        )
    })?;
    ensure_image_attachment_size(bytes)?;
    Ok(ImageBlobMetadata {
        sha256,
        mime_type: stored_mime_type,
    })
}
