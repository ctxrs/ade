use super::*;

pub(super) struct BootstrapAccounts {
    pub(super) codex_accounts: CodexAccountsResponse,
    pub(super) claude_accounts: ClaudeAccountsResponse,
    pub(super) gemini_accounts: GeminiAccountsResponse,
    pub(super) qwen_accounts: QwenAccountsResponse,
    pub(super) kimi_accounts: KimiAccountsResponse,
    pub(super) mistral_accounts: MistralAccountsResponse,
    pub(super) copilot_accounts: CopilotAccountsResponse,
    pub(super) cursor_accounts: CursorAccountsResponse,
    pub(super) amp_accounts: AmpAccountsResponse,
}

pub(super) fn bootstrap_accounts_error(
    provider_id: &str,
    err: anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!(
                "failed to load {provider_id} accounts: {}",
                logs::redact_sensitive(&err.to_string())
            ),
        })),
    )
}

pub(super) async fn load_bootstrap_accounts(
    providers: &ProvidersHandle,
) -> Result<BootstrapAccounts, (StatusCode, Json<serde_json::Value>)> {
    Ok(BootstrapAccounts {
        codex_accounts: accounts::codex_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("codex", err))?,
        claude_accounts: accounts::claude_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("claude-crp", err))?,
        gemini_accounts: accounts::gemini_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("gemini", err))?,
        qwen_accounts: accounts::qwen_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("qwen", err))?,
        kimi_accounts: accounts::kimi_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("kimi", err))?,
        mistral_accounts: accounts::mistral_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("mistral", err))?,
        copilot_accounts: accounts::copilot_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("copilot", err))?,
        cursor_accounts: accounts::cursor_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("cursor", err))?,
        amp_accounts: accounts::amp_accounts_response(providers)
            .await
            .map_err(|err| bootstrap_accounts_error("amp", err))?,
    })
}
