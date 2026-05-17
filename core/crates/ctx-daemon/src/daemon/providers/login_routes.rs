use ctx_provider_accounts as provider_accounts;
use serde::{Deserialize, Serialize};

use crate::daemon::ProvidersHandle;

use super::login_sessions::StartedLoginSession;
use super::{browser_logins, kimi_oauth_login, login_sessions};

#[derive(Debug, Default, Deserialize)]
pub struct ProviderLoginStartRouteRequest {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderLoginStartRouteResponse {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_code: Option<String>,
}

impl From<StartedLoginSession> for ProviderLoginStartRouteResponse {
    fn from(session: StartedLoginSession) -> Self {
        Self {
            login_id: session.login_id,
            auth_url: session.auth_url,
            device_code: session.device_code,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLoginRouteErrorKind {
    NotFound,
    BadGateway,
}

#[derive(Debug)]
pub struct ProviderLoginRouteError {
    kind: ProviderLoginRouteErrorKind,
    message: String,
}

impl ProviderLoginRouteError {
    pub fn kind(&self) -> ProviderLoginRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderLoginRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderLoginRouteErrorKind::BadGateway,
            message: message.into(),
        }
    }
}

impl ProvidersHandle {
    pub async fn start_amp_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        browser_logins::start_amp_browser_login(&self.state, request.label)
            .await
            .into()
    }

    pub async fn start_gemini_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        browser_logins::start_gemini_browser_login(&self.state, request.label)
            .await
            .into()
    }

    pub async fn start_qwen_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        browser_logins::start_qwen_browser_login(&self.state, request.label)
            .await
            .into()
    }

    pub async fn start_mistral_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        browser_logins::start_mistral_browser_login(&self.state, request.label)
            .await
            .into()
    }

    pub async fn start_kimi_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> Result<ProviderLoginStartRouteResponse, ProviderLoginRouteError> {
        kimi_oauth_login::start_kimi_oauth_login(&self.state, request.label)
            .await
            .map(Into::into)
            .map_err(kimi_login_start_route_error)
    }

    pub async fn amp_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::AmpLoginStatus, ProviderLoginRouteError> {
        login_sessions::amp_login_status(&self.state, login_id)
            .await
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn gemini_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::GeminiLoginStatus, ProviderLoginRouteError> {
        login_sessions::gemini_login_status(&self.state, login_id)
            .await
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn qwen_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::QwenLoginStatus, ProviderLoginRouteError> {
        login_sessions::qwen_login_status(&self.state, login_id)
            .await
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn mistral_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::MistralLoginStatus, ProviderLoginRouteError> {
        login_sessions::mistral_login_status(&self.state, login_id)
            .await
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn kimi_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::KimiLoginStatus, ProviderLoginRouteError> {
        login_sessions::kimi_login_status(&self.state, login_id)
            .await
            .ok_or_else(login_not_found_route_error)
    }
}

fn login_not_found_route_error() -> ProviderLoginRouteError {
    ProviderLoginRouteError::not_found("login not found")
}

fn kimi_login_start_route_error(
    error: kimi_oauth_login::KimiOAuthLoginStartError,
) -> ProviderLoginRouteError {
    ProviderLoginRouteError::bad_gateway(error.route_safe_message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_login_route_not_found_error_preserves_body_message() {
        let error = login_not_found_route_error();

        assert_eq!(error.kind(), ProviderLoginRouteErrorKind::NotFound);
        assert_eq!(error.message(), "login not found");
    }

    #[test]
    fn provider_login_route_kimi_start_error_preserves_redacted_message() {
        let error = kimi_login_start_route_error(
            kimi_oauth_login::KimiOAuthLoginStartError::for_route_test(
                "failed to reach Kimi OAuth",
            ),
        );

        assert_eq!(error.kind(), ProviderLoginRouteErrorKind::BadGateway);
        assert_eq!(error.message(), "failed to reach Kimi OAuth");
    }

    #[test]
    fn provider_login_route_start_response_omits_absent_optional_fields() {
        let payload =
            serde_json::to_value(ProviderLoginStartRouteResponse::from(StartedLoginSession {
                login_id: "login-1".to_string(),
                auth_url: None,
                device_code: None,
            }))
            .unwrap();

        assert_eq!(payload["login_id"].as_str(), Some("login-1"));
        assert!(payload.get("auth_url").is_none());
        assert!(payload.get("device_code").is_none());
    }

    #[test]
    fn provider_login_route_start_response_preserves_present_optional_fields() {
        let payload =
            serde_json::to_value(ProviderLoginStartRouteResponse::from(StartedLoginSession {
                login_id: "login-2".to_string(),
                auth_url: Some("https://example.test/auth".to_string()),
                device_code: Some("CODE-123".to_string()),
            }))
            .unwrap();

        assert_eq!(payload["login_id"].as_str(), Some("login-2"));
        assert_eq!(
            payload["auth_url"].as_str(),
            Some("https://example.test/auth")
        );
        assert_eq!(payload["device_code"].as_str(), Some("CODE-123"));
    }
}
