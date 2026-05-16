use super::*;

pub(super) fn provider_account_route_error(
    err: ctx_daemon::daemon::providers::ProviderAccountRouteError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match err.kind() {
        ctx_daemon::daemon::providers::ProviderAccountRouteErrorKind::BadRequest => {
            StatusCode::BAD_REQUEST
        }
        ctx_daemon::daemon::providers::ProviderAccountRouteErrorKind::NotFound => {
            StatusCode::NOT_FOUND
        }
        ctx_daemon::daemon::providers::ProviderAccountRouteErrorKind::Internal => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (
        status,
        Json(ApiErrorResp {
            error: err.message().to_string(),
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
