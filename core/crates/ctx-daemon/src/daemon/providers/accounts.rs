use std::fmt;

mod codex;
mod login_paths;
mod mutations;
mod routes;

pub use codex::{
    import_host_codex_auth, load_codex_account_registry, load_codex_accounts_snapshot,
    persist_successful_codex_login, prepare_codex_login_start, probe_host_codex_auth_candidate,
    remove_codex_account, set_active_codex_account, CodexAccountsSnapshot,
};
pub use login_paths::{
    amp_login_provider_env, gemini_login_provider_env, mistral_login_provider_env,
    prepare_amp_login_paths, prepare_gemini_login_paths, prepare_mistral_login_paths,
    prepare_qwen_login_paths, qwen_login_provider_env, PreparedGeminiLoginPaths,
    PreparedQwenLoginPaths,
};
pub use mutations::{
    add_claude_account, add_claude_account_for_login, add_copilot_account, add_cursor_account,
    add_cursor_oauth_account_for_login, add_gemini_account, add_gemini_account_for_login,
    add_kimi_account, add_kimi_oauth_account_for_login, add_qwen_account,
    add_qwen_account_for_login, ensure_amp_account_registry_from_runtime_auth,
    load_amp_account_registry, load_claude_account_registry, load_copilot_account_registry,
    load_cursor_account_registry, load_gemini_account_registry, load_kimi_account_registry,
    load_mistral_account_registry, load_qwen_account_registry, remove_amp_account,
    remove_claude_account, remove_copilot_account, remove_cursor_account, remove_gemini_account,
    remove_kimi_account, remove_mistral_account, remove_qwen_account, set_active_amp_account,
    set_active_claude_account, set_active_copilot_account, set_active_cursor_account,
    set_active_gemini_account, set_active_kimi_account, set_active_mistral_account,
    set_active_qwen_account, upsert_amp_account, upsert_amp_account_for_login,
    upsert_mistral_account, upsert_mistral_account_for_login,
};
#[derive(Debug)]
pub enum ProviderAccountMutationError {
    BadRequest(anyhow::Error),
    Delete(anyhow::Error),
    Internal(anyhow::Error),
}

pub struct ProviderAccountLoginMutation {
    pub active_account_id: Option<String>,
    restart_error: Option<anyhow::Error>,
}

impl ProviderAccountLoginMutation {
    fn from_restart_result(
        active_account_id: Option<String>,
        restart_result: anyhow::Result<()>,
    ) -> Self {
        Self {
            active_account_id,
            restart_error: restart_result.err(),
        }
    }

    pub fn into_restart_result(self) -> (Option<String>, anyhow::Result<()>) {
        let restart_result = match self.restart_error {
            Some(err) => Err(err),
            None => Ok(()),
        };
        (self.active_account_id, restart_result)
    }

    pub fn into_http_result(self) -> Result<(), ProviderAccountMutationError> {
        match self.restart_error {
            Some(err) => Err(ProviderAccountMutationError::Internal(err)),
            None => Ok(()),
        }
    }

    pub fn restart_error_message(&self) -> Option<String> {
        self.restart_error
            .as_ref()
            .map(|err| format!("auth saved but provider restart failed: {err:#}"))
    }
}

impl ProviderAccountMutationError {
    pub fn auth_login_error_message(&self) -> String {
        match self {
            Self::Internal(err) => format!("auth saved but provider restart failed: {err:#}"),
            Self::BadRequest(err) | Self::Delete(err) => err.to_string(),
        }
    }
}

impl fmt::Display for ProviderAccountMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(err) | Self::Delete(err) | Self::Internal(err) => {
                write!(f, "{err:#}")
            }
        }
    }
}
