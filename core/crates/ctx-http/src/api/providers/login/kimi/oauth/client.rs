use anyhow::Context;
use axum::http::StatusCode;

use super::{
    kimi_oauth_host, KimiDeviceAuthorizationResp, KimiTokenErrorResp, KimiTokenSuccessResp,
    KIMI_CODE_CLIENT_ID,
};

pub(in crate::api::providers::login::kimi) async fn request_kimi_device_authorization(
) -> anyhow::Result<KimiDeviceAuthorizationResp> {
    let url = format!(
        "{}/api/oauth/device_authorization",
        kimi_oauth_host().trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(&url)
        .form(&[("client_id", KIMI_CODE_CLIENT_ID)])
        .send()
        .await
        .context("requesting Kimi device authorization")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Kimi device authorization failed ({status}): {body}");
    }
    serde_json::from_str::<KimiDeviceAuthorizationResp>(&body)
        .context("parsing Kimi device authorization response")
}

pub(in crate::api::providers::login::kimi) async fn poll_kimi_token(
    device_code: &str,
) -> anyhow::Result<Result<KimiTokenSuccessResp, KimiTokenErrorResp>> {
    let url = format!(
        "{}/api/oauth/token",
        kimi_oauth_host().trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(&url)
        .form(&[
            ("client_id", KIMI_CODE_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .context("polling Kimi token endpoint")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == StatusCode::OK {
        let token = serde_json::from_str::<KimiTokenSuccessResp>(&body)
            .context("parsing Kimi token success response")?;
        return Ok(Ok(token));
    }
    if status.is_client_error() {
        let error =
            serde_json::from_str::<KimiTokenErrorResp>(&body).unwrap_or(KimiTokenErrorResp {
                error: Some("oauth_error".to_string()),
                error_description: Some(body),
            });
        return Ok(Err(error));
    }
    anyhow::bail!("Kimi token polling failed ({status}): {body}");
}
