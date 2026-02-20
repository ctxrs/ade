use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const CTX_SEED_CODEX_AUTH_FROM_HOST_ENV: &str = "CTX_SEED_CODEX_AUTH_FROM_HOST";
const CTX_CODEX_HOST_AUTH_PATH_ENV: &str = "CTX_CODEX_HOST_AUTH_PATH";
const CODEX_SECRET_VERSION: u32 = 1;
const CLAUDE_SECRET_VERSION: u32 = 1;
const GEMINI_SECRET_VERSION: u32 = 1;
const KIMI_SECRET_VERSION: u32 = 1;
const COPILOT_SECRET_VERSION: u32 = 1;
const KIRO_SECRET_VERSION: u32 = 1;
const CURSOR_SECRET_VERSION: u32 = 1;
const CODEX_RUNTIME_OWNER_FILE: &str = ".ctx-active-account-id";
pub const CODEX_CREDENTIAL_KIND_OAUTH: &str = "oauth";
pub const CODEX_CREDENTIAL_KIND_API_KEY: &str = "api_key";
pub const CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN: &str = "setup_token";
pub const GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL: &str = "oauth-personal";
pub const KIMI_CREDENTIAL_KIND_CREDENTIALS_JSON: &str = "credentials-json";
pub const COPILOT_CREDENTIAL_KIND_GH_TOKEN: &str = "gh-token";
pub const KIRO_CREDENTIAL_KIND_AUTH_TOKEN_JSON: &str = "auth-token-json";
pub const CURSOR_CREDENTIAL_KIND_API_KEY: &str = "api-key";
pub const GEMINI_AUTH_SELECTED_TYPE_OAUTH_PERSONAL: &str = "oauth-personal";
pub const GEMINI_FORCE_FILE_STORAGE_ENV: &str = "GEMINI_FORCE_FILE_STORAGE";
pub const KIMI_SHARE_DIR_ENV: &str = "KIMI_SHARE_DIR";
pub const CODEX_API_SHAPE_OPENAI_RESPONSES: &str = "openai_responses";
pub const CODEX_AUTH_TYPE_BEARER: &str = "bearer";
pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const KIRO_AUTH_TOKEN_RELATIVE_PATH: &str = ".aws/sso/cache/kiro-auth-token.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexSecretEnvelope {
    version: u32,
    auth: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeSecretEnvelope {
    version: u32,
    #[serde(alias = "anthropic_auth_token")]
    claude_code_oauth_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeminiSecretEnvelope {
    version: u32,
    oauth_creds: serde_json::Value,
    #[serde(default)]
    google_accounts: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KimiSecretEnvelope {
    version: u32,
    provider: String,
    credentials: serde_json::Value,
    #[serde(default)]
    config_toml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CopilotSecretEnvelope {
    version: u32,
    gh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroSecretEnvelope {
    version: u32,
    auth_token: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorSecretEnvelope {
    version: u32,
    api_key: String,
}

fn default_codex_credential_kind() -> String {
    CODEX_CREDENTIAL_KIND_OAUTH.to_string()
}

fn default_claude_credential_kind() -> String {
    CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN.to_string()
}

fn default_gemini_credential_kind() -> String {
    GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()
}

fn default_kimi_credential_kind() -> String {
    KIMI_CREDENTIAL_KIND_CREDENTIALS_JSON.to_string()
}

fn default_copilot_credential_kind() -> String {
    COPILOT_CREDENTIAL_KIND_GH_TOKEN.to_string()
}

fn default_kiro_credential_kind() -> String {
    KIRO_CREDENTIAL_KIND_AUTH_TOKEN_JSON.to_string()
}

fn default_cursor_credential_kind() -> String {
    CURSOR_CREDENTIAL_KIND_API_KEY.to_string()
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
pub struct ClaudeAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_claude_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<ClaudeAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_gemini_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<GeminiAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_kimi_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KimiAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<KimiAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_copilot_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CopilotAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<CopilotAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_kiro_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KiroAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<KiroAccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default = "default_cursor_credential_kind")]
    pub kind: String,
    #[serde(default)]
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorAccountRegistry {
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<CursorAccountEntry>,
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
pub struct ClaudeLoginStatus {
    pub login_id: String,
    #[serde(default)]
    pub auth_url: Option<String>,
    pub status: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiLoginStatus {
    pub login_id: String,
    #[serde(default)]
    pub auth_url: Option<String>,
    pub status: String,
    #[serde(default)]
    pub account_id: Option<String>,
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

pub fn claude_accounts_root(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("claude-crp")
        .join("accounts")
}

pub fn gemini_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("gemini").join("accounts")
}

pub fn kimi_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("kimi").join("accounts")
}

pub fn copilot_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("copilot").join("accounts")
}

pub fn kiro_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("kiro").join("accounts")
}

pub fn cursor_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("cursor").join("accounts")
}

pub fn codex_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("codex")
}

pub fn claude_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("claude-crp")
}

pub fn gemini_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("gemini")
}

pub fn kimi_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("kimi")
}

pub fn copilot_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("copilot")
}

pub fn kiro_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("kiro")
}

pub fn cursor_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("cursor")
}

fn codex_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    codex_secrets_root(data_root).join(secret_ref)
}

fn claude_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    claude_secrets_root(data_root).join(secret_ref)
}

fn gemini_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    gemini_secrets_root(data_root).join(secret_ref)
}

fn kimi_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    kimi_secrets_root(data_root).join(secret_ref)
}

fn copilot_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    copilot_secrets_root(data_root).join(secret_ref)
}

fn kiro_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    kiro_secrets_root(data_root).join(secret_ref)
}

fn cursor_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    cursor_secrets_root(data_root).join(secret_ref)
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

pub fn claude_registry_path(data_root: &Path) -> PathBuf {
    claude_accounts_root(data_root).join("index.json")
}

pub fn gemini_registry_path(data_root: &Path) -> PathBuf {
    gemini_accounts_root(data_root).join("index.json")
}

pub fn kimi_registry_path(data_root: &Path) -> PathBuf {
    kimi_accounts_root(data_root).join("index.json")
}

pub fn copilot_registry_path(data_root: &Path) -> PathBuf {
    copilot_accounts_root(data_root).join("index.json")
}

pub fn kiro_registry_path(data_root: &Path) -> PathBuf {
    kiro_accounts_root(data_root).join("index.json")
}

pub fn cursor_registry_path(data_root: &Path) -> PathBuf {
    cursor_accounts_root(data_root).join("index.json")
}

pub fn codex_account_dir(data_root: &Path, account_id: &str) -> PathBuf {
    codex_accounts_root(data_root).join(account_id)
}

pub fn claude_account_dir(data_root: &Path, account_id: &str) -> PathBuf {
    claude_accounts_root(data_root).join(account_id)
}

pub fn gemini_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    gemini_accounts_root(data_root).join(account_id)
}

pub fn kimi_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    kimi_accounts_root(data_root).join(account_id)
}

pub fn copilot_account_dir(data_root: &Path, account_id: &str) -> PathBuf {
    copilot_accounts_root(data_root).join(account_id)
}

pub fn kiro_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    kiro_accounts_root(data_root).join(account_id)
}

pub fn cursor_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    cursor_accounts_root(data_root).join(account_id)
}

fn ensure_safe_account_id(account_id: &str) -> Result<()> {
    if account_id.trim().is_empty() {
        bail!("account_id is required");
    }

    let mut components = Path::new(account_id).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => bail!("account_id must be a single path segment"),
    }
}

