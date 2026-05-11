use std::time::Duration;

use axum::http::StatusCode;
use url::Url;

pub(super) enum CallbackReplayError {
    InvalidCallbackUrl(String),
    BuildClient(String),
    Request(String),
    NonSuccess(StatusCode),
}

impl CallbackReplayError {
    pub(super) fn status_code(&self) -> StatusCode {
        match self {
            Self::InvalidCallbackUrl(_) => StatusCode::BAD_REQUEST,
            Self::BuildClient(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Request(_) | Self::NonSuccess(_) => StatusCode::BAD_GATEWAY,
        }
    }

    pub(super) fn should_restore_completion_token(&self) -> bool {
        matches!(self, Self::Request(_) | Self::NonSuccess(_))
    }

    pub(super) fn into_message(self) -> String {
        match self {
            Self::InvalidCallbackUrl(err) => format!("invalid callback_url: {err}"),
            Self::BuildClient(err) => format!("failed to build callback replay client: {err}"),
            Self::Request(err) => format!("failed to replay callback: {err}"),
            Self::NonSuccess(status) => format!("callback replay returned {status}"),
        }
    }
}

fn callback_replay_client(callback_url: &str) -> Result<reqwest::Client, CallbackReplayError> {
    let parsed_callback = Url::parse(callback_url)
        .map_err(|err| CallbackReplayError::InvalidCallbackUrl(err.to_string()))?;
    let callback_host = parsed_callback
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    let builder = if callback_host == "localhost" {
        builder.resolve(
            "localhost",
            std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0),
        )
    } else {
        builder
    };
    builder
        .build()
        .map_err(|err| CallbackReplayError::BuildClient(err.to_string()))
}

pub(super) async fn replay_codex_callback(callback_url: &str) -> Result<u16, CallbackReplayError> {
    let client = callback_replay_client(callback_url)?;
    let response = client
        .get(callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|err| CallbackReplayError::Request(err.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(CallbackReplayError::NonSuccess(status));
    }
    Ok(status.as_u16())
}
