use ctx_provider_accounts::{
    AmpLoginStatusRouteResponse, GeminiLoginStatusRouteResponse, KimiLoginStatusRouteResponse,
    MistralLoginStatusRouteResponse, ProviderLoginRouteError, ProviderLoginStartRouteRequest,
    ProviderLoginStartRouteResponse, QwenLoginStatusRouteResponse,
};

use crate::daemon::ProvidersHandle;

use super::login_sessions::StartedLoginSession;
use super::{browser_logins, kimi_oauth_login, login_sessions};

impl ProvidersHandle {
    pub async fn start_amp_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        provider_login_start_response(
            browser_logins::start_amp_browser_login(&self.state, request.into_label()).await,
        )
    }

    pub async fn start_gemini_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        provider_login_start_response(
            browser_logins::start_gemini_browser_login(&self.state, request.into_label()).await,
        )
    }

    pub async fn start_qwen_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        provider_login_start_response(
            browser_logins::start_qwen_browser_login(&self.state, request.into_label()).await,
        )
    }

    pub async fn start_mistral_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> ProviderLoginStartRouteResponse {
        provider_login_start_response(
            browser_logins::start_mistral_browser_login(&self.state, request.into_label()).await,
        )
    }

    pub async fn start_kimi_login_for_route(
        &self,
        request: ProviderLoginStartRouteRequest,
    ) -> Result<ProviderLoginStartRouteResponse, ProviderLoginRouteError> {
        kimi_oauth_login::start_kimi_oauth_login(&self.state, request.into_label())
            .await
            .map(provider_login_start_response)
            .map_err(kimi_login_start_route_error)
    }

    pub async fn amp_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<AmpLoginStatusRouteResponse, ProviderLoginRouteError> {
        login_sessions::amp_login_status(&self.state, login_id)
            .await
            .map(Into::into)
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn gemini_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<GeminiLoginStatusRouteResponse, ProviderLoginRouteError> {
        login_sessions::gemini_login_status(&self.state, login_id)
            .await
            .map(Into::into)
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn qwen_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<QwenLoginStatusRouteResponse, ProviderLoginRouteError> {
        login_sessions::qwen_login_status(&self.state, login_id)
            .await
            .map(Into::into)
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn mistral_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<MistralLoginStatusRouteResponse, ProviderLoginRouteError> {
        login_sessions::mistral_login_status(&self.state, login_id)
            .await
            .map(Into::into)
            .ok_or_else(login_not_found_route_error)
    }

    pub async fn kimi_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<KimiLoginStatusRouteResponse, ProviderLoginRouteError> {
        login_sessions::kimi_login_status(&self.state, login_id)
            .await
            .map(Into::into)
            .ok_or_else(login_not_found_route_error)
    }
}

fn provider_login_start_response(session: StartedLoginSession) -> ProviderLoginStartRouteResponse {
    ProviderLoginStartRouteResponse::new(session.login_id, session.auth_url, session.device_code)
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
    use ctx_provider_accounts::{self as provider_accounts, ProviderLoginRouteErrorKind};

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
        let payload = serde_json::to_value(provider_login_start_response(StartedLoginSession {
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
        let payload = serde_json::to_value(provider_login_start_response(StartedLoginSession {
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

    #[test]
    fn provider_login_status_route_responses_match_provider_account_wire_shape() {
        let amp = provider_accounts::AmpLoginStatus {
            login_id: "amp-login".to_string(),
            auth_url: None,
            status: "pending".to_string(),
            error: None,
        };
        assert_eq!(
            serde_json::to_value(AmpLoginStatusRouteResponse::from(amp.clone())).unwrap(),
            serde_json::to_value(amp).unwrap()
        );

        let gemini = provider_accounts::GeminiLoginStatus {
            login_id: "gemini-login".to_string(),
            auth_url: None,
            status: "complete".to_string(),
            account_id: None,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(GeminiLoginStatusRouteResponse::from(gemini.clone())).unwrap(),
            serde_json::to_value(gemini).unwrap()
        );

        let qwen = provider_accounts::QwenLoginStatus {
            login_id: "qwen-login".to_string(),
            auth_url: None,
            status: "error".to_string(),
            account_id: None,
            error: Some("failed".to_string()),
        };
        assert_eq!(
            serde_json::to_value(QwenLoginStatusRouteResponse::from(qwen.clone())).unwrap(),
            serde_json::to_value(qwen).unwrap()
        );

        let mistral = provider_accounts::MistralLoginStatus {
            login_id: "mistral-login".to_string(),
            auth_url: None,
            status: "pending".to_string(),
            error: None,
        };
        assert_eq!(
            serde_json::to_value(MistralLoginStatusRouteResponse::from(mistral.clone())).unwrap(),
            serde_json::to_value(mistral).unwrap()
        );

        let kimi = provider_accounts::KimiLoginStatus {
            login_id: "kimi-login".to_string(),
            status: "pending".to_string(),
            account_id: None,
            auth_url: None,
            device_code: None,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(KimiLoginStatusRouteResponse::from(kimi.clone())).unwrap(),
            serde_json::to_value(kimi).unwrap()
        );
    }
}
