use super::*;

#[path = "connection_profiles/creation.rs"]
mod creation;

pub(in crate::api) use creation::create_mobile_connection_profile;

pub(in crate::api) async fn list_mobile_connection_profiles(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<Vec<MobileConnectionProfile>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let profiles = state
        .global_store()
        .list_mobile_connection_profiles()
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile profiles: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(profiles))
}

pub(in crate::api) async fn delete_mobile_connection_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    if state
        .global_store()
        .get_mobile_connection_profile(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile profile before delete: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .global_store()
        .delete_mobile_connection_profile(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to delete mobile profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(StatusCode::NO_CONTENT)
}
