use super::host::{seed_codex_auth_from_host, seeding_codex_auth_from_host_enabled};
use super::secret_store::{
    codex_auth_has_refresh_token, codex_auth_has_supported_shape,
    hydrate_legacy_account_auth_to_broker_home, ingest_auth_value_for_account,
    load_codex_auth_from_secret_store, project_auth_value_to_home,
    project_oauth_auth_to_broker_home_with_lock,
};
use super::*;

#[derive(Debug)]
struct PreparedCodexRuntimeAuth {
    home: PathBuf,
    has_auth: bool,
}

fn codex_env_for_home(home: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("CODEX_HOME".to_string(), home.to_string_lossy().to_string());
    env
}

pub async fn codex_env_for_runtime_home(state_root: &Path) -> Result<HashMap<String, String>> {
    let runtime_home = codex_runtime_home(state_root);
    ctx_fs::permissions::ensure_private_dir(&runtime_home).await?;
    Ok(codex_env_for_home(&runtime_home))
}

async fn read_auth_value_from_home(home: &Path) -> Result<Option<serde_json::Value>> {
    let auth_path = home.join("auth.json");
    let payload = match tokio::fs::read_to_string(&auth_path).await {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading codex auth at {}", auth_path.display()));
        }
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", auth_path.display()))?;
    Ok(Some(auth))
}

pub(crate) async fn write_runtime_owner_marker(data_root: &Path, account_id: &str) -> Result<()> {
    let marker = codex_runtime_owner_path(data_root);
    write_secure_file_atomic(&marker, account_id.as_bytes()).await
}

