use super::*;

pub(super) struct AmpLoginPaths {
    pub(super) login_home: PathBuf,
    pub(super) workdir: PathBuf,
    pub(super) amp_home: PathBuf,
}

fn amp_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("amp")
        .join("login-sessions")
        .join(login_id)
}

pub(super) async fn prepare_amp_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<AmpLoginPaths, String> {
    let login_home = amp_login_home(&state.core.data_root, login_id);
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

    Ok(AmpLoginPaths {
        login_home,
        workdir,
        amp_home,
    })
}

pub(super) fn amp_provider_env(
    state: &Arc<AppState>,
    amp_home: &StdPath,
) -> HashMap<String, String> {
    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
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
