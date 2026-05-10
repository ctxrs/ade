use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct CodexActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(alias = "auth_token")]
    pub(in crate::api::providers) setup_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::api::providers) google_accounts_json: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) oauth_creds_json: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeminiActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QwenActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) provider: Option<String>,
    pub(in crate::api::providers) credentials_json: String,
    #[serde(default)]
    pub(in crate::api::providers) config_toml: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KimiActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MistralActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) token: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopilotActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorAccountUpsertReq {
    pub(in crate::api::providers) label: Option<String>,
    pub(in crate::api::providers) token: String,
    #[serde(default)]
    pub(in crate::api::providers) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CursorActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AmpActiveAccountReq {
    pub(in crate::api::providers) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexHostImportReq {
    pub(in crate::api::providers) label: Option<String>,
}
