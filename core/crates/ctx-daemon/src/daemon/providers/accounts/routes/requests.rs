use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ProviderActiveAccountRouteRequest {
    pub(in crate::daemon::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexHostImportRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    #[serde(alias = "auth_token")]
    pub(in crate::daemon::providers) setup_token: String,
}

#[derive(Debug, Deserialize)]
pub struct GeminiAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    pub(in crate::daemon::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::daemon::providers) google_accounts_json: Option<String>,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QwenAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    pub(in crate::daemon::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AmpAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MistralAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KimiAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::daemon::providers) provider: Option<String>,
    pub(in crate::daemon::providers) credentials_json: String,
    #[serde(default)]
    pub(in crate::daemon::providers) config_toml: Option<String>,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CopilotAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    pub(in crate::daemon::providers) token: String,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CursorAccountUpsertRouteRequest {
    pub(in crate::daemon::providers) label: Option<String>,
    pub(in crate::daemon::providers) token: String,
    #[serde(default)]
    pub(in crate::daemon::providers) email: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn claude_upsert_route_request_accepts_auth_token_alias() {
        let request: ClaudeAccountUpsertRouteRequest =
            serde_json::from_value(json!({ "label": "Claude", "auth_token": "token" }))
                .expect("deserialize alias");

        assert_eq!(request.label.as_deref(), Some("Claude"));
        assert_eq!(request.setup_token, "token");
    }

    #[test]
    fn provider_upsert_route_requests_preserve_optional_defaults() {
        let gemini: GeminiAccountUpsertRouteRequest = serde_json::from_value(json!({
            "oauth_creds_json": "{}"
        }))
        .expect("deserialize gemini");
        assert!(gemini.label.is_none());
        assert!(gemini.google_accounts_json.is_none());
        assert!(gemini.email.is_none());

        let kimi: KimiAccountUpsertRouteRequest = serde_json::from_value(json!({
            "credentials_json": "{}"
        }))
        .expect("deserialize kimi");
        assert!(kimi.label.is_none());
        assert!(kimi.provider.is_none());
        assert!(kimi.config_toml.is_none());
        assert!(kimi.email.is_none());
    }
}
