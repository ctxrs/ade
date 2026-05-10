use super::*;

pub(super) struct QwenLoginPaths {
    pub(super) login_home: PathBuf,
    pub(super) workdir: PathBuf,
    pub(super) oauth_path: PathBuf,
}

fn qwen_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("qwen")
        .join("login-sessions")
        .join(login_id)
}

pub(super) async fn prepare_qwen_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<QwenLoginPaths, String> {
    let login_home = qwen_login_home(&state.core.data_root, login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        return Err(format!("failed to prepare login workspace: {err}"));
    }
    let _ = tokio::fs::create_dir_all(login_home.join(".config")).await;
    let _ = tokio::fs::create_dir_all(login_home.join(".cache")).await;
    let oauth_path = login_home.join(provider_accounts::QWEN_OAUTH_CREDS_RELATIVE_PATH);

    Ok(QwenLoginPaths {
        login_home,
        workdir,
        oauth_path,
    })
}

pub(super) fn qwen_provider_env(
    state: &Arc<AppState>,
    login_home: &StdPath,
) -> HashMap<String, String> {
    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
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
