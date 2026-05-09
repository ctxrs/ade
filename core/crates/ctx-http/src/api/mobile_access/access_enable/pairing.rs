use super::*;

pub(super) struct MobilePairingBootstrap {
    pub(super) pairing_token: String,
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn create_mobile_pairing_bootstrap(
    state: &Arc<AppState>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MobilePairingBootstrap, (StatusCode, Json<ApiErrorResp>)> {
    let pairing_token = generate_pairing_token();
    let pairing_hash = hash_pairing_token(&pairing_token);
    let expires_at = now + chrono::Duration::seconds(PAIRING_TOKEN_TTL_SECS);
    state
        .global_store()
        .insert_mobile_pairing_token(&uuid::Uuid::new_v4().to_string(), &pairing_hash, expires_at)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to persist pairing token".into(),
                }),
            )
        })?;
    Ok(MobilePairingBootstrap {
        pairing_token,
        expires_at,
    })
}
