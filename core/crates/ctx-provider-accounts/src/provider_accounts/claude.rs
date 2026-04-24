use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::shared::{
    apply_label_update, collect_secret_paths, ensure_account_exists, ensure_safe_account_id,
    load_json_registry, prepend_dir_to_path_env, remove_projected_account_home_for_runtime_roots,
    save_json_registry, write_secure_file_atomic,
};
use super::{
    CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN, CLAUDE_SECRET_VERSION, claude_account_dir,
    claude_registry_path, claude_secret_path,
};

const CLAUDE_AUTH_ENV_KEY: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const CLAUDE_CONFIG_DIR_ENV_KEY: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_CREDENTIALS_FILENAME: &str = ".credentials.json";
const CLAUDE_CONFIG_FILENAME: &str = ".claude.json";
const CLAUDE_SECURITY_SHIM_DIRNAME: &str = ".ctx-bin";
const CLAUDE_SECURITY_SHIM_FILENAME: &str = "security";
const CLAUDE_SECURITY_SHIM: &str = r#"#!/bin/sh
log_file="${CLAUDE_CONFIG_DIR:-$HOME}/ctx-security.log"
subcommand="$1"
service=""
account=""
shift || true
while [ "$#" -gt 0 ]; do
  case "$1" in
    -s)
      shift || true
      service="${1:-}"
      ;;
    -a)
      shift || true
      account="${1:-}"
      ;;
  esac
  shift || true
done
{
  printf '%s subcommand=%s service=%s account=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$subcommand" "$service" "$account"
} >>"$log_file" 2>/dev/null

case "$subcommand" in
  show-keychain-info)
    exit 1
    ;;
  find-generic-password|delete-generic-password)
    exit 44
    ;;
  add-generic-password)
    exit 1
    ;;
  *)
    exec /usr/bin/security "$@"
    ;;
esac
"#;

fn default_claude_credential_kind() -> String {
    CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN.to_string()
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
struct ClaudeSecretEnvelope {
    version: u32,
    #[serde(default, alias = "anthropic_auth_token")]
    claude_code_oauth_token: Option<String>,
}

pub async fn load_claude_registry(data_root: &Path) -> ClaudeAccountRegistry {
    let mut registry: ClaudeAccountRegistry =
        load_json_registry(&claude_registry_path(data_root)).await;
    let legacy_account_ids: Vec<String> = registry
        .accounts
        .iter()
        .filter(|entry| entry.kind.trim() != CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN)
        .map(|entry| entry.id.clone())
        .collect();
    if legacy_account_ids.is_empty() {
        return registry;
    }

    let legacy_account_id_set: std::collections::HashSet<&str> =
        legacy_account_ids.iter().map(String::as_str).collect();
    let removed_entries: Vec<ClaudeAccountEntry> = registry
        .accounts
        .iter()
        .filter(|entry| legacy_account_id_set.contains(entry.id.as_str()))
        .cloned()
        .collect();
    let secret_paths = match collect_secret_paths(
        data_root,
        removed_entries
            .iter()
            .filter_map(|entry| entry.secret_ref.as_deref()),
        claude_secret_path,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!(
                "refusing to prune legacy Claude accounts with invalid secret_ref: {error:#}"
            );
            return registry;
        }
    };
    registry
        .accounts
        .retain(|entry| !legacy_account_id_set.contains(entry.id.as_str()));
    if registry
        .active_account_id
        .as_deref()
        .is_some_and(|account_id| legacy_account_id_set.contains(account_id))
    {
        registry.active_account_id = None;
    }
    if let Err(error) = save_json_registry(&claude_registry_path(data_root), &registry).await {
        tracing::warn!("failed to persist migrated Claude registry: {error}");
    }
    for secret_path in secret_paths {
        let _ = tokio::fs::remove_file(secret_path).await;
    }
    for entry in removed_entries {
        let account_dir = claude_account_dir(data_root, &entry.id);
        let _ = tokio::fs::remove_dir_all(account_dir).await;
        let _ = remove_projected_account_home_for_runtime_roots(
            data_root,
            &entry.id,
            claude_account_dir,
            "claude-crp",
        )
        .await;
    }
    registry
}

pub async fn save_claude_registry(
    data_root: &Path,
    registry: &ClaudeAccountRegistry,
) -> Result<()> {
    save_json_registry(&claude_registry_path(data_root), registry).await
}