fn container_workspaces_root(data_root: &Path) -> PathBuf {
    data_root.join("containers").join("workspaces")
}

async fn container_runtime_data_roots(data_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut entries = match tokio::fs::read_dir(container_workspaces_root(data_root)).await {
        Ok(entries) => entries,
        Err(_) => return roots,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let runtime_root = entry.path().join("data");
        match tokio::fs::metadata(&runtime_root).await {
            Ok(metadata) if metadata.is_dir() => roots.push(runtime_root),
            _ => {}
        }
    }

    roots
}

async fn remove_projected_account_home_for_runtime_roots(
    data_root: &Path,
    account_id: &str,
    account_home_for_root: fn(&Path, &str) -> PathBuf,
    provider_id: &str,
) -> Result<()> {
    for runtime_root in container_runtime_data_roots(data_root).await {
        let projected_home = account_home_for_root(&runtime_root, account_id);
        match tokio::fs::remove_dir_all(&projected_home).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "removing projected {provider_id} account home {}",
                        projected_home.display()
                    )
                });
            }
        }
    }

    Ok(())
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

pub async fn load_claude_registry(data_root: &Path) -> ClaudeAccountRegistry {
    let path = claude_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => ClaudeAccountRegistry::default(),
    }
}

pub async fn save_claude_registry(
    data_root: &Path,
    registry: &ClaudeAccountRegistry,
) -> Result<()> {
    let path = claude_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn load_gemini_registry(data_root: &Path) -> GeminiAccountRegistry {
    let path = gemini_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => GeminiAccountRegistry::default(),
    }
}

pub async fn save_gemini_registry(
    data_root: &Path,
    registry: &GeminiAccountRegistry,
) -> Result<()> {
    let path = gemini_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn load_kimi_registry(data_root: &Path) -> KimiAccountRegistry {
    let path = kimi_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => KimiAccountRegistry::default(),
    }
}

pub async fn save_kimi_registry(data_root: &Path, registry: &KimiAccountRegistry) -> Result<()> {
    let path = kimi_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn load_copilot_registry(data_root: &Path) -> CopilotAccountRegistry {
    let path = copilot_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => CopilotAccountRegistry::default(),
    }
}

pub async fn save_copilot_registry(
    data_root: &Path,
    registry: &CopilotAccountRegistry,
) -> Result<()> {
    let path = copilot_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn load_kiro_registry(data_root: &Path) -> KiroAccountRegistry {
    let path = kiro_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => KiroAccountRegistry::default(),
    }
}

pub async fn save_kiro_registry(data_root: &Path, registry: &KiroAccountRegistry) -> Result<()> {
    let path = kiro_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub async fn load_cursor_registry(data_root: &Path) -> CursorAccountRegistry {
    let path = cursor_registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => CursorAccountRegistry::default(),
    }
}

pub async fn save_cursor_registry(
    data_root: &Path,
    registry: &CursorAccountRegistry,
) -> Result<()> {
    let path = cursor_registry_path(data_root);
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
    ensure_safe_account_id(account_id)?;
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

pub async fn ensure_claude_account_dir(data_root: &Path, account_id: &str) -> Result<PathBuf> {
    let dir = claude_account_dir(data_root, account_id);
    tokio::fs::create_dir_all(&dir).await?;
    Ok(dir)
}

fn normalize_claude_setup_token(token: &str) -> Result<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        bail!("setup_token is required");
    }
    let unquoted = if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    };
    let collapsed: String = unquoted.chars().filter(|ch| !ch.is_whitespace()).collect();
    if collapsed.is_empty() {
        bail!("setup_token is required");
    }
    if collapsed.contains('#') {
        bail!(
            "setup_token appears to be a browser callback code; paste a long-lived CLAUDE_CODE_OAUTH_TOKEN starting with sk-ant-oat"
        );
    }
    if !collapsed.starts_with("sk-ant-oat") {
        bail!("setup_token must start with sk-ant-oat");
    }
    if !collapsed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("setup_token contains invalid characters");
    }
    Ok(collapsed)
}

async fn write_claude_secret_for_account(
    data_root: &Path,
    account_id: &str,
    setup_token: &str,
) -> Result<String> {
    let token = normalize_claude_setup_token(setup_token)?;
    let secret_ref = format!("{account_id}.json");
    let path = claude_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = ClaudeSecretEnvelope {
        version: CLAUDE_SECRET_VERSION,
        claude_code_oauth_token: token,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_claude_secret_for_ref(data_root: &Path, secret_ref: &str) -> Result<String> {
    let path = claude_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading claude secret {}", path.display()))?;
    let parsed: ClaudeSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid claude secret {}", path.display()))?;
    normalize_claude_setup_token(&parsed.claude_code_oauth_token)
}

pub async fn add_claude_account(
    data_root: &Path,
    label: Option<String>,
    setup_token: String,
) -> Result<ClaudeAccountRegistry> {
    let token = normalize_claude_setup_token(&setup_token)?;
    let mut registry = load_claude_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_token) = read_claude_secret_for_ref(data_root, secret_ref).await {
            if existing_token == token {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_claude_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_claude_secret_for_account(data_root, &account_id, &token).await?;
    let entry = ClaudeAccountEntry {
        id: account_id.clone(),
        label: normalize_claude_label(label, &account_id),
        kind: CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN.to_string(),
        email: None,
        subscription_type: None,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_claude_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_claude_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<ClaudeAccountRegistry> {
    let mut registry = load_claude_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_claude_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_claude_account(
    data_root: &Path,
    account_id: &str,
) -> Result<ClaudeAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_claude_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<ClaudeAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_claude_registry(data_root, &registry).await?;

    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = claude_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }

    let account_dir = claude_account_dir(data_root, account_id);
    if account_dir.exists() {
        tokio::fs::remove_dir_all(account_dir).await?;
    }
    remove_projected_account_home_for_runtime_roots(
        data_root,
        account_id,
        claude_account_dir,
        "claude-crp",
    )
    .await?;

    Ok(registry)
}

pub fn claude_env_for_account(
    data_root: &Path,
    account_id: &str,
    setup_token: &str,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
        setup_token.to_string(),
    );
    env.insert(
        "CLAUDE_CONFIG_DIR".to_string(),
        claude_account_dir(data_root, account_id)
            .to_string_lossy()
            .to_string(),
    );
    env
}

pub async fn claude_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_claude_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };

    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };

    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active claude account has no secret reference");
    };

    let token = read_claude_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_claude_account_dir(data_root, active).await?;
    Ok(claude_env_for_account(data_root, active, &token))
}

pub fn normalize_claude_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Claude Account {account_id}"))
}

fn parse_json_value(raw: &str, field: &str) -> Result<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("{field} is required");
    }
    serde_json::from_str(trimmed).with_context(|| format!("{field} must be valid JSON"))
}

fn parse_required_json_object(raw: &str, field: &str) -> Result<serde_json::Value> {
    let parsed = parse_json_value(raw, field)?;
    if !parsed.is_object() {
        bail!("{field} must be a JSON object");
    }
    Ok(parsed)
}

fn parse_optional_json_value(raw: Option<&str>, field: &str) -> Result<Option<serde_json::Value>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(parse_json_value(trimmed, field)?))
}

fn normalize_optional_email(email: Option<String>) -> Option<String> {
    email
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn apply_label_update(label: Option<String>, current: &mut String) {
    if let Some(raw) = label {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            *current = trimmed.to_string();
        }
    }
}

fn apply_email_update(email: Option<String>, current: &mut Option<String>) {
    if let Some(raw) = email {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            *current = None;
        } else {
            *current = Some(trimmed.to_string());
        }
    }
}

