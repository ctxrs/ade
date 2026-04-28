use super::host::host_codex_auth_path;
use super::*;

pub(super) fn codex_auth_has_supported_shape(value: &serde_json::Value) -> bool {
    let has_api_key = value
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty());
    let has_token_bundle = value
        .get("tokens")
        .and_then(|v| v.as_object())
        .is_some_and(|tokens| {
            let access = tokens
                .get("access_token")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.trim().is_empty());
            let refresh = tokens
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.trim().is_empty());
            access && refresh
        });
    has_api_key || has_token_bundle
}

pub(super) fn codex_auth_kind(value: &serde_json::Value) -> Option<String> {
    let has_api_key = value
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty());
    let has_token_bundle = value
        .get("tokens")
        .and_then(|v| v.as_object())
        .is_some_and(|tokens| {
            let access = tokens
                .get("access_token")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.trim().is_empty());
            let refresh = tokens
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.trim().is_empty());
            access && refresh
        });
    if has_token_bundle {
        return Some(CODEX_CREDENTIAL_KIND_OAUTH.to_string());
    }
    if has_api_key {
        return Some(CODEX_CREDENTIAL_KIND_API_KEY.to_string());
    }
    None
}

async fn write_codex_secret_for_account(
    data_root: &Path,
    account_id: &str,
    auth: &serde_json::Value,
) -> Result<String> {
    if !codex_auth_has_supported_shape(auth) {
        anyhow::bail!(
            "codex auth has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token"
        );
    }
    let secret_ref = format!("{account_id}.json");
    let secret_path = codex_secret_path(data_root, &secret_ref)?;
    let envelope = CodexSecretEnvelope {
        version: CODEX_SECRET_VERSION,
        auth: auth.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&envelope)?;
    write_secure_file_atomic(&secret_path, &bytes).await?;
    Ok(secret_ref)
}

async fn update_account_secret_ref(
    data_root: &Path,
    account_id: &str,
    secret_ref: String,
    kind: Option<String>,
) -> Result<()> {
    let mut registry = load_codex_registry(data_root).await?;
    if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == account_id) {
        entry.secret_ref = Some(secret_ref);
        entry.kind = kind.unwrap_or_else(default_codex_credential_kind);
        save_codex_registry(data_root, &registry).await?;
    }
    Ok(())
}

pub(super) async fn ingest_auth_value_for_account(
    data_root: &Path,
    account_id: &str,
    auth: &serde_json::Value,
) -> Result<bool> {
    if !codex_auth_has_supported_shape(auth) {
        anyhow::bail!(
            "codex auth has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token"
        );
    }
    let secret_ref = write_codex_secret_for_account(data_root, account_id, auth).await?;
    let kind = codex_auth_kind(auth);
    update_account_secret_ref(data_root, account_id, secret_ref, kind).await?;
    Ok(true)
}

async fn find_matching_codex_account(
    data_root: &Path,
    auth: &serde_json::Value,
) -> Result<Option<CodexAccountEntry>> {
    let registry = load_codex_registry(data_root).await?;
    for existing in &registry.accounts {
        let existing_auth = if let Some(secret_ref) = existing.secret_ref.as_deref() {
            load_codex_auth_from_secret_store(data_root, secret_ref).await?
        } else {
            let existing_auth_path = codex_account_dir(data_root, &existing.id).join("auth.json");
            match tokio::fs::read_to_string(&existing_auth_path).await {
                Ok(existing_payload) => {
                    serde_json::from_str(&existing_payload).with_context(|| {
                        format!(
                            "invalid codex auth JSON at {}",
                            existing_auth_path.display()
                        )
                    })?
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "reading existing codex auth at {}",
                            existing_auth_path.display()
                        )
                    });
                }
            }
        };
        if existing_auth == *auth {
            return Ok(Some(existing.clone()));
        }
    }
    Ok(None)
}

async fn remove_codex_account_home_auth_if_present(
    data_root: &Path,
    account_id: &str,
) -> Result<()> {
    let auth_path = codex_account_dir(data_root, account_id).join("auth.json");
    match tokio::fs::remove_file(&auth_path).await {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("removing legacy codex auth at {}", auth_path.display())),
    }
}

