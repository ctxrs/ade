use super::*;

mod metadata;
mod validation;

use metadata::load_image_blob_metadata;
use validation::{decode_inline_image_attachment, image_attachment_too_large_error};

pub(super) async fn normalize_message_attachments(
    state: &SessionsHandle,
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
                let blob_id = state
                    .store_inline_image_blob(&bytes, &mime_type, name.as_deref())
                    .await
                    .map_err(|error| match error {
                        crate::daemon::sessions::SessionImageBlobStoreError::PayloadTooLarge => {
                            image_attachment_too_large_error()
                        }
                        crate::daemon::sessions::SessionImageBlobStoreError::UnsupportedMediaType => api_error(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            "Only image attachments are supported.",
                        ),
                        crate::daemon::sessions::SessionImageBlobStoreError::Internal => api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to persist image attachment.",
                        ),
                    })?;
                out.push(MessageAttachment::ImageRef {
                    blob_id,
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
