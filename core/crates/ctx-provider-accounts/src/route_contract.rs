use serde::{Deserialize, Serialize};

use crate::provider_accounts;

#[derive(Debug, Deserialize)]
pub struct ProviderActiveAccountRouteRequest {
    account_id: Option<String>,
}

impl ProviderActiveAccountRouteRequest {
    pub fn new(account_id: Option<String>) -> Self {
        Self { account_id }
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn into_account_id(self) -> Option<String> {
        self.account_id
    }
}

#[derive(Debug, Deserialize)]
pub struct CodexHostImportRouteRequest {
    label: Option<String>,
}

impl CodexHostImportRouteRequest {
    pub fn new(label: Option<String>) -> Self {
        Self { label }
    }

    pub fn into_label(self) -> Option<String> {
        self.label
    }
}

#[derive(Debug, Deserialize)]
pub struct ClaudeAccountUpsertRouteRequest {
    label: Option<String>,
    #[serde(alias = "auth_token")]
    setup_token: String,
}

impl ClaudeAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, setup_token: String) -> Self {
        Self { label, setup_token }
    }

    pub fn into_parts(self) -> (Option<String>, String) {
        (self.label, self.setup_token)
    }
}

#[derive(Debug, Deserialize)]
pub struct GeminiAccountUpsertRouteRequest {
    label: Option<String>,
    oauth_creds_json: String,
    #[serde(default)]
    google_accounts_json: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl GeminiAccountUpsertRouteRequest {
    pub fn new(
        label: Option<String>,
        oauth_creds_json: String,
        google_accounts_json: Option<String>,
        email: Option<String>,
    ) -> Self {
        Self {
            label,
            oauth_creds_json,
            google_accounts_json,
            email,
        }
    }

    pub fn into_parts(self) -> (Option<String>, String, Option<String>, Option<String>) {
        (
            self.label,
            self.oauth_creds_json,
            self.google_accounts_json,
            self.email,
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct QwenAccountUpsertRouteRequest {
    label: Option<String>,
    oauth_creds_json: String,
    #[serde(default)]
    email: Option<String>,
}

impl QwenAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, oauth_creds_json: String, email: Option<String>) -> Self {
        Self {
            label,
            oauth_creds_json,
            email,
        }
    }

    pub fn into_parts(self) -> (Option<String>, String, Option<String>) {
        (self.label, self.oauth_creds_json, self.email)
    }
}

#[derive(Debug, Deserialize)]
pub struct AmpAccountUpsertRouteRequest {
    label: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl AmpAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, email: Option<String>) -> Self {
        Self { label, email }
    }

    pub fn into_parts(self) -> (Option<String>, Option<String>) {
        (self.label, self.email)
    }
}

#[derive(Debug, Deserialize)]
pub struct MistralAccountUpsertRouteRequest {
    label: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl MistralAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, email: Option<String>) -> Self {
        Self { label, email }
    }

    pub fn into_parts(self) -> (Option<String>, Option<String>) {
        (self.label, self.email)
    }
}