pub async fn import_codex_auth_value_to_secret_store(
    data_root: &Path,
    label: Option<String>,
    auth: &serde_json::Value,
) -> Result<CodexAuthImportOutcome> {
    let kind = codex_auth_kind(auth).ok_or_else(|| {
        anyhow!("codex auth has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token")
    })?;

    if let Some(existing) = find_matching_codex_account(data_root, auth).await? {
        if existing.secret_ref.is_none() {
            ingest_auth_value_for_account(data_root, &existing.id, auth).await?;
            remove_codex_account_home_auth_if_present(data_root, &existing.id).await?;
        }
        let registry = set_active_codex_account(data_root, Some(existing.id.clone())).await?;
        return Ok(CodexAuthImportOutcome {
            registry,
            account_id: existing.id,
            created: false,
        });
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_codex_secret_for_account(data_root, &account_id, auth).await?;
    let entry = CodexAccountEntry {
        id: account_id.clone(),
        label: normalize_label(label, &account_id),
        kind,
        email: None,
        plan_type: None,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
        endpoint_profile: CodexEndpointProfile::default(),
    };
    let _ = upsert_codex_account(data_root, entry).await?;
    let registry = set_active_codex_account(data_root, Some(account_id.clone())).await?;
    Ok(CodexAuthImportOutcome {
        registry,
        account_id,
        created: true,
    })
}

pub(super) async fn load_codex_auth_from_secret_store(
    data_root: &Path,
    secret_ref: &str,
) -> Result<serde_json::Value> {
    let path = codex_secret_path(data_root, secret_ref)?;
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("missing codex secret at {}", path.display()))?;
    let envelope: CodexSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex secret JSON at {}", path.display()))?;
    if envelope.version != CODEX_SECRET_VERSION {
        anyhow::bail!(
            "unsupported codex secret version {} at {}",
            envelope.version,
            path.display()
        );
    }
    if !codex_auth_has_supported_shape(&envelope.auth) {
        anyhow::bail!(
            "codex secret at {} has unsupported auth shape",
            path.display()
        );
    }
    Ok(envelope.auth)
}

pub(super) async fn project_auth_value_to_home(
    home: &Path,
    auth: &serde_json::Value,
) -> Result<bool> {
    let payload = serde_json::to_vec_pretty(auth)?;
    tokio::fs::create_dir_all(home).await?;
    let dest = home.join("auth.json");
    let write = match tokio::fs::read(&dest).await {
        Ok(existing) => existing != payload,
        Err(_) => true,
    };
    if write {
        write_secure_file_atomic(&dest, &payload).await?;
    }
    Ok(write)
}

pub async fn import_host_codex_auth_to_secret_store(
    data_root: &Path,
    label: Option<String>,
) -> Result<CodexAccountRegistry> {
    let auth_path = host_codex_auth_path()?;
    let payload = tokio::fs::read_to_string(&auth_path)
        .await
        .with_context(|| format!("missing host codex auth at {}", auth_path.display()))?;
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", auth_path.display()))?;
    import_codex_auth_value_to_secret_store(data_root, label, &auth)
        .await
        .map(|outcome| outcome.registry)
}

pub async fn hydrate_codex_account_home_from_secret(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let registry = load_codex_registry(data_root).await?;
    let Some(account) = registry.accounts.iter().find(|a| a.id == account_id) else {
        return Ok(false);
    };
    let Some(secret_ref) = account.secret_ref.as_deref() else {
        return Ok(false);
    };
    let auth = load_codex_auth_from_secret_store(data_root, secret_ref).await?;
    project_auth_value_to_home(&codex_account_dir(data_root, account_id), &auth).await
}

pub async fn ingest_codex_account_auth_to_secret_store(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let auth_path = codex_account_dir(data_root, account_id).join("auth.json");
    let payload = match tokio::fs::read_to_string(&auth_path).await {
        Ok(payload) => payload,
        Err(_) => return Ok(false),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", auth_path.display()))?;
    ingest_auth_value_for_account(data_root, account_id, &auth).await
}
