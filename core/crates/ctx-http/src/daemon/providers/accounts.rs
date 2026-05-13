use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use chrono::Utc;
use ctx_provider_accounts as provider_accounts;

use crate::daemon::AppState;

pub(crate) struct CodexAccountsSnapshot {
    pub(crate) active_account_id: Option<String>,
    pub(crate) accounts: Vec<provider_accounts::CodexAccountEntry>,
    pub(crate) logins: Vec<provider_accounts::CodexLoginStatus>,
}

pub(crate) struct PreparedCodexLoginStart {
    pub(crate) account_id: String,
    pub(crate) label: String,
    pub(crate) account_dir: PathBuf,
    pub(crate) codex_bin: String,
}

pub(crate) struct PreparedAmpLoginPaths {
    pub(crate) login_home: PathBuf,
    pub(crate) workdir: PathBuf,
    pub(crate) amp_home: PathBuf,
}

pub(crate) struct PreparedMistralLoginPaths {
    pub(crate) login_home: PathBuf,
    pub(crate) workdir: PathBuf,
    pub(crate) mistral_home: PathBuf,
}

pub(crate) struct PreparedGeminiLoginPaths {
    pub(crate) login_home: PathBuf,
    pub(crate) workdir: PathBuf,
    pub(crate) oauth_path: PathBuf,
    pub(crate) google_accounts_path: PathBuf,
}

pub(crate) struct PreparedQwenLoginPaths {
    pub(crate) login_home: PathBuf,
    pub(crate) workdir: PathBuf,
    pub(crate) oauth_path: PathBuf,
}

#[derive(Debug)]
pub(crate) enum ProviderAccountMutationError {
    BadRequest(anyhow::Error),
    Delete(anyhow::Error),
    Internal(anyhow::Error),
}

pub(crate) struct ProviderAccountLoginMutation {
    pub(crate) active_account_id: Option<String>,
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

    pub(crate) fn into_restart_result(self) -> (Option<String>, anyhow::Result<()>) {
        let restart_result = match self.restart_error {
            Some(err) => Err(err),
            None => Ok(()),
        };
        (self.active_account_id, restart_result)
    }

    pub(crate) fn into_http_result(self) -> Result<(), ProviderAccountMutationError> {
        match self.restart_error {
            Some(err) => Err(ProviderAccountMutationError::Internal(err)),
            None => Ok(()),
        }
    }

    pub(crate) fn restart_error_message(&self) -> Option<String> {
        self.restart_error
            .as_ref()
            .map(|err| format!("auth saved but provider restart failed: {err:#}"))
    }
}

impl ProviderAccountMutationError {
    pub(crate) fn auth_login_error_message(&self) -> String {
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

pub(crate) async fn load_amp_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::load_amp_registry(&state.core.data_root).await
}

pub(crate) async fn ensure_amp_account_registry_from_runtime_auth(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::AmpAccountRegistry> {
    provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await
}

pub(crate) async fn load_claude_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::ClaudeAccountRegistry> {
    provider_accounts::load_claude_registry(&state.core.data_root).await
}

pub(crate) async fn load_codex_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CodexAccountRegistry> {
    provider_accounts::load_codex_registry(&state.core.data_root).await
}

pub(crate) async fn load_codex_accounts_snapshot(
    state: &Arc<AppState>,
) -> anyhow::Result<CodexAccountsSnapshot> {
    let registry = load_codex_account_registry(state).await?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| map.values().cloned().collect::<Vec<_>>())
        .await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn probe_host_codex_auth_candidate() -> provider_accounts::CodexHostImportProbe {
    provider_accounts::probe_host_codex_auth_candidate().await
}

pub(crate) async fn prepare_codex_login_start(
    state: &Arc<AppState>,
    label: Option<String>,
) -> anyhow::Result<PreparedCodexLoginStart> {
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = provider_accounts::normalize_label(label, &account_id);
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, &account_id)
            .await
            .with_context(|| format!("creating codex account directory for {account_id}"))?;

    let prep_result = async {
        let (cfg, managed_config_error) = ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
        if let Some(error) = managed_config_error {
            anyhow::bail!(error);
        }
        let codex_bin = ctx_managed_installs::require_codex_cli_command_path_for_target(
            &cfg,
            Some(ctx_provider_install::install_state::InstallTarget::Host),
        )
        .context("resolving managed Codex CLI command")?;
        Ok(codex_bin)
    }
    .await;

    match prep_result {
        Ok(codex_bin) => Ok(PreparedCodexLoginStart {
            account_id,
            label,
            account_dir,
            codex_bin,
        }),
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&account_dir).await;
            Err(err)
        }
    }
}

