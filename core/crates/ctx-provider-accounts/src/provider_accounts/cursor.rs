use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::shared::{
    apply_email_update, apply_label_update, ensure_safe_account_id, load_json_registry,
    normalize_optional_email, remove_projected_account_home_for_runtime_roots, save_json_registry,
    write_secure_file_atomic,
};
use super::{
    cursor_account_home, cursor_registry_path, cursor_secret_path, CURSOR_CREDENTIAL_KIND_API_KEY,
    CURSOR_CREDENTIAL_KIND_OAUTH_TOKEN, CURSOR_SECRET_VERSION,
};

fn default_cursor_credential_kind() -> String {
    CURSOR_CREDENTIAL_KIND_API_KEY.to_string()
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
struct CursorSecretEnvelope {
    version: u32,
    #[serde(alias = "api_key")]
    auth_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorLoginStatus {
    pub login_id: String,
    #[serde(default)]
    pub auth_url: Option<String>,
    pub status: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct CursorSecretRecord {
    auth_token: String,
    refresh_token: Option<String>,
}

pub async fn load_cursor_registry(data_root: &Path) -> CursorAccountRegistry {
    load_json_registry(&cursor_registry_path(data_root)).await
}

pub async fn save_cursor_registry(
    data_root: &Path,
    registry: &CursorAccountRegistry,
) -> Result<()> {
    save_json_registry(&cursor_registry_path(data_root), registry).await
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
    upsert_cursor_account_internal(
        data_root,
        label,
        token,
        None,
        email,
        CURSOR_CREDENTIAL_KIND_API_KEY,
    )
    .await
}

pub async fn add_cursor_oauth_account(
    data_root: &Path,
    label: Option<String>,
    auth_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
) -> Result<CursorAccountRegistry> {
    upsert_cursor_account_internal(
        data_root,
        label,
        auth_token,
        refresh_token,
        email,
        CURSOR_CREDENTIAL_KIND_OAUTH_TOKEN,
    )
    .await
}

async fn upsert_cursor_account_internal(
    data_root: &Path,
    label: Option<String>,
    auth_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
    credential_kind: &str,
) -> Result<CursorAccountRegistry> {
    let auth_token = normalize_cursor_auth_token(&auth_token)?;
    let refresh_token = normalize_optional_cursor_auth_token(refresh_token.as_deref())?;
    let mut registry = load_cursor_registry(data_root).await;
    let mut existing_account_id: Option<String> = None;

    for existing in &registry.accounts {
        let Some(secret_ref) = existing.secret_ref.as_deref() else {
            continue;
        };
        if let Ok(existing_secret) = read_cursor_secret_for_ref(data_root, secret_ref).await {
            if existing_secret.auth_token == auth_token {
                existing_account_id = Some(existing.id.clone());
                break;
            }
        }
    }

    if let Some(account_id) = existing_account_id {
        let existing_secret_ref = registry
            .accounts
            .iter()
            .find(|entry| entry.id == account_id)
            .and_then(|entry| entry.secret_ref.clone());
        let existing_refresh_token = if let Some(secret_ref) = existing_secret_ref.as_deref() {
            read_cursor_secret_for_ref(data_root, secret_ref)
                .await
                .ok()
                .and_then(|secret| secret.refresh_token)
        } else {
            None
        };
        let next_refresh_token = refresh_token.clone().or(existing_refresh_token);
        if let Some(entry) = registry
            .accounts
            .iter_mut()
            .find(|entry| entry.id == account_id)
        {
            if let Some(secret_ref) = existing_secret_ref {
                write_cursor_secret_for_ref(
                    data_root,
                    &secret_ref,
                    &auth_token,
                    next_refresh_token.as_deref(),
                )
                .await?;
            } else {
                entry.secret_ref = Some(
                    write_cursor_secret_for_account(
                        data_root,
                        &account_id,
                        &auth_token,
                        next_refresh_token.as_deref(),
                    )
                    .await?,
                );
            }
            apply_label_update(label.clone(), &mut entry.label);
            apply_email_update(email.clone(), &mut entry.email);
            entry.kind = credential_kind.to_string();
            entry.last_used_at = Some(Utc::now());
        }
        registry.active_account_id = Some(account_id);
        save_cursor_registry(data_root, &registry).await?;
        return Ok(registry);
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let secret_ref = write_cursor_secret_for_account(
        data_root,
        &account_id,
        &auth_token,
        refresh_token.as_deref(),
    )
    .await?;
    let entry = CursorAccountEntry {
        id: account_id.clone(),
        label: normalize_cursor_label(label, &account_id),
        kind: credential_kind.to_string(),
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
    auth_token: &str,
    credential_kind: &str,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CURSOR_CONFIG_DIR".to_string(),
        cursor_account_home(data_root, account_id)
            .to_string_lossy()
            .to_string(),
    );
    match credential_kind {
        CURSOR_CREDENTIAL_KIND_OAUTH_TOKEN => {
            env.insert("CURSOR_AUTH_TOKEN".to_string(), auth_token.to_string());
        }
        _ => {
            env.insert("CURSOR_API_KEY".to_string(), auth_token.to_string());
        }
    }
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
    let secret = read_cursor_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_cursor_account_home(data_root, active).await?;
    Ok(cursor_env_for_account(
        data_root,
        active,
        &secret.auth_token,
        &entry.kind,
    ))
}

pub(crate) async fn cursor_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<HashMap<String, String>> {
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
    let secret = read_cursor_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_cursor_account_home(runtime_root, active).await?;
    Ok(cursor_env_for_account(
        runtime_root,
        active,
        &secret.auth_token,
        &entry.kind,
    ))
}

pub fn normalize_cursor_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Cursor Account {account_id}"))
}

fn normalize_cursor_auth_token(token: &str) -> Result<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        bail!("auth token is required");
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_cursor_auth_token(token: Option<&str>) -> Result<Option<String>> {
    match token {
        Some(value) => normalize_cursor_auth_token(value).map(Some),
        None => Ok(None),
    }
}

async fn write_cursor_secret_for_account(
    data_root: &Path,
    account_id: &str,
    auth_token: &str,
    refresh_token: Option<&str>,
) -> Result<String> {
    let secret_ref = format!("{account_id}.json");
    write_cursor_secret_for_ref(data_root, &secret_ref, auth_token, refresh_token).await?;
    Ok(secret_ref)
}

async fn write_cursor_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
    auth_token: &str,
    refresh_token: Option<&str>,
) -> Result<()> {
    let auth_token = normalize_cursor_auth_token(auth_token)?;
    let refresh_token =
        normalize_optional_cursor_auth_token(refresh_token)?.filter(|token| token != &auth_token);
    let path = cursor_secret_path(data_root, secret_ref);
    let envelope = CursorSecretEnvelope {
        version: CURSOR_SECRET_VERSION,
        auth_token,
        refresh_token,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(())
}

async fn read_cursor_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
) -> Result<CursorSecretRecord> {
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
    Ok(CursorSecretRecord {
        auth_token: normalize_cursor_auth_token(&parsed.auth_token)?,
        refresh_token: normalize_optional_cursor_auth_token(parsed.refresh_token.as_deref())?,
    })
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
    async fn remove_cursor_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_cursor_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
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
        assert!(!env.contains_key("CURSOR_AUTH_TOKEN"));
        let config_dir = env
            .get("CURSOR_CONFIG_DIR")
            .expect("CURSOR_CONFIG_DIR should be set");
        assert!(config_dir.contains(&active_id));
        assert!(cursor_account_home(root, &active_id).exists());
    }

    #[tokio::test]
    async fn cursor_active_oauth_account_projects_config_dir_and_auth_token() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registry = add_cursor_oauth_account(
            root,
            Some("Cursor OAuth".to_string()),
            "cursor-access-token".to_string(),
            Some("cursor-refresh-token".to_string()),
            Some("oauth@example.com".to_string()),
        )
        .await
        .unwrap();
        let active_id = registry.active_account_id.clone().expect("active account");

        let env = cursor_env_for_active_account(root).await.unwrap();
        assert_eq!(
            env.get("CURSOR_AUTH_TOKEN"),
            Some(&"cursor-access-token".to_string())
        );
        assert!(!env.contains_key("CURSOR_API_KEY"));
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

        let _ = cursor_env_for_active_account_with_runtime_root(root, &runtime_root)
            .await
            .unwrap();
        assert!(projected_home.exists());

        let _ = remove_cursor_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }
}
