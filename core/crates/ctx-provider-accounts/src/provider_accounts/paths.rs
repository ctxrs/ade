use std::path::{Path, PathBuf};

use super::shared::ensure_safe_secret_ref;
use super::CODEX_RUNTIME_OWNER_FILE;
use anyhow::Result;
use ctx_core::provider_ids::{CODEX_CRP_ADAPTER_ID, CODEX_PROVIDER_ID};

const CODEX_PROVIDER_ROOT: &str = CODEX_PROVIDER_ID;

pub fn legacy_codex_accounts_root(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_CRP_ADAPTER_ID)
        .join("accounts")
}

pub fn codex_accounts_root(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_PROVIDER_ROOT)
        .join("accounts")
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

pub fn qwen_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("qwen").join("accounts")
}

pub fn kimi_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("kimi").join("accounts")
}

pub fn mistral_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("mistral").join("accounts")
}

pub fn copilot_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("copilot").join("accounts")
}

pub fn cursor_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("cursor").join("accounts")
}

pub fn amp_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("amp").join("accounts")
}

pub fn legacy_codex_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join(CODEX_CRP_ADAPTER_ID)
}

pub fn codex_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join(CODEX_PROVIDER_ROOT)
}

pub fn claude_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("claude-crp")
}

pub fn gemini_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("gemini")
}

pub fn qwen_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("qwen")
}

pub fn kimi_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("kimi")
}

pub fn copilot_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("copilot")
}

pub fn cursor_secrets_root(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("cursor")
}

pub(crate) fn codex_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(codex_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn claude_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(claude_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn gemini_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(gemini_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn qwen_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(qwen_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn kimi_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(kimi_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn copilot_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(copilot_secrets_root(data_root).join(secret_ref))
}

pub(crate) fn cursor_secret_path(data_root: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_secret_ref(secret_ref)?;
    Ok(cursor_secrets_root(data_root).join(secret_ref))
}

pub fn legacy_codex_runtime_home(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_CRP_ADAPTER_ID)
        .join("home")
}

pub fn codex_runtime_home(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_PROVIDER_ROOT)
        .join("home")
}

pub fn codex_broker_home(data_root: &Path, account_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_PROVIDER_ROOT)
        .join("brokers")
        .join(account_id)
        .join("home")
}

pub fn codex_account_deletion_marker(data_root: &Path, account_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join(CODEX_PROVIDER_ROOT)
        .join("deleting")
        .join(account_id)
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

pub fn qwen_registry_path(data_root: &Path) -> PathBuf {
    qwen_accounts_root(data_root).join("index.json")
}

pub fn kimi_registry_path(data_root: &Path) -> PathBuf {
    kimi_accounts_root(data_root).join("index.json")
}

pub fn mistral_registry_path(data_root: &Path) -> PathBuf {
    mistral_accounts_root(data_root).join("index.json")
}

pub fn copilot_registry_path(data_root: &Path) -> PathBuf {
    copilot_accounts_root(data_root).join("index.json")
}

pub fn cursor_registry_path(data_root: &Path) -> PathBuf {
    cursor_accounts_root(data_root).join("index.json")
}

pub fn amp_registry_path(data_root: &Path) -> PathBuf {
    amp_accounts_root(data_root).join("index.json")
}

pub fn amp_runtime_home(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("amp").join("home")
}

pub fn mistral_runtime_home(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("mistral").join("home")
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

pub fn qwen_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    qwen_accounts_root(data_root).join(account_id)
}

pub fn kimi_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    kimi_accounts_root(data_root).join(account_id)
}

pub fn mistral_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    mistral_accounts_root(data_root).join(account_id)
}

pub fn copilot_account_dir(data_root: &Path, account_id: &str) -> PathBuf {
    copilot_accounts_root(data_root).join(account_id)
}

pub fn cursor_account_home(data_root: &Path, account_id: &str) -> PathBuf {
    cursor_accounts_root(data_root).join(account_id)
}

pub(crate) fn codex_runtime_owner_path(data_root: &Path) -> PathBuf {
    codex_runtime_home(data_root).join(CODEX_RUNTIME_OWNER_FILE)
}
