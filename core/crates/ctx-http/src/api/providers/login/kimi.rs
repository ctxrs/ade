use super::*;

const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const KIMI_LOGIN_POLL_INTERVAL_FALLBACK: Duration = Duration::from_secs(5);
const KIMI_DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";

#[derive(Debug, Deserialize)]
pub(crate) struct KimiLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KimiLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KimiDeviceAuthorizationResp {
    user_code: String,
    device_code: String,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct KimiTokenSuccessResp {
    access_token: String,
    refresh_token: String,
    expires_in: f64,
    scope: String,
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct KimiTokenErrorResp {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
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

async fn request_kimi_device_authorization() -> anyhow::Result<KimiDeviceAuthorizationResp> {
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

async fn poll_kimi_token(
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

fn kimi_token_json(token: &KimiTokenSuccessResp) -> String {
    serde_json::json!({
        "access_token": token.access_token,
        "refresh_token": token.refresh_token,
        "expires_at": (Utc::now().timestamp_millis() as f64 / 1000.0) + token.expires_in,
        "scope": token.scope,
        "token_type": token.token_type,
    })
    .to_string()
}

async fn monitor_kimi_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    device_code: String,
    poll_interval: Duration,
    timeout: Duration,
) {
    let started_at = Instant::now();

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.kimi_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Kimi sign-in completion".to_string());
                }
            }
            return;
        }

        match poll_kimi_token(&device_code).await {
            Ok(Ok(token)) => {
                let added = provider_accounts::add_kimi_oauth_account(
                    &state.core.data_root,
                    label.clone(),
                    kimi_token_json(&token),
                    None,
                )
                .await;
                match added {
                    Ok(registry) => {
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "success".to_string();
                            entry.account_id = registry.active_account_id.clone();
                            entry.error = None;
                        }
                        restarts::restart_kimi_providers_for_auth_change(
                            &state,
                            "kimi auth updated",
                        )
                        .await;
                    }
                    Err(err) => {
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    }
                }
                return;
            }
            Ok(Err(error)) => {
                let error_code = error.error.as_deref().unwrap_or("oauth_error");
                if matches!(
                    error_code,
                    "authorization_pending" | "slow_down" | "access_denied"
                ) {
                    if error_code == "access_denied" {
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(
                                error
                                    .error_description
                                    .unwrap_or_else(|| "Kimi sign-in was denied.".to_string()),
                            );
                        }
                        return;
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                let mut map = state.providers.kimi_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = if error_code == "expired_token" {
                        "timeout".to_string()
                    } else {
                        "failed".to_string()
                    };
                    entry.error = Some(
                        error
                            .error_description
                            .unwrap_or_else(|| format!("Kimi sign-in failed: {error_code}")),
                    );
                }
                return;
            }
            Err(err) => {
                let mut map = state.providers.kimi_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(logs::redact_sensitive(&err.to_string()));
                }
                return;
            }
        }
    }
}

pub(crate) async fn start_kimi_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiLoginStartReq>,
) -> Result<Json<KimiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let auth = request_kimi_device_authorization().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        )
    })?;
    let login_id = uuid::Uuid::new_v4().to_string();
    let auth_url = auth
        .verification_uri_complete
        .clone()
        .or(auth.verification_uri.clone());
    let device_code = Some(auth.user_code.clone());
    {
        let mut map = state.providers.kimi_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::KimiLoginStatus {
                login_id: login_id.clone(),
                status: "pending".to_string(),
                account_id: None,
                auth_url: auth_url.clone(),
                device_code: device_code.clone(),
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    let poll_interval = auth
        .interval
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or(KIMI_LOGIN_POLL_INTERVAL_FALLBACK);
    let timeout = auth
        .expires_in
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .map(|value| value.min(kimi_login_timeout()))
        .unwrap_or_else(kimi_login_timeout);
    tokio::spawn(async move {
        monitor_kimi_login(
            state_clone,
            login_id_for_task,
            req.label,
            auth.device_code,
            poll_interval,
            timeout,
        )
        .await;
    });

    Ok(Json(KimiLoginStartResp {
        login_id,
        auth_url,
        device_code,
    }))
}

pub(crate) async fn get_kimi_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::KimiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.kimi_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
