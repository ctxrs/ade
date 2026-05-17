use ctx_provider_accounts as provider_accounts;
use serde::Serialize;

use super::super::CodexAccountsSnapshot;

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

#[derive(Debug, Serialize)]
pub struct ClaudeAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct GeminiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct QwenAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct KimiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct MistralAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct CopilotAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct CursorAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub struct AmpAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::AmpAccountEntry>,
}

impl CodexAccountsResponse {
    pub(in crate::daemon::providers) fn from_snapshot(snapshot: CodexAccountsSnapshot) -> Self {
        Self {
            active_account_id: snapshot.active_account_id,
            accounts: snapshot.accounts,
            logins: snapshot.logins,
        }
    }
}

macro_rules! account_response_from_registry {
    ($response:ty, $registry:ty) => {
        impl $response {
            pub(in crate::daemon::providers) fn from_registry(registry: $registry) -> Self {
                Self {
                    active_account_id: registry.active_account_id,
                    accounts: registry.accounts,
                }
            }
        }
    };
}

account_response_from_registry!(
    ClaudeAccountsResponse,
    provider_accounts::ClaudeAccountRegistry
);
account_response_from_registry!(
    GeminiAccountsResponse,
    provider_accounts::GeminiAccountRegistry
);
account_response_from_registry!(QwenAccountsResponse, provider_accounts::QwenAccountRegistry);
account_response_from_registry!(KimiAccountsResponse, provider_accounts::KimiAccountRegistry);
account_response_from_registry!(
    MistralAccountsResponse,
    provider_accounts::MistralAccountRegistry
);
account_response_from_registry!(
    CopilotAccountsResponse,
    provider_accounts::CopilotAccountRegistry
);
account_response_from_registry!(
    CursorAccountsResponse,
    provider_accounts::CursorAccountRegistry
);
account_response_from_registry!(AmpAccountsResponse, provider_accounts::AmpAccountRegistry);

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
}