async fn write_gemini_secret_for_account(
    data_root: &Path,
    account_id: &str,
    oauth_creds_json: &str,
    google_accounts_json: Option<&str>,
) -> Result<String> {
    let oauth_creds = parse_required_json_object(oauth_creds_json, "oauth_creds_json")?;
    let google_accounts = parse_optional_json_value(google_accounts_json, "google_accounts_json")?;
    let secret_ref = format!("{account_id}.json");
    let path = gemini_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = GeminiSecretEnvelope {
        version: GEMINI_SECRET_VERSION,
        oauth_creds,
        google_accounts,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_gemini_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
) -> Result<GeminiSecretEnvelope> {
    let path = gemini_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading gemini secret {}", path.display()))?;
    let parsed: GeminiSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid gemini secret {}", path.display()))?;
    if parsed.version != GEMINI_SECRET_VERSION {
        bail!(
            "unsupported gemini secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    if !parsed.oauth_creds.is_object() {
        bail!("gemini oauth_creds must be a JSON object");
    }
    Ok(parsed)
}

async fn ensure_gemini_account_home(
    data_root: &Path,
    account_id: &str,
    secret: &GeminiSecretEnvelope,
) -> Result<PathBuf> {
    let home = gemini_account_home(data_root, account_id);
    let gemini_dir = home.join(".gemini");
    tokio::fs::create_dir_all(&gemini_dir).await?;
    write_secure_file_atomic(
        &gemini_dir.join("oauth_creds.json"),
        &serde_json::to_vec_pretty(&secret.oauth_creds)?,
    )
    .await?;
    if let Some(accounts) = secret.google_accounts.as_ref() {
        write_secure_file_atomic(
            &gemini_dir.join("google_accounts.json"),
            &serde_json::to_vec_pretty(accounts)?,
        )
        .await?;
    } else {
        let accounts_path = gemini_dir.join("google_accounts.json");
        match tokio::fs::remove_file(&accounts_path).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    let settings = serde_json::json!({
        "security": {
            "auth": {
                "selectedType": GEMINI_AUTH_SELECTED_TYPE_OAUTH_PERSONAL
            }
        }
    });
    write_secure_file_atomic(
        &gemini_dir.join("settings.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )
    .await?;
    Ok(home)
}

pub async fn add_gemini_account(
    data_root: &Path,
    label: Option<String>,
    oauth_creds_json: String,
    google_accounts_json: Option<String>,
    email: Option<String>,
) -> Result<GeminiAccountRegistry> {
    let oauth_creds = parse_required_json_object(&oauth_creds_json, "oauth_creds_json")?;
    let mut registry = load_gemini_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_secret) = read_gemini_secret_for_ref(data_root, secret_ref).await {
            if existing_secret.oauth_creds == oauth_creds {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(google_accounts) = google_accounts_json.as_deref() {
            let _ = write_gemini_secret_for_account(
                data_root,
                &account_id,
                &oauth_creds_json,
                Some(google_accounts),
            )
            .await?;
        }
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_gemini_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_gemini_secret_for_account(
        data_root,
        &account_id,
        &oauth_creds_json,
        google_accounts_json.as_deref(),
    )
    .await?;
    let entry = GeminiAccountEntry {
        id: account_id.clone(),
        label: normalize_gemini_label(label, &account_id),
        kind: GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string(),
        email: normalize_optional_email(email),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_gemini_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_gemini_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<GeminiAccountRegistry> {
    let mut registry = load_gemini_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_gemini_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_gemini_account(
    data_root: &Path,
    account_id: &str,
) -> Result<GeminiAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_gemini_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<GeminiAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_gemini_registry(data_root, &registry).await?;

    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = gemini_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }

    let account_home = gemini_account_home(data_root, account_id);
    if account_home.exists() {
        tokio::fs::remove_dir_all(account_home).await?;
    }
    remove_projected_account_home_for_runtime_roots(
        data_root,
        account_id,
        gemini_account_home,
        "gemini",
    )
    .await?;

    Ok(registry)
}

pub fn gemini_env_for_account(data_root: &Path, account_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "GEMINI_CLI_HOME".to_string(),
        gemini_account_home(data_root, account_id)
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        GEMINI_FORCE_FILE_STORAGE_ENV.to_string(),
        "true".to_string(),
    );
    env
}

pub async fn gemini_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_gemini_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };

    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };

    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active gemini account has no secret reference");
    };

    let secret = read_gemini_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_gemini_account_home(data_root, active, &secret).await?;
    Ok(gemini_env_for_account(data_root, active))
}

pub fn normalize_gemini_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Gemini Account {account_id}"))
}

fn normalize_kimi_provider(provider: Option<String>) -> Result<String> {
    let provider = provider
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| "moonshot".to_string());
    if provider
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Ok(provider);
    }
    bail!("provider must contain only [A-Za-z0-9_-]");
}

fn normalize_optional_multiline(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn write_kimi_secret_for_account(
    data_root: &Path,
    account_id: &str,
    provider: &str,
    credentials_json: &str,
    config_toml: Option<String>,
) -> Result<String> {
    let credentials = parse_required_json_object(credentials_json, "credentials_json")?;
    let provider = normalize_kimi_provider(Some(provider.to_string()))?;
    let secret_ref = format!("{account_id}.json");
    let path = kimi_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = KimiSecretEnvelope {
        version: KIMI_SECRET_VERSION,
        provider,
        credentials,
        config_toml: normalize_optional_multiline(config_toml),
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_kimi_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
) -> Result<KimiSecretEnvelope> {
    let path = kimi_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading kimi secret {}", path.display()))?;
    let parsed: KimiSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid kimi secret {}", path.display()))?;
    if parsed.version != KIMI_SECRET_VERSION {
        bail!(
            "unsupported kimi secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    if !parsed.credentials.is_object() {
        bail!("kimi credentials must be a JSON object");
    }
    let _ = normalize_kimi_provider(Some(parsed.provider.clone()))?;
    Ok(parsed)
}

async fn ensure_kimi_account_home(
    data_root: &Path,
    account_id: &str,
    secret: &KimiSecretEnvelope,
) -> Result<PathBuf> {
    let home = kimi_account_home(data_root, account_id);
    let share_dir = home.join(".kimi");
    let credentials_dir = share_dir.join("credentials");
    tokio::fs::create_dir_all(&credentials_dir).await?;
    let provider = normalize_kimi_provider(Some(secret.provider.clone()))?;
    let credentials_path = credentials_dir.join(format!("{provider}.json"));
    write_secure_file_atomic(
        &credentials_path,
        &serde_json::to_vec_pretty(&secret.credentials)?,
    )
    .await?;
    let config_toml = secret
        .config_toml
        .clone()
        .unwrap_or_else(|| format!("current_provider = \"{provider}\"\n"));
    write_secure_file_atomic(&share_dir.join("config.toml"), config_toml.as_bytes()).await?;
    Ok(share_dir)
}

pub async fn add_kimi_account(
    data_root: &Path,
    label: Option<String>,
    provider: Option<String>,
    credentials_json: String,
    config_toml: Option<String>,
    email: Option<String>,
) -> Result<KimiAccountRegistry> {
    let normalized_provider = normalize_kimi_provider(provider)?;
    let credentials = parse_required_json_object(&credentials_json, "credentials_json")?;
    let mut registry = load_kimi_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_secret) = read_kimi_secret_for_ref(data_root, secret_ref).await {
            if existing_secret.provider == normalized_provider
                && existing_secret.credentials == credentials
            {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(config_toml_value) = config_toml.clone() {
            let _ = write_kimi_secret_for_account(
                data_root,
                &account_id,
                &normalized_provider,
                &credentials_json,
                Some(config_toml_value),
            )
            .await?;
        }
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_kimi_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_kimi_secret_for_account(
        data_root,
        &account_id,
        &normalized_provider,
        &credentials_json,
        config_toml,
    )
    .await?;
    let entry = KimiAccountEntry {
        id: account_id.clone(),
        label: normalize_kimi_label(label, &account_id),
        kind: KIMI_CREDENTIAL_KIND_CREDENTIALS_JSON.to_string(),
        email: normalize_optional_email(email),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_kimi_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_kimi_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<KimiAccountRegistry> {
    let mut registry = load_kimi_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_kimi_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_kimi_account(
    data_root: &Path,
    account_id: &str,
) -> Result<KimiAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_kimi_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<KimiAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_kimi_registry(data_root, &registry).await?;

    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = kimi_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }

    let account_home = kimi_account_home(data_root, account_id);
    if account_home.exists() {
        tokio::fs::remove_dir_all(account_home).await?;
    }
    remove_projected_account_home_for_runtime_roots(
        data_root,
        account_id,
        kimi_account_home,
        "kimi",
    )
    .await?;
    Ok(registry)
}

pub fn kimi_env_for_account(data_root: &Path, account_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        KIMI_SHARE_DIR_ENV.to_string(),
        kimi_account_home(data_root, account_id)
            .join(".kimi")
            .to_string_lossy()
            .to_string(),
    );
    env
}

pub async fn kimi_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_kimi_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };
    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };
    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active kimi account has no secret reference");
    };
    let secret = read_kimi_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_kimi_account_home(data_root, active, &secret).await?;
    Ok(kimi_env_for_account(data_root, active))
}