#[derive(Debug, Deserialize)]
pub struct KimiAccountUpsertRouteRequest {
    label: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    credentials_json: String,
    #[serde(default)]
    config_toml: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl KimiAccountUpsertRouteRequest {
    pub fn new(
        label: Option<String>,
        provider: Option<String>,
        credentials_json: String,
        config_toml: Option<String>,
        email: Option<String>,
    ) -> Self {
        Self {
            label,
            provider,
            credentials_json,
            config_toml,
            email,
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    ) {
        (
            self.label,
            self.provider,
            self.credentials_json,
            self.config_toml,
            self.email,
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct CopilotAccountUpsertRouteRequest {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

impl CopilotAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, token: String, email: Option<String>) -> Self {
        Self {
            label,
            token,
            email,
        }
    }

    pub fn into_parts(self) -> (Option<String>, String, Option<String>) {
        (self.label, self.token, self.email)
    }
}

#[derive(Debug, Deserialize)]
pub struct CursorAccountUpsertRouteRequest {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

impl CursorAccountUpsertRouteRequest {
    pub fn new(label: Option<String>, token: String, email: Option<String>) -> Self {
        Self {
            label,
            token,
            email,
        }
    }

    pub fn into_parts(self) -> (Option<String>, String, Option<String>) {
        (self.label, self.token, self.email)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexHostImportProbeRouteResponse {
    available: bool,
    path: Option<String>,
    auth_kind: Option<String>,
    error: Option<String>,
}

impl From<provider_accounts::CodexHostImportProbe> for CodexHostImportProbeRouteResponse {
    fn from(probe: provider_accounts::CodexHostImportProbe) -> Self {
        Self {
            available: probe.available,
            path: probe.path,
            auth_kind: probe.auth_kind,
            error: probe.error,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CodexAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CodexAccountEntry>,
    logins: Vec<provider_accounts::CodexLoginStatus>,
}

impl CodexAccountsResponse {
    pub fn new(
        active_account_id: Option<String>,
        accounts: Vec<provider_accounts::CodexAccountEntry>,
        logins: Vec<provider_accounts::CodexLoginStatus>,
    ) -> Self {
        Self {
            active_account_id,
            accounts,
            logins,
        }
    }
}

macro_rules! account_response {
    ($response:ident, $entry:ty, $registry:ty) => {
        #[derive(Debug, Serialize)]
        pub struct $response {
            active_account_id: Option<String>,
            accounts: Vec<$entry>,
        }

        impl $response {
            pub fn new(active_account_id: Option<String>, accounts: Vec<$entry>) -> Self {
                Self {
                    active_account_id,
                    accounts,
                }
            }
        }

        impl From<$registry> for $response {
            fn from(registry: $registry) -> Self {
                Self::new(registry.active_account_id, registry.accounts)
            }
        }
    };
}

account_response!(
    ClaudeAccountsResponse,
    provider_accounts::ClaudeAccountEntry,
    provider_accounts::ClaudeAccountRegistry
);
account_response!(
    GeminiAccountsResponse,
    provider_accounts::GeminiAccountEntry,
    provider_accounts::GeminiAccountRegistry
);
account_response!(
    QwenAccountsResponse,
    provider_accounts::QwenAccountEntry,
    provider_accounts::QwenAccountRegistry
);
account_response!(
    KimiAccountsResponse,
    provider_accounts::KimiAccountEntry,
    provider_accounts::KimiAccountRegistry
);
account_response!(
    MistralAccountsResponse,
    provider_accounts::MistralAccountEntry,
    provider_accounts::MistralAccountRegistry
);
account_response!(
    CopilotAccountsResponse,
    provider_accounts::CopilotAccountEntry,
    provider_accounts::CopilotAccountRegistry
);
account_response!(
    CursorAccountsResponse,
    provider_accounts::CursorAccountEntry,
    provider_accounts::CursorAccountRegistry
);
account_response!(
    AmpAccountsResponse,
    provider_accounts::AmpAccountEntry,
    provider_accounts::AmpAccountRegistry
);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProviderAccountRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProviderAccountRouteError {
    kind: ProviderAccountRouteErrorKind,
    message: String,
}

impl ProviderAccountRouteError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderAccountRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ProviderAccountRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
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

        let (label, setup_token) = request.into_parts();
        assert_eq!(label.as_deref(), Some("Claude"));
        assert_eq!(setup_token, "token");
    }

    #[test]
    fn provider_upsert_route_requests_preserve_optional_defaults() {
        let gemini: GeminiAccountUpsertRouteRequest = serde_json::from_value(json!({
            "oauth_creds_json": "{}"
        }))
        .expect("deserialize gemini");
        let (label, _, google_accounts_json, email) = gemini.into_parts();
        assert!(label.is_none());
        assert!(google_accounts_json.is_none());
        assert!(email.is_none());

        let kimi: KimiAccountUpsertRouteRequest = serde_json::from_value(json!({
            "credentials_json": "{}"
        }))
        .expect("deserialize kimi");
        let (label, provider, _, config_toml, email) = kimi.into_parts();
        assert!(label.is_none());
        assert!(provider.is_none());
        assert!(config_toml.is_none());
        assert!(email.is_none());
    }

    #[test]
    fn active_account_request_omitted_and_null_account_ids_clear_active_account() {
        let omitted: ProviderActiveAccountRouteRequest =
            serde_json::from_value(json!({})).expect("deserialize omitted account id");
        let null: ProviderActiveAccountRouteRequest =
            serde_json::from_value(json!({ "account_id": null }))
                .expect("deserialize null account id");

        assert_eq!(omitted.account_id(), None);
        assert_eq!(omitted.into_account_id(), None);
        assert_eq!(null.account_id(), None);
        assert_eq!(null.into_account_id(), None);
    }

    #[test]
    fn codex_host_import_probe_route_response_preserves_wire_shape() {
        let response =
            CodexHostImportProbeRouteResponse::from(provider_accounts::CodexHostImportProbe {
                available: false,
                path: None,
                auth_kind: None,
                error: Some("missing auth".to_string()),
            });

        assert_eq!(
            serde_json::to_value(response).expect("serialize probe"),
            json!({
                "available": false,
                "path": null,
                "auth_kind": null,
                "error": "missing auth",
            })
        );
    }

    #[test]
    fn provider_account_route_error_preserves_kind_and_message() {
        let error = ProviderAccountRouteError::not_found("unknown account");

        assert_eq!(error.kind(), ProviderAccountRouteErrorKind::NotFound);
        assert_eq!(error.message(), "unknown account");
    }
}
