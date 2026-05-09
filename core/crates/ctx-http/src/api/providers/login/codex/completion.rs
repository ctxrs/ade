use super::*;

async fn restore_completion_token(state: &Arc<AppState>, id: &str, completion_token: &str) {
    let mut map = state.providers.codex_login_sessions.lock().await;
    if let Some(status) = map.get_mut(id) {
        if status.status == "pending" && status.completion_token.is_none() {
            status.completion_token = Some(completion_token.to_string());
        }
    }
}

pub(crate) async fn complete_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let expected_callback = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        let Some(status) = map.get_mut(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is not pending".to_string(),
                }),
            ));
        }
        if status.completion_token.as_deref() != Some(req.completion_token.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "invalid completion token".to_string(),
                }),
            ));
        }
        let Some(expected_callback) = status.expected_callback_url.clone() else {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is missing expected callback metadata".to_string(),
                }),
            ));
        };
        status.completion_token = None;
        expected_callback
    };

    if let Err(err) = validate_callback_url(&req.callback_url, Some(expected_callback.as_str())) {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: err.to_string(),
            }),
        ));
    }

    let parsed_callback = Url::parse(&req.callback_url).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid callback_url: {err}"),
            }),
        )
    })?;
    let callback_host = parsed_callback
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let client = if callback_host == "localhost" {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .resolve(
                "localhost",
                std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
            )
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    } else {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to build callback replay client: {err}"),
                    }),
                )
            })?
    };

    let response = match client
        .get(&req.callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            restore_completion_token(&state, &id, &req.completion_token).await;
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: format!("failed to replay callback: {err}"),
                }),
            ));
        }
    };
    let status = response.status();
    if !status.is_success() {
        restore_completion_token(&state, &id, &req.completion_token).await;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("callback replay returned {status}"),
            }),
        ));
    }
    let status_code = status.as_u16();

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
}
