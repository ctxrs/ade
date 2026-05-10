use super::*;

const MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_MESSAGE_IMAGE_ATTACHMENT_MIB: usize = MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES / (1024 * 1024);

pub(super) fn image_attachment_too_large_error() -> ApiErr {
    api_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("Image attachments must be {MAX_MESSAGE_IMAGE_ATTACHMENT_MIB} MiB or smaller."),
    )
}

pub(super) fn ensure_image_attachment_size(bytes: usize) -> Result<(), ApiErr> {
    if bytes > MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES {
        return Err(image_attachment_too_large_error());
    }
    Ok(())
}

pub(super) fn ensure_image_attachment_mime_type(mime_type: &str) -> Result<(), ApiErr> {
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

pub(super) fn decoded_base64_len(data_base64: &str) -> Result<usize, ApiErr> {
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

pub(super) fn decode_inline_image_attachment(data_base64: &str) -> Result<Vec<u8>, ApiErr> {
    let decoded_len = decoded_base64_len(data_base64)?;
    ensure_image_attachment_size(decoded_len)?;
    base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid image attachment."))
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