fn provider_login_home(data_root: &Path, provider_id: &str, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join(provider_id)
        .join("login-sessions")
        .join(login_id)
}

fn login_provider_base_env(state: &AppState) -> HashMap<String, String> {
    HashMap::from([
        ("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone()),
        ("CTX_MCP_DISABLED".to_string(), "1".to_string()),
        (
            "CTX_DATA_ROOT".to_string(),
            state.core.data_root.to_string_lossy().to_string(),
        ),
    ])
}

pub(crate) async fn prepare_amp_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<PreparedAmpLoginPaths, String> {
    let login_home = provider_login_home(&state.core.data_root, "amp", login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        return Err(format!("failed to prepare login workspace: {err}"));
    }
    let amp_home = match provider_accounts::ensure_amp_runtime_home(&state.core.data_root).await {
        Ok(home) => home,
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return Err(format!("failed to prepare amp runtime home: {err}"));
        }
    };

    Ok(PreparedAmpLoginPaths {
        login_home,
        workdir,
        amp_home,
    })
}

pub(crate) fn amp_login_provider_env(state: &AppState, amp_home: &Path) -> HashMap<String, String> {
    let mut provider_env = login_provider_base_env(state);
    provider_env.insert("HOME".to_string(), amp_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        amp_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        amp_home.join(".cache").to_string_lossy().to_string(),
    );
    provider_env
}

pub(crate) async fn prepare_mistral_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<PreparedMistralLoginPaths, String> {
    let login_home = provider_login_home(&state.core.data_root, "mistral", login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        return Err(format!("failed to prepare login workspace: {err}"));
    }
    let mistral_home =
        match provider_accounts::ensure_mistral_runtime_home(&state.core.data_root).await {
            Ok(home) => home,
            Err(err) => {
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return Err(format!("failed to prepare mistral runtime home: {err}"));
            }
        };

    Ok(PreparedMistralLoginPaths {
        login_home,
        workdir,
        mistral_home,
    })
}

pub(crate) fn mistral_login_provider_env(
    state: &AppState,
    mistral_home: &Path,
) -> HashMap<String, String> {
    let mut provider_env = login_provider_base_env(state);
    provider_env.insert(
        "HOME".to_string(),
        mistral_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        mistral_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        mistral_home.join(".cache").to_string_lossy().to_string(),
    );
    provider_env
}

pub(crate) async fn prepare_gemini_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<PreparedGeminiLoginPaths, String> {
    let login_home = provider_login_home(&state.core.data_root, "gemini", login_id);
    let workdir = login_home.join("workspace");
    tokio::fs::create_dir_all(&workdir)
        .await
        .map_err(|err| format!("failed to prepare login workspace: {err}"))?;
    Ok(PreparedGeminiLoginPaths {
        oauth_path: login_home.join(".gemini").join("oauth_creds.json"),
        google_accounts_path: login_home.join(".gemini").join("google_accounts.json"),
        login_home,
        workdir,
    })
}

pub(crate) fn gemini_login_provider_env(
    state: &AppState,
    login_home: &Path,
) -> HashMap<String, String> {
    let mut provider_env = login_provider_base_env(state);
    provider_env.insert(
        "GEMINI_CLI_HOME".to_string(),
        login_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        provider_accounts::GEMINI_FORCE_FILE_STORAGE_ENV.to_string(),
        "true".to_string(),
    );
    provider_env
}

