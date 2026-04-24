use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::shared::write_secure_file_atomic;
use super::{
    CODEX_AUTH_TYPE_BEARER, CODEX_CREDENTIAL_KIND_API_KEY, CODEX_CREDENTIAL_KIND_OAUTH,
    CODEX_SECRET_VERSION, CTX_CODEX_HOST_AUTH_PATH_ENV, CTX_SEED_CODEX_AUTH_FROM_HOST_ENV,
    CodexAccountEntry, CodexAccountRegistry, CodexEndpointProfile, CodexHostImportProbe,
    codex_account_dir, codex_runtime_home, codex_runtime_owner_path, codex_secret_path,
    default_codex_api_shape, default_codex_auth_type, default_codex_credential_kind,
    load_codex_registry, normalize_label, save_codex_registry, set_active_codex_account,
    upsert_codex_account,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexSecretEnvelope {
    version: u32,
    auth: serde_json::Value,
}

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

pub fn host_codex_auth_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var(CTX_CODEX_HOST_AUTH_PATH_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    let base = directories::BaseDirs::new().ok_or_else(|| anyhow!("missing home dir"))?;
    Ok(base.home_dir().join(".codex").join("auth.json"))
}

pub fn seeding_codex_auth_from_host_enabled() -> bool {
    matches!(
        std::env::var(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV)
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

pub async fn seed_codex_auth_from_host(codex_home: &Path) -> Result<bool> {
    if !seeding_codex_auth_from_host_enabled() {
        return Ok(false);
    }
    let src = host_codex_auth_path()?;
    if !src.exists() {
        anyhow::bail!(
            "Codex auth seeding is enabled ({CTX_SEED_CODEX_AUTH_FROM_HOST_ENV}=1) but host auth file is missing at {}",
            src.display()
        );
    }
    let bytes = tokio::fs::read(&src).await?;
    if bytes.is_empty() {
        anyhow::bail!(
            "Codex auth seeding is enabled ({CTX_SEED_CODEX_AUTH_FROM_HOST_ENV}=1) but host auth file is empty at {}",
            src.display()
        );
    }
    tokio::fs::create_dir_all(codex_home).await?;
    let dest = codex_home.join("auth.json");
    let write = match tokio::fs::read(&dest).await {
        Ok(existing) => existing != bytes,
        Err(_) => true,
    };
    if write {
        tokio::fs::write(&dest, &bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = tokio::fs::set_permissions(&dest, perms).await;
        }
    }
    Ok(write)
}

async fn mirror_host_codex_auth_to_runtime_root(runtime_root: &Path) -> Result<bool> {
    let src = host_codex_auth_path()?;
    let payload = match tokio::fs::read_to_string(&src).await {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", src.display()))?;
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    let owner_path = codex_runtime_owner_path(runtime_root);
    match tokio::fs::remove_file(&owner_path).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(projected)
}

fn codex_auth_has_supported_shape(value: &serde_json::Value) -> bool {
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

fn codex_auth_kind(value: &serde_json::Value) -> Option<String> {
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

pub(crate) fn normalize_endpoint_profile(profile: &mut CodexEndpointProfile) {
    profile.api_shape = profile
        .api_shape
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    if profile.api_shape.is_empty() {
        profile.api_shape = default_codex_api_shape();
    }
    profile.auth_type = profile.auth_type.trim().to_ascii_lowercase();
    if profile.auth_type.is_empty() {
        profile.auth_type = default_codex_auth_type();
    }
    if let Some(url) = profile.base_url.as_ref() {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            profile.base_url = None;
        } else {
            profile.base_url = Some(trimmed.to_string());
        }
    }
}

pub fn ensure_codex_endpoint_profile_compatible(profile: &CodexEndpointProfile) -> Result<()> {
    let shape = profile
        .api_shape
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    if !matches!(shape.as_str(), "openai_responses" | "responses") {
        anyhow::bail!(
            "codex requires endpoint api_shape=openai_responses; found {}",
            profile.api_shape
        );
    }
    let auth = profile.auth_type.trim().to_ascii_lowercase();
    if auth != CODEX_AUTH_TYPE_BEARER {
        anyhow::bail!(
            "codex requires endpoint auth_type=bearer; found {}",
            profile.auth_type
        );
    }
    Ok(())
}

pub async fn probe_host_codex_auth_candidate() -> CodexHostImportProbe {
    let path = match host_codex_auth_path() {
        Ok(path) => path,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: None,
                auth_kind: None,
                error: Some(err.to_string()),
            };
        }
    };
    if !path.exists() {
        return CodexHostImportProbe {
            available: false,
            path: Some(path.display().to_string()),
            auth_kind: None,
            error: None,
        };
    }
    let payload = match tokio::fs::read_to_string(&path).await {
        Ok(payload) => payload,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: Some(path.display().to_string()),
                auth_kind: None,
                error: Some(err.to_string()),
            };
        }
    };
    let auth: serde_json::Value = match serde_json::from_str(&payload) {
        Ok(auth) => auth,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: Some(path.display().to_string()),
                auth_kind: None,
                error: Some(format!("invalid JSON: {err}")),
            };
        }
    };
    let auth_kind = codex_auth_kind(&auth);
    if auth_kind.is_none() {
        return CodexHostImportProbe {
            available: false,
            path: Some(path.display().to_string()),
            auth_kind: None,
            error: Some(
                "unsupported auth shape; expected OPENAI_API_KEY or tokens.access_token+tokens.refresh_token"
                    .to_string(),
            ),
        };
    }
    CodexHostImportProbe {
        available: true,
        path: Some(path.display().to_string()),
        auth_kind,
        error: None,
    }
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
    let mut registry = load_codex_registry(data_root).await;
    if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == account_id) {
        entry.secret_ref = Some(secret_ref);
        entry.kind = kind.unwrap_or_else(default_codex_credential_kind);
        save_codex_registry(data_root, &registry).await?;
    }
    Ok(())
}

async fn ingest_auth_value_for_account(
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
    let kind = codex_auth_kind(&auth).ok_or_else(|| {
        anyhow!(
            "codex auth file at {} has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token",
            auth_path.display()
        )
    })?;

    let registry = load_codex_registry(data_root).await;
    for existing in &registry.accounts {
        // Secret-backed accounts may not have a materialized auth.json in account dirs.
        // Hydrate first so host imports dedupe across both storage modes.
        let _ = hydrate_codex_account_home_from_secret(data_root, &existing.id).await;
        let existing_auth_path = codex_account_dir(data_root, &existing.id).join("auth.json");
        if let Ok(existing_payload) = tokio::fs::read_to_string(&existing_auth_path).await {
            if let Ok(existing_auth) = serde_json::from_str::<serde_json::Value>(&existing_payload)
            {
                if existing_auth == auth {
                    return set_active_codex_account(data_root, Some(existing.id.clone())).await;
                }
            }
        }
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_codex_secret_for_account(data_root, &account_id, &auth).await?;
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
    set_active_codex_account(data_root, Some(account_id)).await
}

async fn load_codex_auth_from_secret_store(
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

async fn project_auth_value_to_home(home: &Path, auth: &serde_json::Value) -> Result<bool> {
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

pub async fn hydrate_codex_account_home_from_secret(
    data_root: &Path,
    account_id: &str,
) -> Result<bool> {
    let registry = load_codex_registry(data_root).await;
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
