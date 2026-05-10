use super::*;

mod metadata;
mod signature;
mod validation;

use metadata::load_image_blob_metadata;
use signature::attachment_signature;
use validation::{decode_inline_image_attachment, image_attachment_too_large_error};

pub(super) async fn normalize_message_attachments(
    state: &Arc<AppState>,
    attachments: Vec<MessageAttachment>,
) -> Result<Vec<MessageAttachment>, ApiErr> {
    let mut out = Vec::with_capacity(attachments.len());
    for att in attachments {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = decode_inline_image_attachment(&data_base64)?;
                let saved = persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref())
                    .await
                    .map_err(|status| match status {
                        StatusCode::PAYLOAD_TOO_LARGE => image_attachment_too_large_error(),
                        StatusCode::UNSUPPORTED_MEDIA_TYPE => api_error(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            "Only image attachments are supported.",
                        ),
                        _ => api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to persist image attachment.",
                        ),
                    })?;
                out.push(MessageAttachment::ImageRef {
                    blob_id: saved.blob_id,
                    mime_type,
                    name,
                });
            }
            MessageAttachment::ImageRef { blob_id, name, .. } => {
                let metadata = load_image_blob_metadata(state, &blob_id).await?;
                out.push(MessageAttachment::ImageRef {
                    blob_id,
                    mime_type: metadata.mime_type,
                    name,
                });
            }
        }
    }
    Ok(out)
}

pub(super) async fn attachments_match(
    state: &Arc<AppState>,
    existing: &[MessageAttachment],
    requested: &[MessageAttachment],
) -> Result<bool, ApiErr> {
    if existing.len() != requested.len() {
        return Ok(false);
    }
    let mut existing_sig = Vec::with_capacity(existing.len());
    for attachment in existing {
        existing_sig.push(attachment_signature(state, attachment).await?);
    }
    let mut requested_sig = Vec::with_capacity(requested.len());
    for attachment in requested {
        requested_sig.push(attachment_signature(state, attachment).await?);
    }
    Ok(existing_sig == requested_sig)
}
