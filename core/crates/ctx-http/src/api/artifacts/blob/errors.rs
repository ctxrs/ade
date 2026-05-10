use super::*;

pub(super) fn blob_upload_api_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

pub(super) fn blob_upload_status_error(status: StatusCode) -> (StatusCode, Json<ApiErrorResp>) {
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => {
            blob_upload_api_error(status, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE)
        }
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            blob_upload_api_error(status, "Only image attachments are supported.")
        }
        StatusCode::INTERNAL_SERVER_ERROR => {
            blob_upload_api_error(status, "Failed to store image attachment.")
        }
        _ => blob_upload_api_error(status, "Image attachment upload failed."),
    }
}

pub(super) fn blob_upload_multipart_rejection_error(
    status: StatusCode,
) -> (StatusCode, Json<ApiErrorResp>) {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return blob_upload_api_error(status, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE);
    }
    blob_upload_api_error(
        StatusCode::BAD_REQUEST,
        "Image attachment upload was not valid multipart form data.",
    )
}
