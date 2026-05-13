use super::*;
use crate::daemon::mobile_access as daemon_mobile_access;

pub(in crate::api) async fn get_mobile_access_status(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<MobileAccessStatus>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let snapshot = daemon_mobile_access::mobile_access_status(&state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(MobileAccessStatus {
        enabled: snapshot.enabled,
        tunnel_id: snapshot.tunnel_id,
        public_base_url: snapshot.public_base_url,
        relay_base_url: snapshot.relay_base_url,
        daemon_public_key: snapshot.daemon_public_key,
        tunnel_state: snapshot.tunnel_state,
        last_error: snapshot.last_error,
    }))
}