pub async fn ensure_claude_account_dir(data_root: &Path, account_id: &str) -> Result<PathBuf> {
    let dir = claude_account_dir(data_root, account_id);
    tokio::fs::create_dir_all(&dir).await?;
    Ok(dir)
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
        if let Ok(existing_secret) = read_claude_secret_for_ref(data_root, secret_ref).await {
            if existing_secret.claude_code_oauth_token.as_deref() == Some(token.as_str()) {
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
    ensure_account_exists(!removed.is_empty())?;
    let secret_paths = collect_secret_paths(
        data_root,
        removed
            .iter()
            .filter_map(|entry| entry.secret_ref.as_deref()),
        claude_secret_path,
    )?;
    registry.accounts.retain(|a| a.id != account_id);
    if was_active {
        registry.active_account_id = None;
    }
    save_claude_registry(data_root, &registry).await?;

    for secret_path in secret_paths {
        if secret_path.exists() {
            let _ = tokio::fs::remove_file(secret_path).await;
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

pub(crate) fn claude_security_shim_dir(home: &Path) -> PathBuf {
    home.join(CLAUDE_SECURITY_SHIM_DIRNAME)
}

pub(crate) async fn ensure_claude_security_shim(home: &Path) -> Result<PathBuf> {
    let dir = claude_security_shim_dir(home);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating claude shim dir {}", dir.display()))?;
    let path = dir.join(CLAUDE_SECURITY_SHIM_FILENAME);
    write_secure_file_atomic(&path, CLAUDE_SECURITY_SHIM.as_bytes()).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .await
            .with_context(|| {
                format!("marking claude security shim executable {}", path.display())
            })?;
    }
    Ok(dir)
}

pub(crate) fn claude_security_shim_path_env(home: &Path) -> Result<String> {
    prepend_dir_to_path_env(&claude_security_shim_dir(home))
}

pub fn claude_env_for_account(
    data_root: &Path,
    account_id: &str,
    setup_token: &str,
) -> Result<HashMap<String, String>> {
    let mut env = HashMap::new();
    let account_home = claude_account_dir(data_root, account_id);
    env.insert(CLAUDE_AUTH_ENV_KEY.to_string(), setup_token.to_string());
    env.insert(
        CLAUDE_CONFIG_DIR_ENV_KEY.to_string(),
        account_home.to_string_lossy().to_string(),
    );
    env.insert(
        "PATH".to_string(),
        claude_security_shim_path_env(&account_home)?,
    );
    Ok(env)
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

    let secret = read_claude_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_claude_account_home(data_root, active, &secret).await?;
    claude_env_from_secret(data_root, active, &secret)
}

pub(crate) async fn claude_env_for_active_account_with_runtime_root(
    data_root: &Path,
    runtime_root: &Path,
) -> Result<HashMap<String, String>> {
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
    let secret = read_claude_secret_for_ref(data_root, secret_ref).await?;
    let _ = ensure_claude_account_home(runtime_root, active, &secret).await?;
    claude_env_from_secret(runtime_root, active, &secret)
}

pub fn normalize_claude_label(label: Option<String>, account_id: &str) -> String {
    label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Claude Account {account_id}"))
}

pub(crate) fn normalize_claude_setup_token(token: &str) -> Result<String> {
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
    let path = claude_secret_path(data_root, &secret_ref)?;
    let envelope = ClaudeSecretEnvelope {
        version: CLAUDE_SECRET_VERSION,
        claude_code_oauth_token: Some(token),
    };
    write_secure_file_atomic(&path, &serde_json::to_vec_pretty(&envelope)?).await?;
    Ok(secret_ref)
}

async fn read_claude_secret_for_ref(
    data_root: &Path,
    secret_ref: &str,
) -> Result<ClaudeSecretEnvelope> {
    let path = claude_secret_path(data_root, secret_ref)?;
    let payload = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading claude secret {}", path.display()))?;
    let parsed: ClaudeSecretEnvelope = serde_json::from_str(&payload)
        .with_context(|| format!("invalid claude secret {}", path.display()))?;
    if parsed.version != CLAUDE_SECRET_VERSION {
        bail!(
            "unsupported claude secret version {} at {}",
            parsed.version,
            path.display()
        );
    }
    if let Some(token) = parsed.claude_code_oauth_token.as_deref() {
        let _ = normalize_claude_setup_token(token)?;
    }
    if parsed.claude_code_oauth_token.is_none() {
        bail!("claude secret must contain a setup token");
    }
    Ok(parsed)
}

fn claude_env_from_secret(
    data_root: &Path,
    account_id: &str,
    secret: &ClaudeSecretEnvelope,
) -> Result<HashMap<String, String>> {
    if let Some(token) = secret.claude_code_oauth_token.as_deref() {
        return claude_env_for_account(data_root, account_id, token);
    }
    bail!("claude secret did not contain usable credentials")
}

async fn ensure_claude_account_home(
    data_root: &Path,
    account_id: &str,
    secret: &ClaudeSecretEnvelope,
) -> Result<PathBuf> {
    let dir = ensure_claude_account_dir(data_root, account_id).await?;
    let _ = secret;
    remove_optional_file(&dir.join(CLAUDE_CREDENTIALS_FILENAME)).await?;
    remove_optional_file(&dir.join(CLAUDE_CONFIG_FILENAME)).await?;
    let _ = ensure_claude_security_shim(&dir).await?;
    Ok(dir)
}

async fn remove_optional_file(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_accounts::CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN;

    const CLAUDE_TEST_SETUP_TOKEN: &str =
        "sk-ant-oat01-abcDEF1234567890_abcdefghijklmnopqrstuvwxyz_0123456789";

    fn assert_unsafe_account_id_error(err: anyhow::Error) {
        assert!(
            err.to_string().contains("single path segment"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn normalize_claude_setup_token_accepts_wrapped_and_quoted_values() {
        let wrapped = format!("  \"{CLAUDE_TEST_SETUP_TOKEN}\"  ");
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
    async fn remove_claude_account_rejects_unsafe_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove_claude_account(dir.path(), "..").await.unwrap_err();
        assert_unsafe_account_id_error(err);
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
            .get(CLAUDE_CONFIG_DIR_ENV_KEY)
            .expect("CLAUDE_CONFIG_DIR should be set");
        assert!(cfg_dir.contains(&active_id));
        let shim_dir = PathBuf::from(cfg_dir).join(CLAUDE_SECURITY_SHIM_DIRNAME);
        let shim_path = shim_dir.join(CLAUDE_SECURITY_SHIM_FILENAME);
        let path_env = env.get("PATH").expect("PATH should be set");
        assert!(shim_path.exists());
        assert!(
            path_env.starts_with(&shim_dir.to_string_lossy().to_string()),
            "expected PATH to start with shim dir, got {path_env}"
        );
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

        let _ = claude_env_for_active_account_with_runtime_root(root, &runtime_root)
            .await
            .unwrap();
        assert!(projected_dir.exists());

        let _ = remove_claude_account(root, &active_id).await.unwrap();
        assert!(!projected_dir.exists());
    }

    #[tokio::test]
    async fn load_claude_registry_prunes_legacy_non_setup_token_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let legacy_id = "legacy-claude-oauth";
        let setup_id = "setup-claude";
        let legacy_secret_ref = format!("{legacy_id}.json");
        let setup_secret_ref =
            write_claude_secret_for_account(root, setup_id, CLAUDE_TEST_SETUP_TOKEN)
                .await
                .expect("write setup secret");
        let legacy_secret_path = claude_secret_path(root, &legacy_secret_ref).unwrap();
        write_secure_file_atomic(
            &legacy_secret_path,
            br#"{"version":1,"anthropic_auth_token":"sk-ant-oat01-legacy1234567890_abcdefghijklmnopqrstuvwxyz_0123456789"}"#,
        )
        .await
        .expect("write legacy secret");
        let legacy_account_dir = claude_account_dir(root, legacy_id);
        tokio::fs::create_dir_all(&legacy_account_dir)
            .await
            .expect("create legacy account dir");

        let registry = ClaudeAccountRegistry {
            active_account_id: Some(legacy_id.to_string()),
            accounts: vec![
                ClaudeAccountEntry {
                    id: legacy_id.to_string(),
                    label: "Legacy OAuth".to_string(),
                    kind: "claude-ai-oauth".to_string(),
                    email: Some("legacy@example.com".to_string()),
                    subscription_type: Some("pro".to_string()),
                    created_at: Utc::now(),
                    last_used_at: Some(Utc::now()),
                    secret_ref: Some(legacy_secret_ref.clone()),
                },
                ClaudeAccountEntry {
                    id: setup_id.to_string(),
                    label: "Setup Token".to_string(),
                    kind: CLAUDE_CREDENTIAL_KIND_SETUP_TOKEN.to_string(),
                    email: None,
                    subscription_type: None,
                    created_at: Utc::now(),
                    last_used_at: Some(Utc::now()),
                    secret_ref: Some(setup_secret_ref),
                },
            ],
        };
        save_claude_registry(root, &registry)
            .await
            .expect("save registry");

        let loaded = load_claude_registry(root).await;
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].id, setup_id);
        assert_eq!(loaded.active_account_id, None);
        assert!(!legacy_secret_path.exists());
        assert!(!legacy_account_dir.exists());
    }
}
