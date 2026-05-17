use super::*;

#[path = "connection_profiles/creation.rs"]
mod creation;

pub(in crate::api) use creation::create_mobile_connection_profile;

pub(in crate::api) async fn list_mobile_connection_profiles(
    State(state): State<CoreHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<Vec<MobileConnectionProfile>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let profiles = state
        .list_mobile_connection_profiles_for_route()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(profiles))
}

pub(in crate::api) async fn delete_mobile_connection_profile(
    State(state): State<CoreHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .delete_mobile_connection_profile_for_route(ConnectionProfileId(uuid))
        .await
        .map_err(|error| match error.kind() {
            MobileAccessRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
            MobileAccessRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            MobileAccessRouteErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
            MobileAccessRouteErrorKind::Forbidden => StatusCode::FORBIDDEN,
            MobileAccessRouteErrorKind::Conflict => StatusCode::CONFLICT,
            MobileAccessRouteErrorKind::BadGateway => StatusCode::BAD_GATEWAY,
            MobileAccessRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    Ok(StatusCode::NO_CONTENT)
}
