use super::*;

const MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_MESSAGE_IMAGE_ATTACHMENT_MIB: usize = MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES / (1024 * 1024);

fn image_attachment_too_large_error() -> ApiErr {
    api_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("Image attachments must be {MAX_MESSAGE_IMAGE_ATTACHMENT_MIB} MiB or smaller."),
    )
}

fn ensure_image_attachment_size(bytes: usize) -> Result<(), ApiErr> {
    if bytes > MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES {
        return Err(image_attachment_too_large_error());
    }
    Ok(())
}

fn ensure_image_attachment_mime_type(mime_type: &str) -> Result<(), ApiErr> {
    if mime_type
        .trim()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
    {
        return Ok(());
    }
    Err(api_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "Only image attachments are supported.",
    ))
}

fn decoded_base64_len(data_base64: &str) -> Result<usize, ApiErr> {
    let bytes = data_base64.as_bytes();
    if bytes.is_empty() {
        return Ok(0);
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Invalid image attachment.",
        ));
    }
    let padding = if bytes.ends_with(b"==") {
        2
    } else if bytes.ends_with(b"=") {
        1
    } else {
        0
    };
    Ok((bytes.len() / 4) * 3 - padding)
}

fn decode_inline_image_attachment(data_base64: &str) -> Result<Vec<u8>, ApiErr> {
    let decoded_len = decoded_base64_len(data_base64)?;
    ensure_image_attachment_size(decoded_len)?;
    base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid image attachment."))
}

struct ImageBlobMetadata {
    sha256: String,
    mime_type: String,
}

async fn load_image_blob_metadata(
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

#[derive(Debug, PartialEq, Eq)]
struct AttachmentSignature {
    mime_type: String,
    name: Option<String>,
    sha256: String,
}

async fn attachment_signature(
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

#[cfg(test)]
mod tests {
    use super::{decoded_base64_len, ensure_image_attachment_mime_type};

    #[test]
    fn decoded_base64_len_accounts_for_padding() {
        assert_eq!(decoded_base64_len("YQ==").unwrap(), 1);
        assert_eq!(decoded_base64_len("YWE=").unwrap(), 2);
        assert_eq!(decoded_base64_len("YWFh").unwrap(), 3);
    }

    #[test]
    fn image_attachment_mime_type_requires_image_prefix() {
        assert!(ensure_image_attachment_mime_type("image/png").is_ok());
        assert!(ensure_image_attachment_mime_type("text/plain").is_err());
    }
}
