use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;

const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const KIMI_LOGIN_POLL_INTERVAL_FALLBACK: Duration = Duration::from_secs(5);
const KIMI_DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";

#[path = "oauth/client.rs"]
mod client;

pub(super) use client::{poll_kimi_token, request_kimi_device_authorization};

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
