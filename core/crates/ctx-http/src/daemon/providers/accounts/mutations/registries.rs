use std::sync::Arc;

use ctx_provider_accounts as provider_accounts;

use crate::daemon::DaemonState;

pub(crate) async fn load_amp_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::load_amp_registry(&state.core.data_root).await
}

pub(crate) async fn ensure_amp_account_registry_from_runtime_auth(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await
}

pub(crate) async fn load_claude_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::ClaudeAccountRegistry> {
    provider_accounts::load_claude_registry(&state.core.data_root).await
}

pub(crate) async fn load_copilot_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::CopilotAccountRegistry> {
    provider_accounts::load_copilot_registry(&state.core.data_root).await
}

pub(crate) async fn load_cursor_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::CursorAccountRegistry> {
    provider_accounts::load_cursor_registry(&state.core.data_root).await
}

pub(crate) async fn load_gemini_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::GeminiAccountRegistry> {
    provider_accounts::load_gemini_registry(&state.core.data_root).await
}

pub(crate) async fn load_kimi_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::KimiAccountRegistry> {
    provider_accounts::load_kimi_registry(&state.core.data_root).await
}

pub(crate) async fn load_mistral_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::MistralAccountRegistry> {
    provider_accounts::load_mistral_registry(&state.core.data_root).await
}

pub(crate) async fn load_qwen_account_registry(
    state: &Arc<DaemonState>,
) -> anyhow::Result<provider_accounts::QwenAccountRegistry> {
    provider_accounts::load_qwen_registry(&state.core.data_root).await
}
