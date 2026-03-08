use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::shared::{
    apply_email_update, apply_label_update, ensure_safe_account_id, load_json_registry,
    normalize_optional_email, save_json_registry, write_secure_file_atomic,
};
use super::{
    copilot_account_dir, copilot_registry_path, copilot_secret_path,
    COPILOT_CREDENTIAL_KIND_GH_TOKEN, COPILOT_SECRET_VERSION,
};

fn default_copilot_credential_kind() -> String {
    COPILOT_CREDENTIAL_KIND_GH_TOKEN.to_string()
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
struct CopilotSecretEnvelope {
    version: u32,
    gh_token: String,
}

pub async fn load_copilot_registry(data_root: &Path) -> CopilotAccountRegistry {
    load_json_registry(&copilot_registry_path(data_root)).await
}

pub async fn save_copilot_registry(
    data_root: &Path,
    registry: &CopilotAccountRegistry,
) -> Result<()> {
    save_json_registry(&copilot_registry_path(data_root), registry).await
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unsafe_account_id_error(err: anyhow::Error) {
        assert!(
            err.to_string().contains("single path segment"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn remove_copilot_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_copilot_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
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
}