pub fn normalize_kimi_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Kimi Account {account_id}"))
}

fn normalize_copilot_token(token: &str) -> Result<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        bail!("token is required");
    }
    Ok(trimmed.to_string())
}

async fn write_copilot_secret_for_account(
    data_root: &Path,
    account_id: &str,
    token: &str,
) -> Result<String> {
    let token = normalize_copilot_token(token)?;
    let secret_ref = format!("{account_id}.json");
    let path = copilot_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = CopilotSecretEnvelope {
        version: COPILOT_SECRET_VERSION,
        gh_token: token,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_copilot_secret_for_ref(data_root: &Path, secret_ref: &str) -> Result<String> {
    let path = copilot_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading copilot secret {}", path.display()))?;
    let parsed: CopilotSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid copilot secret {}", path.display()))?;
    if parsed.version != COPILOT_SECRET_VERSION {
        bail!(
            "unsupported copilot secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    normalize_copilot_token(&parsed.gh_token)
}

pub async fn ensure_copilot_account_dir(data_root: &Path, account_id: &str) -> Result<PathBuf> {
    let dir = copilot_account_dir(data_root, account_id);
    tokio::fs::create_dir_all(&dir).await?;
    Ok(dir)
}

pub async fn add_copilot_account(
    data_root: &Path,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CopilotAccountRegistry> {
    let token = normalize_copilot_token(&token)?;
    let mut registry = load_copilot_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_token) = read_copilot_secret_for_ref(data_root, secret_ref).await {
            if existing_token == token {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_copilot_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_copilot_secret_for_account(data_root, &account_id, &token).await?;
    let entry = CopilotAccountEntry {
        id: account_id.clone(),
        label: normalize_copilot_label(label, &account_id),
        kind: COPILOT_CREDENTIAL_KIND_GH_TOKEN.to_string(),
        email: normalize_optional_email(email),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_copilot_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_copilot_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<CopilotAccountRegistry> {
    let mut registry = load_copilot_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_copilot_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_copilot_account(
    data_root: &Path,
    account_id: &str,
) -> Result<CopilotAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_copilot_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<CopilotAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_copilot_registry(data_root, &registry).await?;
    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = copilot_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }
    let account_dir = copilot_account_dir(data_root, account_id);
    if account_dir.exists() {
        tokio::fs::remove_dir_all(account_dir).await?;
    }
    Ok(registry)
}

pub fn copilot_env_for_account(
    _data_root: &Path,
    _account_id: &str,
    token: &str,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("GH_TOKEN".to_string(), token.to_string());
    env.insert("GITHUB_TOKEN".to_string(), token.to_string());
    env
}

pub async fn copilot_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_copilot_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };
    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };
    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active copilot account has no secret reference");
    };
    let token = read_copilot_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_copilot_account_dir(data_root, active).await?;
    Ok(copilot_env_for_account(data_root, active, &token))
}

pub fn normalize_copilot_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Copilot Account {account_id}"))
}

async fn write_kiro_secret_for_account(
    data_root: &Path,
    account_id: &str,
    auth_token_json: &str,
) -> Result<String> {
    let auth_token = parse_required_json_object(auth_token_json, "auth_token_json")?;
    let secret_ref = format!("{account_id}.json");
    let path = kiro_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = KiroSecretEnvelope {
        version: KIRO_SECRET_VERSION,
        auth_token,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_kiro_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
) -> Result<KiroSecretEnvelope> {
    let path = kiro_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading kiro secret {}", path.display()))?;
    let parsed: KiroSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid kiro secret {}", path.display()))?;
    if parsed.version != KIRO_SECRET_VERSION {
        bail!(
            "unsupported kiro secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    if !parsed.auth_token.is_object() {
        bail!("kiro auth_token must be a JSON object");
    }
    Ok(parsed)
}

async fn ensure_kiro_account_home(
    data_root: &Path,
    account_id: &str,
    secret: &KiroSecretEnvelope,
) -> Result<PathBuf> {
    let home = kiro_account_home(data_root, account_id);
    tokio::fs::create_dir_all(&home).await?;
    let token_path = home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
    write_secure_file_atomic(&token_path, &serde_json::to_vec_pretty(&secret.auth_token)?).await?;
    tokio::fs::create_dir_all(home.join(".config")).await?;
    tokio::fs::create_dir_all(home.join(".cache")).await?;
    Ok(home)
}

pub async fn add_kiro_account(
    data_root: &Path,
    label: Option<String>,
    auth_token_json: String,
    email: Option<String>,
) -> Result<KiroAccountRegistry> {
    let auth_token = parse_required_json_object(&auth_token_json, "auth_token_json")?;
    let mut registry = load_kiro_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_secret) = read_kiro_secret_for_ref(data_root, secret_ref).await {
            if existing_secret.auth_token == auth_token {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_kiro_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref =
        write_kiro_secret_for_account(data_root, &account_id, &auth_token_json).await?;
    let entry = KiroAccountEntry {
        id: account_id.clone(),
        label: normalize_kiro_label(label, &account_id),
        kind: KIRO_CREDENTIAL_KIND_AUTH_TOKEN_JSON.to_string(),
        email: normalize_optional_email(email),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_kiro_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_kiro_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<KiroAccountRegistry> {
    let mut registry = load_kiro_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_kiro_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_kiro_account(
    data_root: &Path,
    account_id: &str,
) -> Result<KiroAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_kiro_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<KiroAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_kiro_registry(data_root, &registry).await?;

    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = kiro_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }

    let account_home = kiro_account_home(data_root, account_id);
    if account_home.exists() {
        tokio::fs::remove_dir_all(account_home).await?;
    }
    remove_projected_account_home_for_runtime_roots(
        data_root,
        account_id,
        kiro_account_home,
        "kiro",
    )
    .await?;
    Ok(registry)
}

pub fn kiro_env_for_account(data_root: &Path, account_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    let home = kiro_account_home(data_root, account_id);
    env.insert("HOME".to_string(), home.to_string_lossy().to_string());
    env.insert(
        "XDG_CONFIG_HOME".to_string(),
        home.join(".config").to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_CACHE_HOME".to_string(),
        home.join(".cache").to_string_lossy().to_string(),
    );
    env
}

pub async fn kiro_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_kiro_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };
    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };
    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active kiro account has no secret reference");
    };
    let secret = read_kiro_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_kiro_account_home(data_root, active, &secret).await?;
    Ok(kiro_env_for_account(data_root, active))
}

pub fn normalize_kiro_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Kiro Account {account_id}"))
}

fn normalize_cursor_token(token: &str) -> Result<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        bail!("token is required");
    }
    Ok(trimmed.to_string())
}