pub(crate) fn gemini_login_auth_method_id() -> String {
    provider_accounts::GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()
}

pub(crate) async fn prepare_qwen_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<PreparedQwenLoginPaths, String> {
    let login_home = provider_login_home(&state.core.data_root, "qwen", login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        return Err(format!("failed to prepare login workspace: {err}"));
    }
    let _ = tokio::fs::create_dir_all(login_home.join(".config")).await;
    let _ = tokio::fs::create_dir_all(login_home.join(".cache")).await;
    let oauth_path = login_home.join(provider_accounts::QWEN_OAUTH_CREDS_RELATIVE_PATH);

    Ok(PreparedQwenLoginPaths {
        login_home,
        workdir,
        oauth_path,
    })
}

pub(crate) fn qwen_login_provider_env(
    state: &AppState,
    login_home: &Path,
) -> HashMap<String, String> {
    let mut provider_env = login_provider_base_env(state);
    provider_env.insert("HOME".to_string(), login_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        login_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        login_home.join(".cache").to_string_lossy().to_string(),
    );
    provider_env
}

pub(crate) async fn import_host_codex_auth(
    state: &Arc<AppState>,
    label: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, label)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn set_active_codex_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    provider_accounts::set_active_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    load_codex_accounts_snapshot(state)
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_codex_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<CodexAccountsSnapshot, ProviderAccountMutationError> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restart_codex_providers_for_auth_change(state, "codex auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    let logins = state
        .providers
        .with_codex_login_sessions(|map| {
            map.remove(account_id);
            map.values().cloned().collect::<Vec<_>>()
        })
        .await;
    Ok(CodexAccountsSnapshot {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    })
}

pub(crate) async fn persist_successful_codex_login(
    state: &Arc<AppState>,
    account_id: &str,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
) -> anyhow::Result<()> {
    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.to_string(),
        label,
        kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
        email,
        plan_type,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: None,
        endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
    };
    provider_accounts::upsert_codex_account(&state.core.data_root, entry)
        .await
        .with_context(|| format!("persisting codex account {account_id}"))?;

    let persist_result = async {
        let ingested = provider_accounts::ingest_codex_account_auth_to_secret_store(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("ingesting codex auth for account {account_id}"))?;
        if !ingested {
            anyhow::bail!("missing persisted codex auth file for account {account_id}");
        }
        provider_accounts::remove_codex_account_home_auth_if_present(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("removing account-home codex auth for account {account_id}"))?;
        provider_accounts::set_active_codex_account(
            &state.core.data_root,
            Some(account_id.to_string()),
        )
        .await
        .with_context(|| format!("setting active codex account {account_id}"))?;
        super::restart_codex_providers_for_auth_change(state, "codex auth updated")
            .await
            .context("restarting codex providers after auth change")?;
        Ok(())
    }
    .await;

    if let Err(err) = persist_result {
        let _ = provider_accounts::remove_codex_account(&state.core.data_root, account_id).await;
        return Err(err);
    }

    Ok(())
}

pub(crate) async fn load_copilot_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CopilotAccountRegistry> {
    provider_accounts::load_copilot_registry(&state.core.data_root).await
}

pub(crate) async fn load_cursor_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::CursorAccountRegistry> {
    provider_accounts::load_cursor_registry(&state.core.data_root).await
}

pub(crate) async fn load_gemini_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::GeminiAccountRegistry> {
    provider_accounts::load_gemini_registry(&state.core.data_root).await
}

pub(crate) async fn load_kimi_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::KimiAccountRegistry> {
    provider_accounts::load_kimi_registry(&state.core.data_root).await
}

pub(crate) async fn load_mistral_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::MistralAccountRegistry> {
    provider_accounts::load_mistral_registry(&state.core.data_root).await
}

pub(crate) async fn load_qwen_account_registry(
    state: &Arc<AppState>,
) -> anyhow::Result<provider_accounts::QwenAccountRegistry> {
    provider_accounts::load_qwen_registry(&state.core.data_root).await
}

