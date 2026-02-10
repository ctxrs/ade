use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const CTX_SEED_CODEX_AUTH_FROM_HOST_ENV: &str = "CTX_SEED_CODEX_AUTH_FROM_HOST";
const CTX_CODEX_HOST_AUTH_PATH_ENV: &str = "CTX_CODEX_HOST_AUTH_PATH";
const CODEX_SECRET_VERSION: u32 = 1;
const CODEX_RUNTIME_OWNER_FILE: &str = ".ctx-active-account-id";
pub const CODEX_CREDENTIAL_KIND_OAUTH: &str = "oauth";
pub const CODEX_CREDENTIAL_KIND_API_KEY: &str = "api_key";
pub const CODEX_API_SHAPE_OPENAI_RESPONSES: &str = "openai_responses";
pub const CODEX_AUTH_TYPE_BEARER: &str = "bearer";
pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexSecretEnvelope {
    version: u32,
    auth: serde_json::Value,
}

fn default_codex_credential_kind() -> String {
    CODEX_CREDENTIAL_KIND_OAUTH.to_string()
}

fn default_codex_api_shape() -> String {
    CODEX_API_SHAPE_OPENAI_RESPONSES.to_string()
}

fn default_codex_auth_type() -> String {
    CODEX_AUTH_TYPE_BEARER.to_string()
}