async fn write_cursor_secret_for_account(
    data_root: &Path,
    account_id: &str,
    token: &str,
) -> Result<String> {
    let token = normalize_cursor_token(token)?;
    let secret_ref = format!("{account_id}.json");
    let path = cursor_secret_path(data_root, &secret_ref);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let envelope = CursorSecretEnvelope {
        version: CURSOR_SECRET_VERSION,
        api_key: token,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_cursor_secret_for_ref(data_root: &Path, secret_ref: &str) -> Result<String> {
    let path = cursor_secret_path(data_root, secret_ref);
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading cursor secret {}", path.display()))?;
    let parsed: CursorSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid cursor secret {}", path.display()))?;
    if parsed.version != CURSOR_SECRET_VERSION {
        bail!(
            "unsupported cursor secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    normalize_cursor_token(&parsed.api_key)
}

pub async fn ensure_cursor_account_home(data_root: &Path, account_id: &str) -> Result<PathBuf> {
    let home = cursor_account_home(data_root, account_id);
    tokio::fs::create_dir_all(&home).await?;
    Ok(home)
}

pub async fn add_cursor_account(
    data_root: &Path,
    label: Option<String>,
    token: String,
    email: Option<String>,
) -> Result<CursorAccountRegistry> {
    let token = normalize_cursor_token(&token)?;
    let mut registry = load_cursor_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_token) = read_cursor_secret_for_ref(data_root, secret_ref).await {
            if existing_token == token {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_cursor_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_cursor_secret_for_account(data_root, &account_id, &token).await?;
    let entry = CursorAccountEntry {
        id: account_id.clone(),
        label: normalize_cursor_label(label, &account_id),
        kind: CURSOR_CREDENTIAL_KIND_API_KEY.to_string(),
        email: normalize_optional_email(email),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: Some(secret_ref),
    };
    registry.accounts.push(entry);
    registry.active_account_id = Some(account_id);
    save_cursor_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn set_active_cursor_account(
    data_root: &Path,
    account_id: Option<String>,
) -> Result<CursorAccountRegistry> {
    let mut registry = load_cursor_registry(data_root).await;
    if let Some(active_id) = account_id.as_deref() {
        let Some(entry) = registry.accounts.iter().find(|a| a.id == active_id) else {
            bail!("unknown account");
        };
        if entry.secret_ref.as_deref().is_none() {
            bail!("active account has no secret");
        }
    }
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_cursor_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_cursor_account(
    data_root: &Path,
    account_id: &str,
) -> Result<CursorAccountRegistry> {
    ensure_safe_account_id(account_id)?;
    let mut registry = load_cursor_registry(data_root).await;
    let was_active = registry.active_account_id.as_deref() == Some(account_id);
    let removed: Vec<CursorAccountEntry> = registry
        .accounts
        .iter()
        .filter(|a| a.id == account_id)
        .cloned()
        .collect();
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_cursor_registry(data_root, &registry).await?;

    for entry in removed {
        if let Some(secret_ref) = entry.secret_ref {
            let secret_path = cursor_secret_path(data_root, &secret_ref);
            if secret_path.exists() {
                let _ = tokio::fs::remove_file(secret_path).await;
            }
        }
    }

    let account_home = cursor_account_home(data_root, account_id);
    if account_home.exists() {
        tokio::fs::remove_dir_all(account_home).await?;
    }
    remove_projected_account_home_for_runtime_roots(
        data_root,
        account_id,
        cursor_account_home,
        "cursor",
    )
    .await?;
    Ok(registry)
}

pub fn cursor_env_for_account(
    data_root: &Path,
    account_id: &str,
    token: &str,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CURSOR_CONFIG_DIR".to_string(),
        cursor_account_home(data_root, account_id)
            .to_string_lossy()
            .to_string(),
    );
    env.insert("CURSOR_API_KEY".to_string(), token.to_string());
    env
}

pub async fn cursor_env_for_active_account(data_root: &Path) -> Result<HashMap<String, String>> {
    let registry = load_cursor_registry(data_root).await;
    let Some(active) = registry
        .active_account_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(HashMap::new());
    };
    let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
        return Ok(HashMap::new());
    };
    let Some(secret_ref) = entry.secret_ref.as_deref() else {
        bail!("active cursor account has no secret reference");
    };
    let token = read_cursor_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_cursor_account_home(data_root, active).await?;
    Ok(cursor_env_for_account(data_root, active, &token))
}

pub fn normalize_cursor_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Cursor Account {account_id}"))
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

pub async fn subscription_env_for_active_account(
    data_root: &Path,
    provider_id: &str,
) -> Result<HashMap<String, String>> {
    match provider_id {
        "codex" => codex_env_for_active_account(data_root).await,
        "claude-crp" => claude_env_for_active_account(data_root).await,
        "gemini" => gemini_env_for_active_account(data_root).await,
        "kimi" => kimi_env_for_active_account(data_root).await,
        "copilot" => copilot_env_for_active_account(data_root).await,
        "kiro" => kiro_env_for_active_account(data_root).await,
        "cursor" => cursor_env_for_active_account(data_root).await,
        _ => Ok(HashMap::new()),
    }
}

pub async fn subscription_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
    provider_id: &str,
) -> Result<HashMap<String, String>> {
    if data_root == runtime_root {
        return subscription_env_for_active_account(data_root, provider_id).await;
    }

    match provider_id {
        "codex" => codex_env_for_active_account(data_root).await,
        "claude-crp" => {
            let registry = load_claude_registry(data_root).await;
            let Some(active) = registry
                .active_account_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            else {
                return Ok(HashMap::new());
            };
            let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
                return Ok(HashMap::new());
            };
            let Some(secret_ref) = entry.secret_ref.as_deref() else {
                bail!("active claude account has no secret reference");
            };
            let token = read_claude_secret_for_ref(data_root, secret_ref).await?;
            let _ = ensure_claude_account_dir(runtime_root, active).await?;
            Ok(claude_env_for_account(runtime_root, active, &token))
        }
        "gemini" => {
            let registry = load_gemini_registry(data_root).await;
            let Some(active) = registry
                .active_account_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            else {
                return Ok(HashMap::new());
            };
            let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
                return Ok(HashMap::new());
            };
            let Some(secret_ref) = entry.secret_ref.as_deref() else {
                bail!("active gemini account has no secret reference");
            };
            let secret = read_gemini_secret_for_ref(data_root, secret_ref).await?;
            let _ = ensure_gemini_account_home(runtime_root, active, &secret).await?;
            Ok(gemini_env_for_account(runtime_root, active))
        }
        "kimi" => {
            let registry = load_kimi_registry(data_root).await;
            let Some(active) = registry
                .active_account_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            else {
                return Ok(HashMap::new());
            };
            let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
                return Ok(HashMap::new());
            };
            let Some(secret_ref) = entry.secret_ref.as_deref() else {
                bail!("active kimi account has no secret reference");
            };
            let secret = read_kimi_secret_for_ref(data_root, secret_ref).await?;
            let _ = ensure_kimi_account_home(runtime_root, active, &secret).await?;
            Ok(kimi_env_for_account(runtime_root, active))
        }
        "copilot" => copilot_env_for_active_account(data_root).await,
        "kiro" => {
            let registry = load_kiro_registry(data_root).await;
            let Some(active) = registry
                .active_account_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            else {
                return Ok(HashMap::new());
            };
            let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
                return Ok(HashMap::new());
            };
            let Some(secret_ref) = entry.secret_ref.as_deref() else {
                bail!("active kiro account has no secret reference");
            };
            let secret = read_kiro_secret_for_ref(data_root, secret_ref).await?;
            let _ = ensure_kiro_account_home(runtime_root, active, &secret).await?;
            Ok(kiro_env_for_account(runtime_root, active))
        }
        "cursor" => {
            let registry = load_cursor_registry(data_root).await;
            let Some(active) = registry
                .active_account_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            else {
                return Ok(HashMap::new());
            };
            let Some(entry) = registry.accounts.iter().find(|a| a.id == active) else {
                return Ok(HashMap::new());
            };
            let Some(secret_ref) = entry.secret_ref.as_deref() else {
                bail!("active cursor account has no secret reference");
            };
            let token = read_cursor_secret_for_ref(data_root, secret_ref).await?;
            let _ = ensure_cursor_account_home(runtime_root, active).await?;
            Ok(cursor_env_for_account(runtime_root, active, &token))
        }
        _ => Ok(HashMap::new()),
    }
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
    const CLAUDE_TEST_SETUP_TOKEN: &str =
        "sk-ant-oat01-abcDEF1234567890_abcdefghijklmnopqrstuvwxyz_0123456789";

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

    fn assert_unsafe_account_id_error(err: anyhow::Error) {
        assert!(
            err.to_string().contains("single path segment"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn account_id_validation_rejects_path_traversal() {
        ensure_safe_account_id("acct-123").unwrap();
        ensure_safe_account_id("acct_123").unwrap();

        assert!(ensure_safe_account_id("").is_err());
        assert!(ensure_safe_account_id("  ").is_err());
        assert!(ensure_safe_account_id(".").is_err());
        assert!(ensure_safe_account_id("..").is_err());
        assert!(ensure_safe_account_id("../acct").is_err());
        assert!(ensure_safe_account_id("acct/../x").is_err());
        assert!(ensure_safe_account_id("acct/x").is_err());
    }

    #[test]
    fn normalize_claude_setup_token_accepts_wrapped_and_quoted_values() {
        let wrapped = format!("  \"{}\"  ", CLAUDE_TEST_SETUP_TOKEN);
        let normalized = normalize_claude_setup_token(&wrapped).expect("normalize token");
        assert_eq!(normalized, CLAUDE_TEST_SETUP_TOKEN);

        let line_wrapped = "sk-ant-oat01-abcDEF1234567890_\nabcdefghijklmnopqrstuvwxyz_0123456789";
        let normalized =
            normalize_claude_setup_token(line_wrapped).expect("normalize wrapped token");
        assert_eq!(normalized, CLAUDE_TEST_SETUP_TOKEN);
    }

    #[test]
    fn normalize_claude_setup_token_rejects_callback_codes() {
        let err = normalize_claude_setup_token("ePBMdWetJlSbZ0a#state")
            .expect_err("callback token should fail");
        assert!(err.to_string().contains("browser callback code"));
    }

    #[test]
    fn normalize_claude_setup_token_requires_setup_token_prefix() {
        let err = normalize_claude_setup_token("token-abc").expect_err("invalid token should fail");
        assert!(err.to_string().contains("must start with sk-ant-oat"));
    }

    #[tokio::test]
    async fn remove_codex_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_codex_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_claude_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_claude_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_gemini_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_gemini_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_kimi_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_kimi_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_copilot_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_copilot_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_kiro_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_kiro_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
    }

    #[tokio::test]
    async fn remove_cursor_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_cursor_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
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
    async fn import_host_auth_dedupes_existing_account() {
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

        let first = import_host_codex_auth_to_secret_store(root, Some("First".to_string()))
            .await
            .unwrap();
        let first_active = first.active_account_id.clone().expect("active account");
        assert_eq!(first.accounts.len(), 1);

        let second = import_host_codex_auth_to_secret_store(root, Some("Second".to_string()))
            .await
            .unwrap();
        let second_active = second.active_account_id.clone().expect("active account");
        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second_active, first_active);
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

    #[tokio::test]
    async fn claude_active_account_projects_token_to_env() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_claude_account(
            root,
            Some("Claude Test".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let env = claude_env_for_active_account(root).await.unwrap();
        assert_eq!(
            env.get("CLAUDE_CODE_OAUTH_TOKEN"),
            Some(&CLAUDE_TEST_SETUP_TOKEN.to_string())
        );
        let cfg_dir = env
            .get("CLAUDE_CONFIG_DIR")
            .expect("CLAUDE_CONFIG_DIR should be set");
        assert!(cfg_dir.contains(&active_id));
    }

    #[tokio::test]
    async fn adding_existing_claude_account_updates_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let first = add_claude_account(
            root,
            Some("Claude Initial".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_claude_account(
            root,
            Some("Claude Updated".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Claude Updated");
    }

    #[tokio::test]
    async fn deleting_active_claude_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_claude_account(
            root,
            Some("Claude Test".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let _ = remove_claude_account(root, &active_id).await.unwrap();
        let env = claude_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn deleting_active_claude_account_removes_runtime_root_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = root
            .join("containers")
            .join("workspaces")
            .join("workspace-claude")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root).await.unwrap();

        let registry = add_claude_account(
            root,
            Some("Claude Test".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let projected_dir = claude_account_dir(&runtime_root, &active_id);

        let _ = subscription_env_for_active_account_with_runtime_root(
            root,
            &runtime_root,
            "claude-crp",
        )
        .await
        .unwrap();
        assert!(projected_dir.exists());

        let _ = remove_claude_account(root, &active_id).await.unwrap();
        assert!(!projected_dir.exists());
    }

    #[tokio::test]
    async fn gemini_active_account_projects_home_and_auth_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let oauth_creds =
            r#"{"access_token":"token-a","refresh_token":"token-r","token_type":"Bearer"}"#;
        let google_accounts = r#"[{"email":"dev@example.com"}]"#;
        let registry = add_gemini_account(
            root,
            Some("Gemini Test".to_string()),
            oauth_creds.to_string(),
            Some(google_accounts.to_string()),
            Some("dev@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let env = gemini_env_for_active_account(root).await.unwrap();
        let home = env
            .get("GEMINI_CLI_HOME")
            .expect("GEMINI_CLI_HOME should be set");
        assert!(home.contains(&active_id));
        assert_eq!(
            env.get(GEMINI_FORCE_FILE_STORAGE_ENV),
            Some(&"true".to_string())
        );
        let oauth_path = Path::new(home).join(".gemini").join("oauth_creds.json");
        assert!(oauth_path.exists());
        let settings_path = Path::new(home).join(".gemini").join("settings.json");
        let settings_payload = tokio::fs::read_to_string(settings_path).await.unwrap();
        assert!(settings_payload.contains(GEMINI_AUTH_SELECTED_TYPE_OAUTH_PERSONAL));
    }

    #[tokio::test]
    async fn adding_existing_gemini_account_updates_metadata_and_google_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let oauth_creds = r#"{"access_token":"token-a","refresh_token":"token-r"}"#;

        let first = add_gemini_account(
            root,
            Some("Gemini Initial".to_string()),
            oauth_creds.to_string(),
            Some(r#"[{"email":"initial@example.com"}]"#.to_string()),
            Some("initial@example.com".to_string()),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_gemini_account(
            root,
            Some("Gemini Updated".to_string()),
            oauth_creds.to_string(),
            Some(r#"[{"email":"updated@example.com"}]"#.to_string()),
            Some("updated@example.com".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Gemini Updated");
        assert_eq!(
            second.accounts[0].email.as_deref(),
            Some("updated@example.com")
        );

        let secret_ref = second.accounts[0]
            .secret_ref
            .as_deref()
            .expect("secret ref should be set");
        let secret = read_gemini_secret_for_ref(root, secret_ref).await.unwrap();
        assert_eq!(
            secret.google_accounts,
            Some(serde_json::json!([{"email":"updated@example.com"}]))
        );
    }

    #[tokio::test]
    async fn deleting_active_gemini_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_gemini_account(
            root,
            Some("Gemini Test".to_string()),
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let _ = remove_gemini_account(root, &active_id).await.unwrap();
        let env = gemini_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn deleting_active_gemini_account_removes_runtime_root_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = root
            .join("containers")
            .join("workspaces")
            .join("workspace-gemini")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root).await.unwrap();

        let registry = add_gemini_account(
            root,
            Some("Gemini Test".to_string()),
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let projected_home = gemini_account_home(&runtime_root, &active_id);

        let _ =
            subscription_env_for_active_account_with_runtime_root(root, &runtime_root, "gemini")
                .await
                .unwrap();
        assert!(projected_home.exists());

        let _ = remove_gemini_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }

    #[tokio::test]
    async fn kimi_active_account_projects_share_dir_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_kimi_account(
            root,
            Some("Kimi Test".to_string()),
            Some("moonshot".to_string()),
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            Some("dev@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let env = kimi_env_for_active_account(root).await.unwrap();
        let share_dir = env
            .get(KIMI_SHARE_DIR_ENV)
            .expect("KIMI_SHARE_DIR should be set");
        assert!(share_dir.contains(&active_id));
        let credentials_path = Path::new(share_dir)
            .join("credentials")
            .join("moonshot.json");
        assert!(credentials_path.exists());
        let config_path = Path::new(share_dir).join("config.toml");
        assert!(config_path.exists());
    }

    #[tokio::test]
    async fn adding_existing_kimi_account_updates_metadata_and_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let credentials = r#"{"access_token":"token-a","refresh_token":"token-r"}"#;

        let first = add_kimi_account(
            root,
            Some("Kimi Initial".to_string()),
            Some("moonshot".to_string()),
            credentials.to_string(),
            Some("model = \"k1\"".to_string()),
            Some("initial@example.com".to_string()),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_kimi_account(
            root,
            Some("Kimi Updated".to_string()),
            Some("moonshot".to_string()),
            credentials.to_string(),
            Some("model = \"k2\"".to_string()),
            Some("updated@example.com".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Kimi Updated");
        assert_eq!(
            second.accounts[0].email.as_deref(),
            Some("updated@example.com")
        );

        let secret_ref = second.accounts[0]
            .secret_ref
            .as_deref()
            .expect("secret ref should be set");
        let secret = read_kimi_secret_for_ref(root, secret_ref).await.unwrap();
        assert_eq!(secret.config_toml.as_deref(), Some("model = \"k2\""));
    }

    #[tokio::test]
    async fn deleting_active_kimi_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_kimi_account(
            root,
            Some("Kimi Test".to_string()),
            None,
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let _ = remove_kimi_account(root, &active_id).await.unwrap();
        let env = kimi_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn deleting_active_kimi_account_removes_runtime_root_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = root
            .join("containers")
            .join("workspaces")
            .join("workspace-kimi")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root).await.unwrap();

        let registry = add_kimi_account(
            root,
            Some("Kimi Test".to_string()),
            None,
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let projected_home = kimi_account_home(&runtime_root, &active_id);

        let _ = subscription_env_for_active_account_with_runtime_root(root, &runtime_root, "kimi")
            .await
            .unwrap();
        assert!(projected_home.exists());

        let _ = remove_kimi_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }

    #[tokio::test]
    async fn copilot_active_account_projects_token_env() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_copilot_account(
            root,
            Some("Copilot Test".to_string()),
            "ghp_abc".to_string(),
            Some("dev@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let env = copilot_env_for_active_account(root).await.unwrap();
        assert_eq!(env.get("GH_TOKEN"), Some(&"ghp_abc".to_string()));
        assert_eq!(env.get("GITHUB_TOKEN"), Some(&"ghp_abc".to_string()));
        assert!(copilot_account_dir(root, &active_id).exists());
    }

    #[tokio::test]
    async fn adding_existing_copilot_account_updates_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let first = add_copilot_account(
            root,
            Some("Copilot Initial".to_string()),
            "ghp_abc".to_string(),
            Some("initial@example.com".to_string()),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_copilot_account(
            root,
            Some("Copilot Updated".to_string()),
            "ghp_abc".to_string(),
            Some("updated@example.com".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Copilot Updated");
        assert_eq!(
            second.accounts[0].email.as_deref(),
            Some("updated@example.com")
        );
    }

    #[tokio::test]
    async fn deleting_active_copilot_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_copilot_account(
            root,
            Some("Copilot Test".to_string()),
            "ghp_abc".to_string(),
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let _ = remove_copilot_account(root, &active_id).await.unwrap();
        let env = copilot_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn kiro_active_account_projects_token_cache_under_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_kiro_account(
            root,
            Some("Kiro Test".to_string()),
            r#"{"accessToken":"token-a","expiresAt":"2099-01-01T00:00:00Z"}"#.to_string(),
            Some("dev@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let env = kiro_env_for_active_account(root).await.unwrap();
        let home = env.get("HOME").expect("HOME should be set");
        assert!(home.contains(&active_id));
        let token_path = Path::new(home).join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
        assert!(token_path.exists());
    }

    #[tokio::test]
    async fn adding_existing_kiro_account_updates_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let auth_token_json = r#"{"accessToken":"token-a"}"#;

        let first = add_kiro_account(
            root,
            Some("Kiro Initial".to_string()),
            auth_token_json.to_string(),
            Some("initial@example.com".to_string()),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_kiro_account(
            root,
            Some("Kiro Updated".to_string()),
            auth_token_json.to_string(),
            Some("updated@example.com".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Kiro Updated");
        assert_eq!(
            second.accounts[0].email.as_deref(),
            Some("updated@example.com")
        );
    }

    #[tokio::test]
    async fn deleting_active_kiro_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_kiro_account(
            root,
            Some("Kiro Test".to_string()),
            r#"{"accessToken":"token-a"}"#.to_string(),
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let _ = remove_kiro_account(root, &active_id).await.unwrap();
        let env = kiro_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn deleting_active_kiro_account_removes_runtime_root_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = root
            .join("containers")
            .join("workspaces")
            .join("workspace-kiro")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root).await.unwrap();

        let registry = add_kiro_account(
            root,
            Some("Kiro Test".to_string()),
            r#"{"accessToken":"token-a","expiresAt":"2099-01-01T00:00:00Z"}"#.to_string(),
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let projected_home = kiro_account_home(&runtime_root, &active_id);

        let _ = subscription_env_for_active_account_with_runtime_root(root, &runtime_root, "kiro")
            .await
            .unwrap();
        assert!(projected_home.exists());

        let _ = remove_kiro_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }

    #[tokio::test]
    async fn cursor_active_account_projects_config_dir_and_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_cursor_account(
            root,
            Some("Cursor Test".to_string()),
            "cursor-key".to_string(),
            Some("dev@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let env = cursor_env_for_active_account(root).await.unwrap();
        assert_eq!(env.get("CURSOR_API_KEY"), Some(&"cursor-key".to_string()));
        let config_dir = env
            .get("CURSOR_CONFIG_DIR")
            .expect("CURSOR_CONFIG_DIR should be set");
        assert!(config_dir.contains(&active_id));
        assert!(cursor_account_home(root, &active_id).exists());
    }

    #[tokio::test]
    async fn adding_existing_cursor_account_updates_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let first = add_cursor_account(
            root,
            Some("Cursor Initial".to_string()),
            "cursor-key".to_string(),
            Some("initial@example.com".to_string()),
        )
        .await
        .unwrap();
        let first_id = first.active_account_id.clone().expect("active account");

        let second = add_cursor_account(
            root,
            Some("Cursor Updated".to_string()),
            "cursor-key".to_string(),
            Some("updated@example.com".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(second.accounts.len(), 1);
        assert_eq!(second.active_account_id.as_deref(), Some(first_id.as_str()));
        assert_eq!(second.accounts[0].label, "Cursor Updated");
        assert_eq!(
            second.accounts[0].email.as_deref(),
            Some("updated@example.com")
        );
    }

    #[tokio::test]
    async fn deleting_active_cursor_account_clears_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_cursor_account(
            root,
            Some("Cursor Test".to_string()),
            "cursor-key".to_string(),
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let _ = remove_cursor_account(root, &active_id).await.unwrap();
        let env = cursor_env_for_active_account(root).await.unwrap();
        assert!(env.is_empty());
    }

    #[tokio::test]
    async fn deleting_active_cursor_account_removes_runtime_root_projection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = root
            .join("containers")
            .join("workspaces")
            .join("workspace-cursor")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root).await.unwrap();

        let registry = add_cursor_account(
            root,
            Some("Cursor Test".to_string()),
            "cursor-key".to_string(),
            None,
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");
        let projected_home = cursor_account_home(&runtime_root, &active_id);

        let _ =
            subscription_env_for_active_account_with_runtime_root(root, &runtime_root, "cursor")
                .await
                .unwrap();
        assert!(projected_home.exists());

        let _ = remove_cursor_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }

    #[tokio::test]
    async fn subscription_env_dispatches_to_supported_providers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let _ = add_claude_account(
            root,
            Some("Claude".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let _ = add_gemini_account(
            root,
            Some("Gemini".to_string()),
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let _ = add_kimi_account(
            root,
            Some("Kimi".to_string()),
            None,
            r#"{"access_token":"token-a"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let _ = add_copilot_account(
            root,
            Some("Copilot".to_string()),
            "ghp_abc".to_string(),
            None,
        )
        .await
        .unwrap();
        let _ = add_kiro_account(
            root,
            Some("Kiro".to_string()),
            r#"{"accessToken":"token-a"}"#.to_string(),
            None,
        )
        .await
        .unwrap();
        let _ = add_cursor_account(
            root,
            Some("Cursor".to_string()),
            "cursor-key".to_string(),
            None,
        )
        .await
        .unwrap();

        let claude_env = subscription_env_for_active_account(root, "claude-crp")
            .await
            .unwrap();
        assert!(claude_env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        let gemini_env = subscription_env_for_active_account(root, "gemini")
            .await
            .unwrap();
        assert!(gemini_env.contains_key("GEMINI_CLI_HOME"));
        let kimi_env = subscription_env_for_active_account(root, "kimi")
            .await
            .unwrap();
        assert!(kimi_env.contains_key(KIMI_SHARE_DIR_ENV));
        let copilot_env = subscription_env_for_active_account(root, "copilot")
            .await
            .unwrap();
        assert!(copilot_env.contains_key("GH_TOKEN"));
        let kiro_env = subscription_env_for_active_account(root, "kiro")
            .await
            .unwrap();
        assert!(kiro_env.contains_key("HOME"));
        let cursor_env = subscription_env_for_active_account(root, "cursor")
            .await
            .unwrap();
        assert!(cursor_env.contains_key("CURSOR_CONFIG_DIR"));
        let unknown_env = subscription_env_for_active_account(root, "unknown")
            .await
            .unwrap();
        assert!(unknown_env.is_empty());
    }

    #[tokio::test]
    async fn subscription_env_runtime_root_projects_path_based_providers() {
        let dir = tempfile::tempdir().unwrap();
        let runtime_dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let runtime_root = runtime_dir.path();

        let _ = add_claude_account(
            root,
            Some("Claude".to_string()),
            CLAUDE_TEST_SETUP_TOKEN.to_string(),
        )
        .await
        .unwrap();
        let _ = add_gemini_account(
            root,
            Some("Gemini".to_string()),
            r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let _ = add_kimi_account(
            root,
            Some("Kimi".to_string()),
            None,
            r#"{"access_token":"token-a"}"#.to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        let _ = add_kiro_account(
            root,
            Some("Kiro".to_string()),
            r#"{"accessToken":"token-a"}"#.to_string(),
            None,
        )
        .await
        .unwrap();
        let _ = add_cursor_account(
            root,
            Some("Cursor".to_string()),
            "cursor-key".to_string(),
            None,
        )
        .await
        .unwrap();

        let claude_env =
            subscription_env_for_active_account_with_runtime_root(root, runtime_root, "claude-crp")
                .await
                .unwrap();
        let claude_dir = PathBuf::from(claude_env.get("CLAUDE_CONFIG_DIR").unwrap());
        assert!(claude_dir.starts_with(runtime_root));

        let gemini_env =
            subscription_env_for_active_account_with_runtime_root(root, runtime_root, "gemini")
                .await
                .unwrap();
        let gemini_home = PathBuf::from(gemini_env.get("GEMINI_CLI_HOME").unwrap());
        assert!(gemini_home.starts_with(runtime_root));

        let kimi_env =
            subscription_env_for_active_account_with_runtime_root(root, runtime_root, "kimi")
                .await
                .unwrap();
        let kimi_share = PathBuf::from(kimi_env.get(KIMI_SHARE_DIR_ENV).unwrap());
        assert!(kimi_share.starts_with(runtime_root));

        let kiro_env =
            subscription_env_for_active_account_with_runtime_root(root, runtime_root, "kiro")
                .await
                .unwrap();
        let kiro_home = PathBuf::from(kiro_env.get("HOME").unwrap());
        assert!(kiro_home.starts_with(runtime_root));
        let kiro_config = PathBuf::from(kiro_env.get("XDG_CONFIG_HOME").unwrap());
        assert!(kiro_config.starts_with(runtime_root));
        let kiro_cache = PathBuf::from(kiro_env.get("XDG_CACHE_HOME").unwrap());
        assert!(kiro_cache.starts_with(runtime_root));

        let cursor_env =
            subscription_env_for_active_account_with_runtime_root(root, runtime_root, "cursor")
                .await
                .unwrap();
        let cursor_config = PathBuf::from(cursor_env.get("CURSOR_CONFIG_DIR").unwrap());
        assert!(cursor_config.starts_with(runtime_root));
    }
}
