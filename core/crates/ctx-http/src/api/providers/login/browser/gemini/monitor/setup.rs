use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use crate::daemon::AppState;
use ctx_provider_accounts as provider_accounts;

pub(super) struct GeminiLoginPaths {
    pub(super) login_home: PathBuf,
    pub(super) workdir: PathBuf,
    pub(super) oauth_path: PathBuf,
    pub(super) google_accounts_path: PathBuf,
}

pub(super) async fn prepare_gemini_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<GeminiLoginPaths, String> {
    let login_home = gemini_login_home(&state.core.data_root, login_id);
    let workdir = login_home.join("workspace");
    tokio::fs::create_dir_all(&workdir)
        .await
        .map_err(|err| format!("failed to prepare login workspace: {err}"))?;
    Ok(GeminiLoginPaths {
        oauth_path: login_home.join(".gemini").join("oauth_creds.json"),
        google_accounts_path: login_home.join(".gemini").join("google_accounts.json"),
        login_home,
        workdir,
    })
}

pub(super) fn gemini_provider_env(
    state: &AppState,
    login_home: &StdPath,
) -> HashMap<String, String> {
    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    provider_env.insert(
        "GEMINI_CLI_HOME".to_string(),
        login_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        provider_accounts::GEMINI_FORCE_FILE_STORAGE_ENV.to_string(),
        "true".to_string(),
    );
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env
}

fn gemini_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("gemini")
        .join("login-sessions")
        .join(login_id)
}
