use super::*;

pub(crate) async fn start_qwen_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<ProviderLoginStartRouteRequest>,
) -> Result<Json<ProviderLoginStartRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    Ok(Json(providers.start_qwen_login_for_route(req).await))
}

pub(crate) async fn get_qwen_login(
    State(providers): State<ProvidersHandle>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::QwenLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    providers
        .qwen_login_status_for_route(&id)
        .await
        .map(Json)
        .map_err(provider_login_route_error)
}