pub(crate) async fn upsert_amp_account(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_amp_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn upsert_amp_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::upsert_amp_account(&state.core.data_root, label, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_amp_providers_for_auth_change(state, "amp auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_amp_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_amp_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_amp_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_amp_providers_for_auth_change(state, "amp auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_claude_account(
    state: &Arc<AppState>,
    label: Option<String>,
    setup_token: String,
) -> Result<(), ProviderAccountMutationError> {
    add_claude_account_for_login(state, label, setup_token)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_claude_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    setup_token: String,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_claude_account(&state.core.data_root, label, setup_token)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
            .await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_claude_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_claude_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_claude_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_claude_providers_for_auth_change(state, "claude auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn upsert_mistral_account(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    upsert_mistral_account_for_login(state, label, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn upsert_mistral_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::upsert_mistral_account(&state.core.data_root, label, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
            .await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_mistral_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_mistral_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_mistral_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_mistral_providers_for_auth_change(state, "mistral auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_copilot_account(
    state: &Arc<AppState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_copilot_account(&state.core.data_root, label, token, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn set_active_copilot_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_copilot_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_copilot_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_copilot_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_copilot_providers_for_auth_change(state, "copilot auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_cursor_account(
    state: &Arc<AppState>,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_cursor_account(&state.core.data_root, label, token, email)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_cursor_oauth_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    auth_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_cursor_oauth_account(
        &state.core.data_root,
        label,
        auth_token,
        refresh_token,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
            .await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_cursor_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_cursor_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_cursor_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_cursor_providers_for_auth_change(state, "cursor auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_gemini_account(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_gemini_account_for_login(state, label, oauth_creds_json, google_accounts_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_gemini_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_gemini_account(
        &state.core.data_root,
        label,
        oauth_creds_json,
        google_accounts_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
            .await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_gemini_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_gemini_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_gemini_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_gemini_providers_for_auth_change(state, "gemini auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_kimi_account(
    state: &Arc<AppState>,
    label: Option<String>,
    provider: Option<String>,
    credentials_json: String,
    config_toml: Option<String>,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::add_kimi_account(
        &state.core.data_root,
        label,
        provider,
        credentials_json,
        config_toml,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)?;
    Ok(())
}

pub(crate) async fn add_kimi_oauth_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    credentials_json: String,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry = provider_accounts::add_kimi_oauth_account(
        &state.core.data_root,
        label,
        credentials_json,
        email,
    )
    .await
    .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_kimi_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_kimi_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_kimi_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_kimi_providers_for_auth_change(state, "kimi auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn add_qwen_account(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    add_qwen_account_for_login(state, label, oauth_creds_json, email)
        .await
        .and_then(ProviderAccountLoginMutation::into_http_result)
}

pub(crate) async fn add_qwen_account_for_login(
    state: &Arc<AppState>,
    label: Option<String>,
    oauth_creds_json: String,
    email: Option<String>,
) -> Result<ProviderAccountLoginMutation, ProviderAccountMutationError> {
    let registry =
        provider_accounts::add_qwen_account(&state.core.data_root, label, oauth_creds_json, email)
            .await
            .map_err(ProviderAccountMutationError::BadRequest)?;
    let restart_result =
        super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated").await;
    Ok(ProviderAccountLoginMutation::from_restart_result(
        registry.active_account_id,
        restart_result,
    ))
}

pub(crate) async fn set_active_qwen_account(
    state: &Arc<AppState>,
    account_id: Option<String>,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::set_active_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::BadRequest)?;
    super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}

pub(crate) async fn remove_qwen_account(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(), ProviderAccountMutationError> {
    provider_accounts::remove_qwen_account(&state.core.data_root, account_id)
        .await
        .map_err(ProviderAccountMutationError::Delete)?;
    super::restarts::restart_qwen_providers_for_auth_change(state, "qwen auth updated")
        .await
        .map_err(ProviderAccountMutationError::Internal)
}
