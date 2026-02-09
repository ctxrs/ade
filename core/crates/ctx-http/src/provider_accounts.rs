use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccountEntry {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
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
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn codex_accounts_root(data_root: &Path) -> PathBuf {
    data_root.join("providers").join("codex").join("accounts")
}

pub fn codex_fallback_home(data_root: &Path) -> PathBuf {
    codex_accounts_root(data_root).join("fallback")
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
    if let Some(existing) = registry.accounts.iter_mut().find(|a| a.id == entry.id) {
        *existing = entry;
    } else {
        registry.accounts.push(entry);
    }
    save_codex_registry(data_root, &registry).await?;
    Ok(registry)
}

pub async fn remove_codex_account(
    data_root: &Path,
    account_id: &str,
) -> Result<CodexAccountRegistry> {
    let mut registry = load_codex_registry(data_root).await;
    registry.accounts.retain(|a| a.id != account_id);
    if registry.active_account_id.as_deref() == Some(account_id) {
        registry.active_account_id = None;
    }
    save_codex_registry(data_root, &registry).await?;
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
    registry.active_account_id = account_id.clone();
    if let Some(active_id) = account_id {
        let now = Utc::now();
        if let Some(entry) = registry.accounts.iter_mut().find(|a| a.id == active_id) {
            entry.last_used_at = Some(now);
        }
    }
    save_codex_registry(data_root, &registry).await?;
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

pub async fn codex_env_for_fallback_root(state_root: &Path) -> Result<HashMap<String, String>> {
    let fallback = codex_fallback_home(state_root);
    tokio::fs::create_dir_all(&fallback).await?;
    let mut env = HashMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        fallback.to_string_lossy().to_string(),
    );
    Ok(env)
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
        let _ = ensure_codex_account_dir(data_root, active).await?;
        return Ok(codex_env_for_account(data_root, active));
    }

    codex_env_for_fallback_root(data_root).await
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
    async fn codex_env_uses_active_account_dir() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = CodexAccountRegistry {
            active_account_id: Some("acct-123".to_string()),
            accounts: Vec::new(),
        };
        save_codex_registry(root, &registry).await.unwrap();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_account_dir(root, "acct-123").to_string_lossy());
        assert!(codex_account_dir(root, "acct-123").exists());
    }

    #[tokio::test]
    async fn codex_env_falls_back_to_ctx_home() {
        let _env_lock = lock_env().await;
        let _guard = EnvGuard::without("CTX_CODEX_HOME");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let env = codex_env_for_active_account(root).await.unwrap();
        let home = env.get("CODEX_HOME").unwrap();
        assert_eq!(home, &codex_fallback_home(root).to_string_lossy());
        assert!(codex_fallback_home(root).exists());
    }
}