fn default_codex_base_url() -> Option<String> {
    Some(CODEX_DEFAULT_BASE_URL.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexEndpointProfile {
    #[serde(default = "default_codex_api_shape")]
    pub api_shape: String,
    #[serde(default = "default_codex_auth_type")]
    pub auth_type: String,
    #[serde(
        default = "default_codex_base_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_url: Option<String>,
}

impl Default for CodexEndpointProfile {
    fn default() -> Self {
        Self {
            api_shape: default_codex_api_shape(),
            auth_type: default_codex_auth_type(),
            base_url: default_codex_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_codex_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
    #[serde(default)]
    pub endpoint_profile: CodexEndpointProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<CodexAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexLoginStatus {
    pub account_id: String,
    pub auth_url: String,
    #[serde(default)]
    pub expected_callback_url: Option<String>,
    #[serde(default)]
    pub completion_token: Option<String>,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexHostImportProbe {
    pub available: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub auth_kind: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn codex_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("codex").join("accounts")
}

pub fn codex_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("codex")
}

fn codex_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    codex_secrets_root(data_root).join(secret_ref)
}

pub fn codex_runtime_home(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("codex").join("home")
}

fn codex_runtime_owner_path(data_root: &Path) -> PathBuf {
    codex_runtime_home(data_root).join(CODEX_RUNTIME_OWNER_FILE)
}

pub fn codex_registry_path(data_root: &Path) -> PathBuf {
    codex_accounts_root(data_root).join("index.json")
}

pub fn codex_account_dir(data_root: &Path, account_id: &str) -> PathBuf {
    codex_accounts_root(data_root).join(account_id)
}

pub async fn load_codex_registry(data_root: &Path) -> CodexAccountRegistry {
    let path = codex_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => CodexAccountRegistry::default(),
    }
}

pub async fn save_codex_registry(data_root: &Path, registry: &CodexAccountRegistry) -> Result<()> {
    let path = codex_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn upsert_codex_account(
    data_root: &Path,
    entry: CodexAccountEntry,
) -> Result<CodexAccountRegistry> {
    let mut registry = load_codex_registry(data_root).await;
    let mut normalized = entry;
    let account_id = normalized.id.clone();
    if normalized.kind.trim().is_empty() {
        normalized.kind = default_codex_credential_kind();
    }
    normalize_endpoint_profile(&mut normalized.endpoint_profile);
    ensure_codex_endpoint_profile_compatible(&normalized.endpoint_profile)?;
    if let Some(existing) = registry.accounts.iter_mut().find(|a| a.id == account_id) {
        let prev_secret_ref = existing.secret_ref.clone();
        *existing = normalized;
        if existing.secret_ref.is_none() {
            existing.secret_ref = prev_secret_ref;
        }
    } else {
        registry.accounts.push(normalized);
    }
    save_codex_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_codex_account(
    data_root: &Path,
    account_id: &str,
) -> Result<CodexAccountRegistry> {
    let mut registry = load_codex_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<CodexAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_codex_registry(data_root, &registry).await?;
    if was_active {
        clear_runtime_auth_projection(data_root).await?;
    }
    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = codex_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }
    let account_dir = codex_account_dir(data_root, account_id);
    if account_dir.exists() {
        tokio::fs::remove_dir_all(account_dir).await?;
    }
    Ok(registry)
}

pub async fn set_active_codex_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<CodexAccountRegistry> {
    let mut registry = load_codex_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            anyhow::bail!("unknown account");
        };
        ensure_codex_endpoint_profile_compatible(&entry.endpoint_profile)?;
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_codex_registry(data_root, &registry).await?;
    if registry.active_account_id.is_none() {
        clear_runtime_auth_projection(data_root).await?;
    }
    Ok(registry)
}

pub async fn ensure_codex_account_dir(data_root: &Path, account_id: &str) -> Result<PathBuf> {
    let dir = codex_account_dir(data_root, account_id);
    tokio::fs::create_dir_all(&dir).await?;
    Ok(dir)
}

pub fn codex_env_for_account(data_root: &Path, account_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    let dir = codex_account_dir(data_root, account_id);
    env.insert("CODEX_HOME".to_string(), dir.to_string_lossy().to_string());
    env
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

async fn write_secure_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("missing parent dir for {}", path.display()))?;
    tokio::fs::create_dir_all(parent).await?;
    let tmp = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = tokio::fs::set_permissions(&tmp, perms).await;
    }
    tokio::fs::rename(&tmp, path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = tokio::fs::set_permissions(path, perms).await;
    }
    Ok(())
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

fn normalize_endpoint_profile(profile: &mut CodexEndpointProfile) {
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
    let secret_path = codex_secret_path(data_root, &secret_ref);
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

async fn write_runtime_owner_marker(data_root: &Path, account_id: &str) -> Result<()> {
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

async fn clear_runtime_auth_projection(data_root: &Path) -> Result<()> {
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
    let path = codex_secret_path(data_root, secret_ref);
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

pub async fn ingest_runtime_home_auth_to_active_secret(
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
                if project_secret_to_runtime_home(data_root, active, secret_ref)
                    .await
                    .is_ok()
                {
                    return codex_env_for_runtime_home(data_root).await;
                }
            }
        } else {
            clear_runtime_auth_projection(data_root).await?;
            return codex_env_for_runtime_home(data_root).await;
        }
        let mirrored = mirror_account_auth_to_runtime_home(data_root, active).await?;
        if !mirrored {
            clear_runtime_auth_projection(data_root).await?;
        }
        return codex_env_for_runtime_home(data_root).await;
    }

    clear_runtime_auth_projection(data_root).await?;
    codex_env_for_runtime_home(data_root).await
}

pub fn normalize_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Codex Account {account_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }

        fn without(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prev.as_deref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn codex_env_mirrors_active_account_auth_into_runtime_home() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = CodexAccountRegistry {
            active_account_id: Some("acct-123".to_string()),
            accounts: vec![CodexAccountEntry {
                id: "acct-123".to_string(),
                label: "Account".to_string(),
                kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        let account_dir = ensure_codex_account_dir(root, "acct-123").await.unwrap();
        tokio::fs::write(
            account_dir.join("auth.json"),
            br#"{"OPENAI_API_KEY":"test-key"}"#,
        )
        .await
        .unwrap();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
        assert!(codex_runtime_home(root).exists());
        let mirrored = tokio::fs::read_to_string(codex_runtime_home(root).join("auth.json"))
            .await
            .unwrap();
        assert!(mirrored.contains("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn codex_env_defaults_to_runtime_home() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
        assert!(codex_runtime_home(root).exists());
    }

    #[tokio::test]
    async fn codex_env_clears_stale_runtime_auth_when_no_active_account() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        tokio::fs::create_dir_all(codex_runtime_home(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_runtime_home(root).join("auth.json"),
            br#"{"OPENAI_API_KEY":"stale-key"}"#,
        )
        .await
        .unwrap();
        write_runtime_owner_marker(root, "acct-stale")
            .await
            .unwrap();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
        assert!(!codex_runtime_home(root).join("auth.json").exists());
        assert!(!codex_runtime_owner_path(root).exists());
    }

    #[tokio::test]
    async fn codex_auth_preflight_accepts_api_key_shape() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        tokio::fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"test-key"}"#)
            .await
            .unwrap();
        ensure_codex_auth_ready(home).await.unwrap();
    }

    #[tokio::test]
    async fn codex_auth_preflight_rejects_missing_supported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        tokio::fs::write(
            home.join("auth.json"),
            br#"{"tokens":{"access_token":"a"}}"#,
        )
        .await
        .unwrap();
        let err = ensure_codex_auth_ready(home).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn ingested_secret_projects_even_without_account_dir_auth() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let account_id = "acct-123";
        let registry = CodexAccountRegistry {
            active_account_id: Some(account_id.to_string()),
            accounts: vec![CodexAccountEntry {
                id: account_id.to_string(),
                label: "acct".to_string(),
                kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        let account_dir = ensure_codex_account_dir(root, account_id).await.unwrap();
        tokio::fs::write(
            account_dir.join("auth.json"),
            br#"{"OPENAI_API_KEY":"test-key"}"#,
        )
        .await
        .unwrap();

        ingest_codex_account_auth_to_secret_store(root, account_id)
            .await
            .unwrap();
        tokio::fs::remove_file(account_dir.join("auth.json"))
            .await
            .unwrap();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
        ensure_codex_auth_ready(Path::new(home)).await.unwrap();
    }

    #[tokio::test]
    async fn runtime_home_refresh_reconciles_back_to_active_secret() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let account_id = "acct-123";
        let secret_ref = format!("{account_id}.json");
        let registry = CodexAccountRegistry {
            active_account_id: Some(account_id.to_string()),
            accounts: vec![CodexAccountEntry {
                id: account_id.to_string(),
                label: "acct".to_string(),
                kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: Some(secret_ref.clone()),
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        tokio::fs::create_dir_all(codex_secrets_root(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_secret_path(root, &secret_ref),
            br#"{"version":1,"auth":{"tokens":{"access_token":"old-access","refresh_token":"old-refresh"}}}"#,
        )
        .await
        .unwrap();
        tokio::fs::create_dir_all(codex_runtime_home(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_runtime_home(root).join("auth.json"),
            br#"{"tokens":{"access_token":"new-access","refresh_token":"new-refresh"}}"#,
        )
        .await
        .unwrap();
        write_runtime_owner_marker(root, account_id).await.unwrap();

        let _ = codex_env_for_active_account(root).await.unwrap();

        let secret_payload = tokio::fs::read_to_string(codex_secret_path(root, &secret_ref))
            .await
            .unwrap();
        assert!(secret_payload.contains("new-access"));
        assert!(secret_payload.contains("new-refresh"));
    }

    #[tokio::test]
    async fn removing_account_cleans_secret_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let account_id = "acct-123";
        let secret_ref = format!("{account_id}.json");
        let registry = CodexAccountRegistry {
            active_account_id: Some(account_id.to_string()),
            accounts: vec![CodexAccountEntry {
                id: account_id.to_string(),
                label: "acct".to_string(),
                kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: Some(secret_ref.clone()),
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        tokio::fs::create_dir_all(codex_secrets_root(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_secret_path(root, &secret_ref),
            br#"{"version":1,"auth":{"OPENAI_API_KEY":"test-key"}}"#,
        )
        .await
        .unwrap();

        remove_codex_account(root, account_id).await.unwrap();
        assert!(!codex_secret_path(root, &secret_ref).exists());
    }

    #[tokio::test]
    async fn probe_host_auth_candidate_reports_api_key_shape() {
        let _env_lock = lock_env().await;
        let auth_dir = tempfile::tempdir().unwrap();
        let auth_path = auth_dir.path().join("auth.json");
        tokio::fs::write(&auth_path, br#"{"OPENAI_API_KEY":"test-key"}"#)
            .await
            .unwrap();
        let _path_guard = EnvGuard::set(
            CTX_CODEX_HOST_AUTH_PATH_ENV,
            auth_path.to_string_lossy().as_ref(),
        );

        let probe = probe_host_codex_auth_candidate().await;
        assert!(probe.available);
        assert_eq!(
            probe.auth_kind.as_deref(),
            Some(CODEX_CREDENTIAL_KIND_API_KEY)
        );
        assert_eq!(
            probe.path.as_deref(),
            Some(auth_path.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn import_host_auth_persists_secret_and_sets_active() {
        let _env_lock = lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let host_dir = tempfile::tempdir().unwrap();
        let auth_path = host_dir.path().join("auth.json");
        tokio::fs::write(
            &auth_path,
            br#"{"tokens":{"access_token":"a","refresh_token":"b"}}"#,
        )
        .await
        .unwrap();
        let _path_guard = EnvGuard::set(
            CTX_CODEX_HOST_AUTH_PATH_ENV,
            auth_path.to_string_lossy().as_ref(),
        );

        let registry = import_host_codex_auth_to_secret_store(root, Some("Imported".to_string()))
            .await
            .unwrap();
        let active = registry.active_account_id.clone().expect("active account");
        let entry = registry
            .accounts
            .iter()
            .find(|account| account.id == active)
            .expect("imported account");
        assert_eq!(entry.label, "Imported");
        assert_eq!(entry.kind, CODEX_CREDENTIAL_KIND_OAUTH);
        assert!(entry.secret_ref.is_some());
        assert_eq!(
            entry.endpoint_profile.api_shape,
            CODEX_API_SHAPE_OPENAI_RESPONSES
        );
        assert_eq!(entry.endpoint_profile.auth_type, CODEX_AUTH_TYPE_BEARER);

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        ensure_codex_auth_ready(Path::new(home)).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_rejects_incompatible_endpoint_profile() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let entry = CodexAccountEntry {
            id: "acct-incompatible".to_string(),
            label: "Bad Profile".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile {
                api_shape: "anthropic_messages".to_string(),
                auth_type: CODEX_AUTH_TYPE_BEARER.to_string(),
                base_url: Some("https://example.com/v1".to_string()),
            },
        };

        let err = upsert_codex_account(root, entry).await.unwrap_err();
        assert!(err.to_string().contains("api_shape=openai_responses"));
    }

    #[tokio::test]
    async fn set_active_rejects_incompatible_auth_type() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = CodexAccountRegistry {
            active_account_id: None,
            accounts: vec![CodexAccountEntry {
                id: "acct-bad".to_string(),
                label: "Bad Profile".to_string(),
                kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: CodexEndpointProfile {
                    api_shape: CODEX_API_SHAPE_OPENAI_RESPONSES.to_string(),
                    auth_type: "basic".to_string(),
                    base_url: Some("https://example.com/v1".to_string()),
                },
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();

        let err = set_active_codex_account(root, Some("acct-bad".to_string()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("auth_type=bearer"));
    }

    #[tokio::test]
    async fn clearing_active_account_clears_runtime_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = CodexAccountRegistry {
            active_account_id: Some("acct-1".to_string()),
            accounts: vec![CodexAccountEntry {
                id: "acct-1".to_string(),
                label: "Account".to_string(),
                kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        tokio::fs::create_dir_all(codex_runtime_home(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_runtime_home(root).join("auth.json"),
            br#"{"OPENAI_API_KEY":"stale"}"#,
        )
        .await
        .unwrap();
        write_runtime_owner_marker(root, "acct-1").await.unwrap();

        let _ = set_active_codex_account(root, None).await.unwrap();
        assert!(!codex_runtime_home(root).join("auth.json").exists());
        assert!(!codex_runtime_owner_path(root).exists());
    }

    #[tokio::test]
    async fn removing_active_account_clears_runtime_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = CodexAccountRegistry {
            active_account_id: Some("acct-remove".to_string()),
            accounts: vec![CodexAccountEntry {
                id: "acct-remove".to_string(),
                label: "Account".to_string(),
                kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                email: None,
                plan_type: None,
                created_at: Utc::now(),
                last_used_at: None,
                secret_ref: None,
                endpoint_profile: CodexEndpointProfile::default(),
            }],
        };
        save_codex_registry(root, &registry).await.unwrap();
        tokio::fs::create_dir_all(codex_runtime_home(root))
            .await
            .unwrap();
        tokio::fs::write(
            codex_runtime_home(root).join("auth.json"),
            br#"{"OPENAI_API_KEY":"stale"}"#,
        )
        .await
        .unwrap();
        write_runtime_owner_marker(root, "acct-remove")
            .await
            .unwrap();

        let _ = remove_codex_account(root, "acct-remove").await.unwrap();
        assert!(!codex_runtime_home(root).join("auth.json").exists());
        assert!(!codex_runtime_owner_path(root).exists());
    }
}
