use std::collections::HashMap;

use serde::Serialize;

use ctx_harness_sources as harness_sources;
use ctx_providers::adapters::ProviderStatus;

use super::accounts::{
    AmpAccountsResponse, ClaudeAccountsResponse, CodexAccountsResponse, CopilotAccountsResponse,
    CursorAccountsResponse, GeminiAccountsResponse, KimiAccountsResponse, MistralAccountsResponse,
    QwenAccountsResponse,
};

#[derive(Debug, Serialize)]
pub(crate) struct ProvidersBootstrapResponse {
    pub(in crate::api::providers) providers: Vec<ProviderStatus>,
    pub(in crate::api::providers) provider_options: HashMap<String, serde_json::Value>,
    pub(in crate::api::providers) provider_harness_config:
        HashMap<String, harness_sources::HarnessProviderSourceConfig>,
    pub(in crate::api::providers) codex_accounts: CodexAccountsResponse,
    pub(in crate::api::providers) claude_accounts: ClaudeAccountsResponse,
    pub(in crate::api::providers) gemini_accounts: GeminiAccountsResponse,
    pub(in crate::api::providers) qwen_accounts: QwenAccountsResponse,
    pub(in crate::api::providers) kimi_accounts: KimiAccountsResponse,
    pub(in crate::api::providers) mistral_accounts: MistralAccountsResponse,
    pub(in crate::api::providers) copilot_accounts: CopilotAccountsResponse,
    pub(in crate::api::providers) cursor_accounts: CursorAccountsResponse,
    pub(in crate::api::providers) amp_accounts: AmpAccountsResponse,
}
