use super::metadata::load_image_blob_metadata;
use super::validation::decode_inline_image_attachment;
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AttachmentSignature {
    mime_type: String,
    name: Option<String>,
    sha256: String,
}

pub(super) async fn attachment_signature(
    state: &Arc<AppState>,
    attachment: &MessageAttachment,
) -> Result<AttachmentSignature, ApiErr> {
    match attachment {
        MessageAttachment::Image {
            mime_type,
            data_base64,
            name,
        } => {
            let bytes = decode_inline_image_attachment(data_base64)?;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&bytes);
            let sha256 = hex::encode(hasher.finalize());
            Ok(AttachmentSignature {
                mime_type: mime_type.clone(),
                name: name.clone(),
                sha256,
            })
        }
        MessageAttachment::ImageRef { blob_id, name, .. } => {
            let metadata = load_image_blob_metadata(state, blob_id).await?;
            Ok(AttachmentSignature {
                mime_type: metadata.mime_type,
                name: name.clone(),
                sha256: metadata.sha256,
            })
        }
    }
}
