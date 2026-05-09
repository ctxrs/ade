use std::time::Duration;

use anyhow::Context;
use axum::http::StatusCode;
use chrono::Utc;
use serde::Deserialize;

const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const KIMI_LOGIN_POLL_INTERVAL_FALLBACK: Duration = Duration::from_secs(5);
const KIMI_DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";

#[derive(Debug, Deserialize)]
pub(super) struct KimiDeviceAuthorizationResp {
    pub(super) user_code: String,
    pub(super) device_code: String,
    #[serde(default)]
    pub(super) verification_uri: Option<String>,
    #[serde(default)]
    pub(super) verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiTokenSuccessResp {
    access_token: String,
    refresh_token: String,
    expires_in: f64,
    scope: String,
    token_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiTokenErrorResp {
    #[serde(default)]
    pub(super) error: Option<String>,
    #[serde(default)]
    pub(super) error_description: Option<String>,
}

pub(super) fn poll_interval_for_authorization(auth: &KimiDeviceAuthorizationResp) -> Duration {
    auth.interval
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or(KIMI_LOGIN_POLL_INTERVAL_FALLBACK)
}

pub(super) fn timeout_for_authorization(auth: &KimiDeviceAuthorizationResp) -> Duration {
    auth.expires_in
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .map(|value| value.min(kimi_login_timeout()))
        .unwrap_or_else(kimi_login_timeout)
}

fn kimi_oauth_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("KIMI_OAUTH_HOST")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| KIMI_DEFAULT_OAUTH_HOST.to_string())
}

fn kimi_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_KIMI_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(KIMI_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

pub(super) async fn request_kimi_device_authorization(
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

pub(super) async fn poll_kimi_token(
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

pub(super) fn kimi_token_json(token: &KimiTokenSuccessResp) -> String {
    serde_json::json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token,
        "expires_at": (Utc::now().timestamp_millis() as f64 / 1000.0) + token.expires_in,
        "scope": token.scope,
        "token_type": token.token_type,
    })
    .to_string()
}
