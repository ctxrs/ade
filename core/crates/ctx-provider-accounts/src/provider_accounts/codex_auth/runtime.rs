use super::host::{
    mirror_host_codex_auth_to_runtime_root, seed_codex_auth_from_host,
    seeding_codex_auth_from_host_enabled,
};
use super::secret_store::{
    codex_auth_has_supported_shape, ingest_auth_value_for_account,
    load_codex_auth_from_secret_store, project_auth_value_to_home,
};
use super::*;

pub async fn codex_env_for_runtime_home(state_root: &Path) -> Result<HashMap<String, String>> {
    let runtime_home = codex_runtime_home(state_root);
    tokio::fs::create_dir_all(&runtime_home).await?;
    let mut env = HashMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        runtime_home.to_string_lossy().to_string(),
    );
    Ok(env)
}

async fn project_secret_to_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
    account_id: &str,
    secret_ref: &str,
) -> Result<bool> {
    let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    write_runtime_owner_marker(runtime_root, account_id).await?;
    Ok(projected)
}

async fn mirror_account_auth_to_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let src = codex_account_dir(data_root, account_id).join("auth.json");
    let payload = match tokio::fs::read_to_string(&src).await {
        Ok(payload) => payload,
        Err(_) => return Ok(false),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", src.display()))?;
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    write_runtime_owner_marker(runtime_root, account_id).await?;
    Ok(projected)
}

pub(crate) async fn write_runtime_owner_marker(data_root: &Path, account_id: &str) -> Result<()> {
    let marker = codex_runtime_owner_path(data_root);
    write_secure_file_atomic(&marker, account_id.as_bytes()).await
}

async fn read_runtime_owner_marker(data_root: &Path) -> Result<Option<String>> {
    let marker = codex_runtime_owner_path(data_root);
    let value = match tokio::fs::read_to_string(&marker).await {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value.to_string()))
}

pub(crate) async fn clear_runtime_auth_projection(data_root: &Path) -> Result<()> {
    let auth_path = codex_runtime_home(data_root).join("auth.json");
    match tokio::fs::remove_file(&auth_path).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let owner_path = codex_runtime_owner_path(data_root);
    match tokio::fs::remove_file(&owner_path).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

async fn project_secret_to_runtime_home(
    data_root: &Path,
    account_id: &str,
    secret_ref: &str,
) -> Result<bool> {
    let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    let projected = project_auth_value_to_home(&codex_runtime_home(data_root), &auth).await?;
    write_runtime_owner_marker(data_root, account_id).await?;
    Ok(projected)
}

pub async fn ensure_codex_auth_ready(codex_home: &Path) -> Result<()> {
    let auth_path = codex_home.join("auth.json");
    let payload = tokio::fs::read_to_string(&auth_path)
        .await
        .with_context(|| format!("missing codex auth file at {}", auth_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&payload).with_context(|| {
        format!(
            "invalid codex auth file JSON at {}; expected auth.json shape",
            auth_path.display()
        )
    })?;
    if !codex_auth_has_supported_shape(&parsed) {
        anyhow::bail!(
            "codex auth file at {} has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token",
            auth_path.display()
        );
    }
    Ok(())
}

async fn mirror_account_auth_to_runtime_home(data_root: &Path, account_id: &str) -> Result<bool> {
    let src = codex_account_dir(data_root, account_id).join("auth.json");
    let payload = match tokio::fs::read_to_string(&src).await {
        Ok(payload) => payload,
        Err(_) => return Ok(false),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", src.display()))?;
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(data_root), &auth).await?;
    write_runtime_owner_marker(data_root, account_id).await?;
    Ok(projected)
}

pub(crate) async fn ingest_runtime_home_auth_to_active_secret(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let owner = read_runtime_owner_marker(data_root).await?;
    if owner.as_deref() != Some(account_id) {
        return Ok(false);
    }
    let auth_path = codex_runtime_home(data_root).join("auth.json");
    let payload = match tokio::fs::read_to_string(&auth_path).await {
        Ok(payload) => payload,
        Err(_) => return Ok(false),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", auth_path.display()))?;
    ingest_auth_value_for_account(data_root, account_id, &auth).await
}

async fn prepare_codex_runtime_auth_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<bool> {
    if let Ok(value) = std::env::var("CTX_CODEX_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let dir = PathBuf::from(trimmed);
            tokio::fs::create_dir_all(&dir).await?;
            return Ok(ensure_codex_auth_ready(&dir).await.is_ok());
        }
    }

    let registry = load_codex_registry(data_root).await;
    if let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let _ = ingest_runtime_home_auth_to_active_secret(data_root, active).await;
        if let Some(entry) = registry.accounts.iter().find(|a| a.id == active) {
            ensure_codex_endpoint_profile_compatible(&entry.endpoint_profile)?;
            if let Some(secret_ref) = entry.secret_ref.as_deref() {
                if if data_root == runtime_root {
                    project_secret_to_runtime_home(data_root, active, secret_ref)
                        .await
                        .is_ok()
                } else {
                    project_secret_to_runtime_root(data_root, runtime_root, active, secret_ref)
                        .await
                        .is_ok()
                } {
                    return Ok(ensure_codex_auth_ready(&codex_runtime_home(runtime_root))
                        .await
                        .is_ok());
                }
            }
        } else {
            clear_runtime_auth_projection(runtime_root).await?;
            return Ok(false);
        }

        let runtime_home = codex_runtime_home(runtime_root);
        let mirrored = if data_root == runtime_root {
            mirror_account_auth_to_runtime_home(data_root, active).await?
        } else {
            mirror_account_auth_to_runtime_root(data_root, runtime_root, active).await?
        };
        if !mirrored && ensure_codex_auth_ready(&runtime_home).await.is_err() {
            clear_runtime_auth_projection(runtime_root).await?;
            return Ok(false);
        }
        return Ok(ensure_codex_auth_ready(&runtime_home).await.is_ok());
    }

    clear_runtime_auth_projection(runtime_root).await?;
    let runtime_home = codex_runtime_home(runtime_root);
    if seeding_codex_auth_from_host_enabled() {
        seed_codex_auth_from_host(&runtime_home).await?;
        return Ok(ensure_codex_auth_ready(&runtime_home).await.is_ok());
    }
    if mirror_host_codex_auth_to_runtime_root(runtime_root).await? {
        return Ok(ensure_codex_auth_ready(&runtime_home).await.is_ok());
    }
    Ok(false)
}

pub async fn codex_has_active_auth(data_root: &Path) -> Result<bool> {
    prepare_codex_runtime_auth_with_runtime_root(data_root, data_root).await
}

pub async fn codex_has_active_auth_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<bool> {
    prepare_codex_runtime_auth_with_runtime_root(data_root, runtime_root).await
}

pub async fn codex_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    if let Ok(value) = std::env::var("CTX_CODEX_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let dir = PathBuf::from(trimmed);
            tokio::fs::create_dir_all(&dir).await?;
            let mut env = HashMap::new();
            env.insert("CODEX_HOME".to_string(), dir.to_string_lossy().to_string());
            return Ok(env);
        }
    }

    let _ = prepare_codex_runtime_auth_with_runtime_root(data_root, data_root).await?;
    codex_env_for_runtime_home(data_root).await
}

pub async fn codex_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<HashMap<String, String>> {
    if data_root == runtime_root {
        return codex_env_for_active_account(data_root).await;
    }
    let _ = prepare_codex_runtime_auth_with_runtime_root(data_root, runtime_root).await?;
    codex_env_for_runtime_home(runtime_root).await
}