async fn write_broker_owner_marker(data_root: &Path, account_id: &str) -> Result<()> {
    let marker = codex_broker_home(data_root, account_id).join(".ctx-auth-authority");
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

pub(crate) async fn clear_runtime_auth_projection_for_runtime_roots(
    data_root: &Path,
    account_id: &str,
) -> Result<()> {
    clear_runtime_auth_projection_if_owned_by(data_root, account_id).await?;
    for runtime_root in container_runtime_data_roots(data_root).await {
        clear_runtime_auth_projection_if_owned_by(&runtime_root, account_id).await?;
    }
    Ok(())
}

async fn clear_runtime_auth_projection_if_owned_by(
    data_root: &Path,
    account_id: &str,
) -> Result<()> {
    let owner = read_runtime_owner_marker(data_root).await?;
    if owner.as_deref() == Some(account_id) {
        clear_runtime_auth_projection(data_root).await?;
    }
    Ok(())
}

async fn project_secret_to_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
    account_id: &str,
    secret_ref: &str,
) -> Result<bool> {
    let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    if codex_auth_has_refresh_token(&auth) {
        anyhow::bail!(
            "Codex OAuth credentials cannot be copied into an isolated runtime root. Sign in through ctx on the host or select a Codex endpoint for this sandboxed workspace."
        );
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    write_runtime_owner_marker(runtime_root, account_id).await?;
    Ok(projected)
}

async fn project_secret_to_runtime_home(
    data_root: &Path,
    account_id: &str,
    secret_ref: &str,
) -> Result<bool> {
    let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    if codex_auth_has_refresh_token(&auth) {
        anyhow::bail!(
            "Codex OAuth credentials must run from their broker-owned home, not the shared runtime projection"
        );
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(data_root), &auth).await?;
    write_runtime_owner_marker(data_root, account_id).await?;
    Ok(projected)
}

async fn mirror_account_auth_to_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let src = codex_account_dir(data_root, account_id).join("auth.json");
    let auth = match read_auth_value_from_home(&codex_account_dir(data_root, account_id)).await? {
        Some(auth) => auth,
        None => return Ok(false),
    };
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    if codex_auth_has_refresh_token(&auth) {
        anyhow::bail!(
            "Codex OAuth credentials cannot be copied from {} into an isolated runtime root. Complete the login through ctx on the host or select a Codex endpoint.",
            src.display()
        );
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    write_runtime_owner_marker(runtime_root, account_id).await?;
    Ok(projected)
}

async fn mirror_account_auth_to_runtime_home(data_root: &Path, account_id: &str) -> Result<bool> {
    let auth = match read_auth_value_from_home(&codex_account_dir(data_root, account_id)).await? {
        Some(auth) => auth,
        None => return Ok(false),
    };
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    if codex_auth_has_refresh_token(&auth) {
        anyhow::bail!(
            "Codex OAuth credentials must be adopted into the broker-owned home before launch"
        );
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(data_root), &auth).await?;
    write_runtime_owner_marker(data_root, account_id).await?;
    Ok(projected)
}

async fn prepare_broker_home_from_secret(
    data_root: &Path,
    account_id: &str,
    secret_ref: &str,
) -> Result<PathBuf> {
    ensure_safe_account_id(account_id)?;
    let broker_home = codex_broker_home(data_root, account_id);
    ctx_fs::permissions::ensure_private_dir(&broker_home).await?;
    let secret_auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    if !codex_auth_has_refresh_token(&secret_auth) {
        anyhow::bail!("codex secret for account {account_id} is not an OAuth credential");
    }

    if let Some(auth) = read_auth_value_from_home(&broker_home).await? {
        if !codex_auth_has_supported_shape(&auth) {
            anyhow::bail!(
                "codex broker auth at {} has unsupported auth shape",
                broker_home.join("auth.json").display()
            );
        }
        if !codex_auth_has_refresh_token(&auth) {
            anyhow::bail!(
                "codex broker auth at {} is not an OAuth refresh-token credential",
                broker_home.join("auth.json").display()
            );
        }
        write_broker_owner_marker(data_root, account_id).await?;
        return Ok(broker_home);
    }

    project_oauth_auth_to_broker_home_with_lock(data_root, account_id, &secret_auth).await?;
    write_broker_owner_marker(data_root, account_id).await?;
    Ok(broker_home)
}

pub(super) async fn migrate_owned_runtime_oauth_projection_to_broker_if_needed(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    if read_runtime_owner_marker(data_root).await?.as_deref() != Some(account_id) {
        return Ok(false);
    }
    let broker_home = codex_broker_home(data_root, account_id);
    if let Some(broker_auth) = read_auth_value_from_home(&broker_home).await? {
        if !codex_auth_has_supported_shape(&broker_auth) {
            anyhow::bail!(
                "codex broker auth at {} has unsupported auth shape",
                broker_home.join("auth.json").display()
            );
        }
        if codex_auth_has_refresh_token(&broker_auth) {
            clear_runtime_auth_projection_if_owned_by(data_root, account_id).await?;
        }
        return Ok(false);
    }

    let runtime_home = codex_runtime_home(data_root);
    let Some(auth) = read_auth_value_from_home(&runtime_home).await? else {
        return Ok(false);
    };
    if !codex_auth_has_supported_shape(&auth) || !codex_auth_has_refresh_token(&auth) {
        return Ok(false);
    }

    ingest_auth_value_for_account(data_root, account_id, &auth).await?;
    project_oauth_auth_to_broker_home_with_lock(data_root, account_id, &auth).await?;
    clear_runtime_auth_projection_if_owned_by(data_root, account_id).await?;
    Ok(true)
}

async fn prepare_broker_home_from_legacy_account_auth(
    data_root: &Path,
    account_id: &str,
) -> Result<Option<PathBuf>> {
    let broker_home =
        hydrate_legacy_account_auth_to_broker_home(data_root, account_id, false).await?;
    if broker_home.is_some() {
        write_broker_owner_marker(data_root, account_id).await?;
    }
    Ok(broker_home)
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

pub(crate) async fn ingest_runtime_home_auth_to_active_secret(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let owner = read_runtime_owner_marker(data_root).await?;
    if owner.as_deref() != Some(account_id) {
        return Ok(false);
    }
    let auth = match read_auth_value_from_home(&codex_runtime_home(data_root)).await? {
        Some(auth) => auth,
        None => return Ok(false),
    };
    ingest_auth_value_for_account(data_root, account_id, &auth).await
}

async fn prepare_codex_runtime_auth_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<PreparedCodexRuntimeAuth> {
    if let Ok(value) = std::env::var("CTX_CODEX_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let dir = PathBuf::from(trimmed);
            ctx_fs::permissions::ensure_private_dir(&dir).await?;
            let has_auth = ensure_codex_auth_ready(&dir).await.is_ok();
            return Ok(PreparedCodexRuntimeAuth {
                home: dir,
                has_auth,
            });
        }
    }

    let registry = load_codex_registry(data_root).await?;
    if let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if let Some(entry) = registry.accounts.iter().find(|a| a.id == active) {
            if codex_account_deletion_in_progress(data_root, active).await? {
                clear_runtime_auth_projection(runtime_root).await?;
                return Ok(PreparedCodexRuntimeAuth {
                    home: codex_runtime_home(runtime_root),
                    has_auth: false,
                });
            }
            ensure_codex_endpoint_profile_compatible(&entry.endpoint_profile)?;
            if let Some(secret_ref) = entry.secret_ref.as_deref() {
                let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
                if codex_auth_has_refresh_token(&auth) {
                    if data_root != runtime_root {
                        clear_runtime_auth_projection(runtime_root).await?;
                        anyhow::bail!(
                            "Codex OAuth subscription auth is bound to its broker-owned host home and cannot be copied into this sandbox runtime. Select a Codex endpoint for sandbox execution or run the Codex session on the host."
                        );
                    }
                    migrate_owned_runtime_oauth_projection_to_broker_if_needed(data_root, active)
                        .await?;
                    let broker_home =
                        prepare_broker_home_from_secret(data_root, active, secret_ref).await?;
                    return Ok(PreparedCodexRuntimeAuth {
                        home: broker_home.clone(),
                        has_auth: ensure_codex_auth_ready(&broker_home).await.is_ok(),
                    });
                }
                let _ = ingest_runtime_home_auth_to_active_secret(data_root, active).await;
                if data_root == runtime_root {
                    let projected =
                        project_secret_to_runtime_home(data_root, active, secret_ref).await?;
                    let runtime_home = codex_runtime_home(data_root);
                    return Ok(PreparedCodexRuntimeAuth {
                        home: runtime_home.clone(),
                        has_auth: projected || ensure_codex_auth_ready(&runtime_home).await.is_ok(),
                    });
                }
                let projected =
                    project_secret_to_runtime_root(data_root, runtime_root, active, secret_ref)
                        .await?;
                let runtime_home = codex_runtime_home(runtime_root);
                return Ok(PreparedCodexRuntimeAuth {
                    home: runtime_home.clone(),
                    has_auth: projected || ensure_codex_auth_ready(&runtime_home).await.is_ok(),
                });
            }
        } else {
            clear_runtime_auth_projection(runtime_root).await?;
            return Ok(PreparedCodexRuntimeAuth {
                home: codex_runtime_home(runtime_root),
                has_auth: false,
            });
        }

        let _ = ingest_runtime_home_auth_to_active_secret(data_root, active).await;
        if data_root == runtime_root {
            if let Some(broker_home) =
                prepare_broker_home_from_legacy_account_auth(data_root, active).await?
            {
                return Ok(PreparedCodexRuntimeAuth {
                    home: broker_home.clone(),
                    has_auth: ensure_codex_auth_ready(&broker_home).await.is_ok(),
                });
            }
            let mirrored = mirror_account_auth_to_runtime_home(data_root, active).await?;
            let runtime_home = codex_runtime_home(data_root);
            if !mirrored && ensure_codex_auth_ready(&runtime_home).await.is_err() {
                clear_runtime_auth_projection(runtime_root).await?;
                return Ok(PreparedCodexRuntimeAuth {
                    home: runtime_home,
                    has_auth: false,
                });
            }
            return Ok(PreparedCodexRuntimeAuth {
                home: runtime_home.clone(),
                has_auth: ensure_codex_auth_ready(&runtime_home).await.is_ok(),
            });
        }

        let runtime_home = codex_runtime_home(runtime_root);
        let mirrored = mirror_account_auth_to_runtime_root(data_root, runtime_root, active).await?;
        if !mirrored && ensure_codex_auth_ready(&runtime_home).await.is_err() {
            clear_runtime_auth_projection(runtime_root).await?;
            return Ok(PreparedCodexRuntimeAuth {
                home: runtime_home,
                has_auth: false,
            });
        }
        return Ok(PreparedCodexRuntimeAuth {
            home: runtime_home.clone(),
            has_auth: ensure_codex_auth_ready(&runtime_home).await.is_ok(),
        });
    }

    clear_runtime_auth_projection(runtime_root).await?;
    let runtime_home = codex_runtime_home(runtime_root);
    if seeding_codex_auth_from_host_enabled() {
        seed_codex_auth_from_host(&runtime_home).await?;
        return Ok(PreparedCodexRuntimeAuth {
            home: runtime_home.clone(),
            has_auth: ensure_codex_auth_ready(&runtime_home).await.is_ok(),
        });
    }
    Ok(PreparedCodexRuntimeAuth {
        home: runtime_home,
        has_auth: false,
    })
}

