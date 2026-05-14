use super::*;

pub(super) fn provider_account_delete_error(
    err: anyhow::Error,
) -> (StatusCode, Json<ApiErrorResp>) {
    let error = err.to_string();
    let status = if error.contains("unknown account") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(ApiErrorResp { error }))
}

pub(super) fn provider_account_mutation_error(
    err: ctx_daemon::daemon::providers::ProviderAccountMutationError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match err {
        ctx_daemon::daemon::providers::ProviderAccountMutationError::BadRequest(err) => {
            bad_request(err)
        }
        ctx_daemon::daemon::providers::ProviderAccountMutationError::Delete(err) => {
            provider_account_delete_error(err)
        }
        ctx_daemon::daemon::providers::ProviderAccountMutationError::Internal(err) => {
            internal_error(err)
        }
    }
}

pub(super) fn bad_request(err: impl ToString) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: err.to_string(),
        }),
    )
}

pub(super) fn internal_error(err: impl ToString) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: err.to_string(),
        }),
    )
}

pub(super) fn unknown_account() -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResp {
            error: "unknown account".to_string(),
        }),
    )
}
