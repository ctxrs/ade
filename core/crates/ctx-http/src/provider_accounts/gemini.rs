use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::shared::{
    apply_email_update, apply_label_update, ensure_safe_account_id, load_json_registry,
    normalize_optional_email, parse_optional_json_value, parse_required_json_object,
    remove_projected_account_home_for_runtime_roots, save_json_registry, write_secure_file_atomic,
};
use super::{
    gemini_account_home, gemini_registry_path, gemini_secret_path,
    GEMINI_AUTH_SELECTED_TYPE_OAUTH_PERSONAL, GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL,
    GEMINI_FORCE_FILE_STORAGE_ENV, GEMINI_SECRET_VERSION,
};

const GEMINI_RUNTIME_AUTH_ENV_KEYS: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_PROJECT_ID",
    "GOOGLE_CLOUD_LOCATION",
];

fn default_gemini_credential_kind() -> String {
    GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()
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
pub(crate) struct GeminiSecretEnvelope {
    version: u32,
    oauth_creds: serde_json::Value,
    #[serde(default)]
    google_accounts: Option<serde_json::Value>,
}

pub async fn load_gemini_registry(data_root: &Path) -> GeminiAccountRegistry {
    load_json_registry(&gemini_registry_path(data_root)).await
}

pub async fn save_gemini_registry(
    data_root: &Path,
    registry: &GeminiAccountRegistry,
) -> Result<()> {
    save_json_registry(&gemini_registry_path(data_root), registry).await
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

pub(crate) fn clear_gemini_runtime_auth_env(env: &mut HashMap<String, String>) {
    for key in GEMINI_RUNTIME_AUTH_ENV_KEYS {
        env.insert((*key).to_string(), String::new());
    }
}

pub(crate) fn apply_gemini_api_key_runtime_auth_env(
    env: &mut HashMap<String, String>,
    api_key: String,
) {
    clear_gemini_runtime_auth_env(env);
    env.insert("GEMINI_API_KEY".to_string(), api_key);
}

pub(crate) fn apply_gemini_vertex_runtime_auth_env(
    env: &mut HashMap<String, String>,
    api_key: String,
) {
    clear_gemini_runtime_auth_env(env);
    env.insert("GOOGLE_API_KEY".to_string(), api_key);
    env.insert("GOOGLE_GENAI_USE_VERTEXAI".to_string(), "true".to_string());
}

pub fn gemini_env_for_account(data_root: &Path, account_id: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    clear_gemini_runtime_auth_env(&mut env);
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

pub(crate) async fn gemini_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<HashMap<String, String>> {
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

pub fn normalize_gemini_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Gemini Account {account_id}"))
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
    let envelope = GeminiSecretEnvelope {
        version: GEMINI_SECRET_VERSION,
        oauth_creds,
        google_accounts,
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

pub(crate) async fn read_gemini_secret_for_ref(
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
    write_gemini_auth_settings(&gemini_dir, GEMINI_AUTH_SELECTED_TYPE_OAUTH_PERSONAL).await?;
    Ok(home)
}

pub(crate) async fn write_gemini_auth_settings(
    gemini_dir: &Path,
    selected_type: &str,
) -> Result<()> {
    let settings = serde_json::json!({
        "security": {
            "auth": {
                "selectedType": selected_type
            }
        }
    });
    write_secure_file_atomic(
        &gemini_dir.join("settings.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )
    .await
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
    async fn remove_gemini_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_gemini_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
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

        let _ = gemini_env_for_active_account_with_runtime_root(root, &runtime_root)
            .await
            .unwrap();
        assert!(projected_home.exists());

        let _ = remove_gemini_account(root, &active_id).await.unwrap();
        assert!(!projected_home.exists());
    }
}