pub async fn codex_has_active_auth(data_root: &Path) -> Result<bool> {
    prepare_codex_runtime_auth_with_runtime_root(data_root, data_root)
        .await
        .map(|prepared| prepared.has_auth)
}

pub async fn codex_has_active_auth_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<bool> {
    prepare_codex_runtime_auth_with_runtime_root(data_root, runtime_root)
        .await
        .map(|prepared| prepared.has_auth)
}

pub async fn codex_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    if let Ok(value) = std::env::var("CTX_CODEX_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let dir = PathBuf::from(trimmed);
            ctx_fs::permissions::ensure_private_dir(&dir).await?;
            return Ok(codex_env_for_home(&dir));
        }
    }

    let prepared = prepare_codex_runtime_auth_with_runtime_root(data_root, data_root).await?;
    ctx_fs::permissions::ensure_private_dir(&prepared.home).await?;
    Ok(codex_env_for_home(&prepared.home))
}

pub async fn codex_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<HashMap<String, String>> {
    if data_root == runtime_root {
        return codex_env_for_active_account(data_root).await;
    }
    let prepared = prepare_codex_runtime_auth_with_runtime_root(data_root, runtime_root).await?;
    ctx_fs::permissions::ensure_private_dir(&prepared.home).await?;
    Ok(codex_env_for_home(&prepared.home))
}
