use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::harness_sources;
use crate::provider_accounts;

const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAuthImportCandidate {
    pub id: String,
    pub provider_id: String,
    pub provider_label: String,
    pub kind: String,
    pub path: String,
    pub signal_strength: String,
    pub confidence: String,
    pub parse_status: String,
    #[serde(default)]
    pub unsupported_reason: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub account_identity: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub auth_type: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub last_modified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAuthImportResult {
    pub candidate_id: String,
    pub provider_id: String,
    pub status: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderImportedAuthProfile {
    pub id: String,
    pub provider_id: String,
    pub provider_label: String,
    pub label: String,
    #[serde(default)]
    pub account_identity: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub auth_type: Option<String>,
    pub source_path: String,
    pub source_kind: String,
    pub secret_fingerprint: String,
    pub imported_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderImportedAuthRegistry {
    #[serde(default)]
    pub profiles: Vec<ProviderImportedAuthProfile>,
}

#[derive(Debug, Clone)]
struct CandidateMaterial {
    candidate: ProviderAuthImportCandidate,
    importable: bool,
    secret_bytes: Option<Vec<u8>>,
    label: Option<String>,
}

#[derive(Debug, Clone)]
struct HostRoots {
    home: PathBuf,
    xdg_config: PathBuf,
    xdg_data: PathBuf,
    codex_home: PathBuf,
}

#[derive(Debug, Clone)]
struct PathSpec {
    provider_id: &'static str,
    provider_label: &'static str,
    kind: &'static str,
    signal_strength: &'static str,
    confidence: &'static str,
    importable: bool,
    unsupported_reason: Option<&'static str>,
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecretMaterial {
    kind: String,
    source_path: String,
    #[serde(default)]
    content_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyMigrationMarker {
    version: u32,
    completed_at: DateTime<Utc>,
}

fn imported_registry_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("auth_import")
        .join("profiles.json")
}

fn imported_secrets_dir(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("auth_import")
        .join("secrets")
}

fn imported_secret_path(data_root: &Path, profile_id: &str) -> PathBuf {
    imported_secrets_dir(data_root).join(format!("{profile_id}.json"))
}

fn legacy_migration_marker_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("auth_import")
        .join("migration_v1.json")
}

pub async fn load_imported_registry(data_root: &Path) -> ProviderImportedAuthRegistry {
    let path = imported_registry_path(data_root);
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => ProviderImportedAuthRegistry::default(),
    }
}

pub async fn save_imported_registry(
    data_root: &Path,
    registry: &ProviderImportedAuthRegistry,
) -> Result<()> {
    let path = imported_registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

async fn legacy_migration_marker_exists(data_root: &Path) -> bool {
    tokio::fs::metadata(legacy_migration_marker_path(data_root))
        .await
        .is_ok()
}

async fn write_legacy_migration_marker(data_root: &Path) -> Result<()> {
    let marker_path = legacy_migration_marker_path(data_root);
    if let Some(parent) = marker_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let marker = LegacyMigrationMarker {
        version: 1,
        completed_at: Utc::now(),
    };
    tokio::fs::write(marker_path, serde_json::to_vec_pretty(&marker)?).await?;
    Ok(())
}

fn host_roots() -> Result<HostRoots> {
    let base = directories::BaseDirs::new().context("missing home directory")?;
    let home = base.home_dir().to_path_buf();
    let xdg_config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let xdg_data = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    let codex_home = std::env::var("CODEX_HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    Ok(HostRoots {
        home,
        xdg_config,
        xdg_data,
        codex_home,
    })
}

fn build_catalog(roots: &HostRoots) -> Vec<PathSpec> {
    let mut out = vec![
        PathSpec {
            provider_id: "codex",
            provider_label: "Codex",
            kind: "auth_file",
            signal_strength: "strong",
            confidence: "high",
            importable: true,
            unsupported_reason: None,
            path: roots.codex_home.join("auth.json"),
        },
        PathSpec {
            provider_id: "amp",
            provider_label: "Amp",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some(
                "Amp auth is often keychain-backed; no canonical importable auth file was found.",
            ),
            path: roots.xdg_config.join("amp").join("settings.json"),
        },
        PathSpec {
            provider_id: "copilot",
            provider_label: "Copilot",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some(
                "Copilot auth storage is not a stable canonical file path in available docs.",
            ),
            path: roots.home.join(".copilot").join("lsp-config.json"),
        },
        PathSpec {
            provider_id: "cursor",
            provider_label: "Cursor",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some(
                "Cursor auth storage is not a stable canonical file path in available docs.",
            ),
            path: roots.home.join(".cursor").join("cli-config.json"),
        },
        PathSpec {
            provider_id: "droid",
            provider_label: "Droid",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some(
                "Droid account auth is documented as encrypted/keychain-backed storage.",
            ),
            path: roots.home.join(".factory").join("settings.json"),
        },
        PathSpec {
            provider_id: "gemini",
            provider_label: "Gemini",
            kind: "env_file",
            signal_strength: "strong",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".gemini").join(".env"),
        },
        PathSpec {
            provider_id: "kiro",
            provider_label: "Kiro",
            kind: "auth_file",
            signal_strength: "weak",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots
                .home
                .join(".aws")
                .join("sso")
                .join("cache")
                .join("kiro-auth-token.json"),
        },
        PathSpec {
            provider_id: "gemini",
            provider_label: "Gemini",
            kind: "auth_file",
            signal_strength: "weak",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".gemini").join("oauth_creds.json"),
        },
        PathSpec {
            provider_id: "opencode",
            provider_label: "OpenCode",
            kind: "auth_file",
            signal_strength: "strong",
            confidence: "high",
            importable: true,
            unsupported_reason: None,
            path: roots.xdg_data.join("opencode").join("auth.json"),
        },
        PathSpec {
            provider_id: "qwen",
            provider_label: "Qwen",
            kind: "env_file",
            signal_strength: "strong",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".qwen").join(".env"),
        },
    ];

    let amp_oauth = roots.home.join(".amp").join("oauth");
    if let Ok(entries) = std::fs::read_dir(&amp_oauth) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(OsStr::new("json")) {
                out.push(PathSpec {
                    provider_id: "amp",
                    provider_label: "Amp",
                    kind: "auth_file",
                    signal_strength: "weak",
                    confidence: "low-medium",
                    importable: true,
                    unsupported_reason: None,
                    path,
                });
            }
        }
    }

    out
}

fn file_mtime(path: &Path) -> Option<DateTime<Utc>> {
    let md = std::fs::metadata(path).ok()?;
    let t = md.modified().ok()?;
    Some(DateTime::<Utc>::from(t))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn build_candidate_id(provider_id: &str, kind: &str, path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b"|");
    hasher.update(kind.as_bytes());
    hasher.update(b"|");
    hasher.update(path.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

fn parse_env_file(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let value = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim_matches('"').trim_matches('\'');
        out.insert(key.to_string(), value.to_string());
    }
    out
}

fn summarize_env(
    provider_id: &str,
    env_map: &BTreeMap<String, String>,
) -> (Option<String>, Option<String>) {
    let endpoint = [
        "OPENAI_BASE_URL",
        "BASE_URL",
        "CTX_GATEWAY_BASE_URL",
        "ANTHROPIC_BASE_URL",
    ]
    .into_iter()
    .find_map(|k| env_map.get(k).cloned().filter(|s| !s.trim().is_empty()));

    let key_name = env_map
        .keys()
        .find(|k| k.contains("API_KEY") || k.contains("TOKEN"))
        .cloned();
    let summary = match (provider_id, key_name) {
        ("gemini", Some(name)) => Some(format!("Gemini key from {name}")),
        ("qwen", Some(name)) => Some(format!("Qwen/OpenAI-compatible key from {name}")),
        (_, Some(name)) => Some(format!("Credential from {name}")),
        _ => None,
    };

    (summary, endpoint)
}

fn candidate_from_spec(spec: &PathSpec) -> Option<CandidateMaterial> {
    if !spec.path.exists() {
        return None;
    }

    let id = build_candidate_id(spec.provider_id, spec.kind, &spec.path);
    let mut candidate = ProviderAuthImportCandidate {
        id,
        provider_id: spec.provider_id.to_string(),
        provider_label: spec.provider_label.to_string(),
        kind: spec.kind.to_string(),
        path: spec.path.to_string_lossy().to_string(),
        signal_strength: spec.signal_strength.to_string(),
        confidence: spec.confidence.to_string(),
        parse_status: "detected".to_string(),
        unsupported_reason: spec.unsupported_reason.map(|s| s.to_string()),
        summary: None,
        account_identity: None,
        endpoint: None,
        auth_type: None,
        fingerprint: None,
        last_modified: file_mtime(&spec.path),
    };

    if !spec.importable {
        candidate.parse_status = "unsupported".to_string();
        return Some(CandidateMaterial {
            candidate,
            importable: false,
            secret_bytes: None,
            label: None,
        });
    }

    let bytes = match std::fs::read(&spec.path) {
        Ok(bytes) => bytes,
        Err(err) => {
            candidate.parse_status = "parse_error".to_string();
            candidate.unsupported_reason = Some(format!("failed to read file: {err}"));
            return Some(CandidateMaterial {
                candidate,
                importable: false,
                secret_bytes: None,
                label: None,
            });
        }
    };

    if bytes.is_empty() {
        candidate.parse_status = "parse_error".to_string();
        candidate.unsupported_reason = Some("file is empty".to_string());
        return Some(CandidateMaterial {
            candidate,
            importable: false,
            secret_bytes: None,
            label: None,
        });
    }

    let fingerprint = sha256_hex(&bytes);
    candidate.fingerprint = Some(fingerprint);

    if spec.provider_id == "gemini" && spec.kind == "auth_file" {
        let file_name = spec
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !file_name.eq_ignore_ascii_case("oauth_creds.json") {
            return None;
        }

        let oauth_raw = match String::from_utf8(bytes.clone()) {
            Ok(raw) => raw,
            Err(_) => {
                candidate.parse_status = "parse_error".to_string();
                candidate.unsupported_reason =
                    Some("gemini oauth_creds.json must be UTF-8 JSON".to_string());
                return Some(CandidateMaterial {
                    candidate,
                    importable: false,
                    secret_bytes: None,
                    label: None,
                });
            }
        };

        let oauth_value = match serde_json::from_str::<serde_json::Value>(&oauth_raw) {
            Ok(value) => value,
            Err(err) => {
                candidate.parse_status = "parse_error".to_string();
                candidate.unsupported_reason =
                    Some(format!("gemini oauth_creds.json must be valid JSON: {err}"));
                return Some(CandidateMaterial {
                    candidate,
                    importable: false,
                    secret_bytes: None,
                    label: None,
                });
            }
        };
        if !oauth_value.is_object() {
            candidate.parse_status = "parse_error".to_string();
            candidate.unsupported_reason =
                Some("gemini oauth_creds.json must be a JSON object".to_string());
            return Some(CandidateMaterial {
                candidate,
                importable: false,
                secret_bytes: None,
                label: None,
            });
        }

        let google_accounts_path = spec
            .path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("google_accounts.json");
        if google_accounts_path.exists() {
            let google_raw = match std::fs::read_to_string(&google_accounts_path) {
                Ok(contents) => contents,
                Err(err) => {
                    candidate.parse_status = "parse_error".to_string();
                    candidate.unsupported_reason =
                        Some(format!("failed to read google_accounts.json: {err}"));
                    return Some(CandidateMaterial {
                        candidate,
                        importable: false,
                        secret_bytes: None,
                        label: None,
                    });
                }
            };
            if let Some(raw) = trim_to_option(&google_raw) {
                if let Err(err) = serde_json::from_str::<serde_json::Value>(&raw) {
                    candidate.parse_status = "parse_error".to_string();
                    candidate.unsupported_reason = Some(format!(
                        "gemini google_accounts.json must be valid JSON: {err}"
                    ));
                    return Some(CandidateMaterial {
                        candidate,
                        importable: false,
                        secret_bytes: None,
                        label: None,
                    });
                }
            }
        }

        candidate.summary = summarize_json_candidate(spec.provider_id, &oauth_value);
        candidate.auth_type = Some("subscription".to_string());
        candidate.parse_status = "parsed".to_string();
        return Some(CandidateMaterial {
            candidate,
            importable: true,
            secret_bytes: Some(bytes),
            label: Some(format!("Imported {} profile", spec.provider_label)),
        });
    }

    let label = match spec.kind {
        "env_file" => {
            let raw = String::from_utf8_lossy(&bytes);
            let env_map = parse_env_file(&raw);
            let has_secret = env_map
                .keys()
                .any(|k| k.contains("API_KEY") || k.contains("TOKEN"));
            if !has_secret {
                candidate.parse_status = "unsupported".to_string();
                candidate.unsupported_reason =
                    Some("No API key/token variable found in env file.".to_string());
                return Some(CandidateMaterial {
                    candidate,
                    importable: false,
                    secret_bytes: None,
                    label: None,
                });
            }
            let (summary, endpoint) = summarize_env(spec.provider_id, &env_map);
            candidate.summary = summary;
            candidate.endpoint = endpoint;
            candidate.auth_type = Some("api_key".to_string());
            candidate.parse_status = "parsed".to_string();
            Some(format!("{} API key", spec.provider_label))
        }
        _ => {
            let summary = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(value) => summarize_json_candidate(spec.provider_id, &value),
                Err(_) => None,
            };
            candidate.summary = summary;
            candidate.auth_type = Some(if spec.provider_id == "codex" {
                "subscription".to_string()
            } else {
                "file_auth".to_string()
            });
            candidate.parse_status = "parsed".to_string();
            Some(format!("Imported {} profile", spec.provider_label))
        }
    };

    Some(CandidateMaterial {
        candidate,
        importable: true,
        secret_bytes: Some(bytes),
        label,
    })
}

fn summarize_json_candidate(provider_id: &str, value: &serde_json::Value) -> Option<String> {
    let find_string = |keys: &[&str]| -> Option<String> {
        for key in keys {
            if let Some(v) = value.get(key).and_then(|v| v.as_str()) {
                let t = v.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
        None
    };
    if provider_id == "codex" {
        if let Some(email) = find_string(&["email", "user_email"]) {
            return Some(format!("Codex account {email}"));
        }
        return None;
    }
    if let Some(email) = find_string(&["email", "user_email", "username"]) {
        return Some(email);
    }
    None
}

fn scan_with_roots(roots: &HostRoots) -> Vec<CandidateMaterial> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    for spec in build_catalog(roots) {
        if let Some(candidate) = candidate_from_spec(&spec) {
            if seen.insert(candidate.candidate.id.clone(), ()).is_none() {
                out.push(candidate);
            }
        }
    }
    out.sort_by(|a, b| {
        a.candidate
            .provider_label
            .cmp(&b.candidate.provider_label)
            .then_with(|| a.candidate.path.cmp(&b.candidate.path))
    });
    out
}

pub async fn list_provider_auth_import_candidates() -> Result<Vec<ProviderAuthImportCandidate>> {
    let roots = host_roots()?;
    let candidates = scan_with_roots(&roots)
        .into_iter()
        .map(|c| c.candidate)
        .collect();
    Ok(candidates)
}

pub async fn list_provider_auth_profiles(
    data_root: &Path,
) -> Result<Vec<ProviderImportedAuthProfile>> {
    migrate_legacy_imported_profiles_once(data_root).await?;
    let registry = load_imported_registry(data_root).await;
    Ok(registry.profiles)
}

async fn import_codex_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        return Ok(ProviderAuthImportResult {
            candidate_id: material.candidate.id.clone(),
            provider_id: material.candidate.provider_id.clone(),
            status: "unsupported".to_string(),
            profile_id: None,
            message: Some("Codex candidate has no importable auth material".to_string()),
        });
    };

    let imported_fingerprint = sha256_hex(bytes);
    let imported_auth = serde_json::from_slice::<serde_json::Value>(bytes).ok();
    let mut registry = provider_accounts::load_codex_registry(data_root).await;

    for account in &registry.accounts {
        // Accounts imported via host flow may only have secret_ref and no account-dir auth.json.
        // Hydrate before fingerprint comparison so dedupe catches both storage modes.
        let _ =
            provider_accounts::hydrate_codex_account_home_from_secret(data_root, &account.id).await;
        let auth_path =
            provider_accounts::codex_account_dir(data_root, &account.id).join("auth.json");
        if let Ok(existing) = tokio::fs::read(&auth_path).await {
            let matches_auth = if let Some(imported_auth) = imported_auth.as_ref() {
                serde_json::from_slice::<serde_json::Value>(&existing)
                    .ok()
                    .is_some_and(|existing_auth| existing_auth == *imported_auth)
            } else {
                sha256_hex(&existing) == imported_fingerprint
            };
            if matches_auth {
                return Ok(ProviderAuthImportResult {
                    candidate_id: material.candidate.id.clone(),
                    provider_id: "codex".to_string(),
                    status: "already_imported".to_string(),
                    profile_id: Some(account.id.clone()),
                    message: Some("Matching Codex auth is already imported.".to_string()),
                });
            }
        }
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let account_dir = provider_accounts::ensure_codex_account_dir(data_root, &account_id).await?;
    tokio::fs::write(account_dir.join("auth.json"), bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = tokio::fs::set_permissions(account_dir.join("auth.json"), perms).await;
    }

    let label = material
        .label
        .clone()
        .unwrap_or_else(|| format!("Codex import {}", &account_id[..8]));
    let kind = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            let has_tokens =
                value
                    .get("tokens")
                    .and_then(|v| v.as_object())
                    .is_some_and(|tokens| {
                        tokens
                            .get("access_token")
                            .and_then(|v| v.as_str())
                            .is_some_and(|v| !v.trim().is_empty())
                            && tokens
                                .get("refresh_token")
                                .and_then(|v| v.as_str())
                                .is_some_and(|v| !v.trim().is_empty())
                    });
            if has_tokens {
                return Some(provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string());
            }
            let has_api_key = value
                .get("OPENAI_API_KEY")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.trim().is_empty());
            if has_api_key {
                return Some(provider_accounts::CODEX_CREDENTIAL_KIND_API_KEY.to_string());
            }
            None
        })
        .unwrap_or_else(|| provider_accounts::CODEX_CREDENTIAL_KIND_API_KEY.to_string());

    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.clone(),
        label,
        kind,
        email: None,
        plan_type: None,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: None,
        endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
    };
    if let Some(existing) = registry.accounts.iter_mut().find(|a| a.id == entry.id) {
        *existing = entry;
    } else {
        registry.accounts.push(entry);
    }
    if registry.active_account_id.is_none() {
        registry.active_account_id = Some(account_id.clone());
    }
    provider_accounts::save_codex_registry(data_root, &registry).await?;
    let _ = harness_sources::set_provider_source_selection(
        data_root,
        "codex",
        harness_sources::HarnessSourceKind::Subscription,
        None,
    )
    .await?;

    Ok(ProviderAuthImportResult {
        candidate_id: material.candidate.id.clone(),
        provider_id: "codex".to_string(),
        status: "imported".to_string(),
        profile_id: Some(account_id),
        message: Some("Codex auth imported and available for new turns.".to_string()),
    })
}

fn trim_to_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_json_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>()
}

fn json_key_matches(actual: &str, expected: &str) -> bool {
    normalize_json_key(actual) == normalize_json_key(expected)
}

fn find_json_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for expected in keys {
                for (actual_key, actual_value) in map {
                    if !json_key_matches(actual_key, expected) {
                        continue;
                    }
                    if let Some(value) = actual_value.as_str().and_then(trim_to_option) {
                        return Some(value);
                    }
                }
            }
            for nested in map.values() {
                if let Some(value) = find_json_string_by_keys(nested, keys) {
                    return Some(value);
                }
            }
            None
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| find_json_string_by_keys(item, keys)),
        _ => None,
    }
}

fn env_value_case_insensitive(env_map: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = env_map
            .iter()
            .find_map(|(actual, value)| actual.eq_ignore_ascii_case(key).then_some(value))
            .and_then(|value| trim_to_option(value))
        {
            return Some(value);
        }
    }
    None
}

fn default_endpoint_base_url_for_provider(provider_id: &str) -> Option<String> {
    match provider_id {
        "qwen" | "opencode" => Some(DEFAULT_OPENROUTER_BASE_URL.to_string()),
        _ => None,
    }
}

fn parse_endpoint_env_candidate(
    provider_id: &str,
    material: &CandidateMaterial,
    default_keys: &[&str],
) -> Result<(String, Option<String>)> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        anyhow::bail!("No importable auth material.");
    };
    let env_map = parse_env_file(&String::from_utf8_lossy(bytes));
    let api_key = env_value_case_insensitive(&env_map, default_keys)
        .or_else(|| {
            env_map
                .iter()
                .find(|(key, _)| {
                    key.to_ascii_uppercase().contains("API_KEY")
                        || key.to_ascii_uppercase().contains("TOKEN")
                })
                .and_then(|(_, value)| trim_to_option(value))
        })
        .ok_or_else(|| anyhow::anyhow!("No API key/token variable found in env file."))?;

    let base_url = material
        .candidate
        .endpoint
        .as_deref()
        .and_then(trim_to_option)
        .or_else(|| {
            env_value_case_insensitive(
                &env_map,
                &["OPENAI_BASE_URL", "BASE_URL", "CTX_GATEWAY_BASE_URL"],
            )
        })
        .or_else(|| default_endpoint_base_url_for_provider(provider_id));

    Ok((api_key, base_url))
}

fn parse_endpoint_json_candidate(
    provider_id: &str,
    material: &CandidateMaterial,
) -> Result<(String, Option<String>, Option<String>)> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        anyhow::bail!("No importable auth material.");
    };
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("Auth file must be valid JSON.")?;
    let api_key = find_json_string_by_keys(
        &value,
        &[
            "api_key",
            "apiKey",
            "token",
            "auth_token",
            "authToken",
            "openai_api_key",
            "openrouter_api_key",
            "anthropic_api_key",
        ],
    )
    .ok_or_else(|| anyhow::anyhow!("No API key/token field found in auth file."))?;
    let base_url = find_json_string_by_keys(
        &value,
        &[
            "base_url",
            "baseURL",
            "url",
            "endpoint",
            "openai_base_url",
            "openrouter_base_url",
        ],
    )
    .or_else(|| default_endpoint_base_url_for_provider(provider_id));
    let model_override = find_json_string_by_keys(&value, &["model", "model_name", "openai_model"]);
    Ok((api_key, base_url, model_override))
}

async fn set_subscription_source_if_supported(data_root: &Path, provider_id: &str) -> Result<()> {
    let should_set = matches!(
        provider_id,
        "codex"
            | "gemini"
            | "kimi"
            | "qwen"
            | "opencode"
            | "mistral"
            | "goose"
            | "cagent"
            | "amp"
            | "droid"
            | "cody"
            | "continue"
            | "cline"
            | "swe-agent"
            | "openhands"
            | "copilot"
            | "kiro"
            | "rovo"
            | "auggie"
            | "pi"
    );
    if should_set {
        let _ = harness_sources::set_provider_source_selection(
            data_root,
            provider_id,
            harness_sources::HarnessSourceKind::Subscription,
            None,
        )
        .await?;
    }
    Ok(())
}

fn import_result(
    material: &CandidateMaterial,
    status: &str,
    profile_id: Option<String>,
    message: Option<String>,
) -> ProviderAuthImportResult {
    ProviderAuthImportResult {
        candidate_id: material.candidate.id.clone(),
        provider_id: material.candidate.provider_id.clone(),
        status: status.to_string(),
        profile_id,
        message,
    }
}

async fn import_endpoint_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
    provider_id: &str,
    api_key: String,
    base_url: Option<String>,
    auth_type: Option<String>,
    model_override: Option<String>,
) -> Result<ProviderAuthImportResult> {
    let api_shape = harness_sources::default_shape_for_provider(provider_id)
        .ok_or_else(|| anyhow::anyhow!("provider does not support endpoint auth import"))?;
    let match_state = harness_sources::find_provider_endpoint_import_match(
        data_root,
        provider_id,
        base_url.clone(),
        api_shape,
        auth_type.clone(),
        model_override.clone(),
        &api_key,
    )
    .await?;

    if let Some(found) = match_state.as_ref() {
        if found.kind == harness_sources::HarnessEndpointImportMatchKind::ExactCredentials {
            let _ = harness_sources::set_provider_source_selection(
                data_root,
                provider_id,
                harness_sources::HarnessSourceKind::Endpoint,
                Some(found.endpoint_id.clone()),
            )
            .await?;
            return Ok(import_result(
                material,
                "already_imported",
                Some(found.endpoint_id.clone()),
                Some("Matching endpoint credential already imported.".to_string()),
            ));
        }
    }

    let endpoint = harness_sources::upsert_provider_endpoint(
        data_root,
        provider_id,
        harness_sources::HarnessEndpointUpsert {
            endpoint_id: match_state.as_ref().map(|found| found.endpoint_id.clone()),
            name: material.label.clone().unwrap_or_else(|| {
                format!("{} imported endpoint", material.candidate.provider_label)
            }),
            base_url,
            api_shape: Some(api_shape),
            auth_type,
            model_override,
            api_key: Some(api_key),
        },
    )
    .await?;

    let _ = harness_sources::set_provider_source_selection(
        data_root,
        provider_id,
        harness_sources::HarnessSourceKind::Endpoint,
        Some(endpoint.id.clone()),
    )
    .await?;

    let status = match match_state {
        Some(found)
            if found.kind == harness_sources::HarnessEndpointImportMatchKind::SameConfig =>
        {
            "updated"
        }
        _ => "imported",
    };

    Ok(import_result(
        material,
        status,
        Some(endpoint.id),
        Some(if status == "updated" {
            "Endpoint credential updated.".to_string()
        } else {
            "Endpoint credential imported.".to_string()
        }),
    ))
}

async fn import_gemini_auth_file_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        return Ok(import_result(
            material,
            "unsupported",
            None,
            Some("No importable auth material.".to_string()),
        ));
    };

    let path = Path::new(&material.candidate.path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    let oauth_creds_json = if file_name.eq_ignore_ascii_case("google_accounts.json") {
        let oauth_path = parent.join("oauth_creds.json");
        match tokio::fs::read_to_string(&oauth_path).await {
            Ok(contents) => trim_to_option(&contents),
            Err(_) => None,
        }
        .ok_or_else(|| anyhow::anyhow!("google_accounts.json requires sibling oauth_creds.json"))?
    } else {
        String::from_utf8(bytes.to_vec()).context("gemini auth file must be UTF-8 JSON")?
    };

    let google_accounts_json = if file_name.eq_ignore_ascii_case("google_accounts.json") {
        Some(String::from_utf8(bytes.to_vec()).context("google_accounts.json must be UTF-8 JSON")?)
    } else {
        let google_path = parent.join("google_accounts.json");
        tokio::fs::read_to_string(&google_path)
            .await
            .ok()
            .and_then(|raw| trim_to_option(&raw))
    };

    let before_len = provider_accounts::load_gemini_registry(data_root)
        .await
        .accounts
        .len();
    let registry = provider_accounts::add_gemini_account(
        data_root,
        material.label.clone(),
        oauth_creds_json,
        google_accounts_json,
        None,
    )
    .await?;
    let imported = registry.accounts.len() > before_len;
    if imported {
        set_subscription_source_if_supported(data_root, "gemini").await?;
    }
    Ok(import_result(
        material,
        if imported {
            "imported"
        } else {
            "already_imported"
        },
        registry.active_account_id,
        Some(if imported {
            "Gemini OAuth auth imported.".to_string()
        } else {
            "Matching Gemini OAuth auth is already imported.".to_string()
        }),
    ))
}

async fn import_gemini_env_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let (api_key, base_url, auth_type) = {
        let Some(bytes) = material.secret_bytes.as_ref() else {
            anyhow::bail!("No importable auth material.");
        };
        let env_map = parse_env_file(&String::from_utf8_lossy(bytes));
        if let Some(key) = env_value_case_insensitive(&env_map, &["GOOGLE_API_KEY"]) {
            (key, None, Some("vertex_ai".to_string()))
        } else {
            let (key, url) = parse_endpoint_env_candidate(
                "gemini",
                material,
                &["GEMINI_API_KEY", "OPENAI_API_KEY"],
            )?;
            let auth_type = if url.is_some() {
                // Preserve legacy OpenAI-compatible Gemini endpoint imports when a base URL is present.
                Some("bearer".to_string())
            } else {
                Some("gemini_api_key".to_string())
            };
            (key, url, auth_type)
        }
    };
    import_endpoint_candidate(
        data_root, material, "gemini", api_key, base_url, auth_type, None,
    )
    .await
}

async fn import_qwen_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let (api_key, base_url) = parse_endpoint_env_candidate(
        "qwen",
        material,
        &["QWEN_API_KEY", "DASHSCOPE_API_KEY", "OPENAI_API_KEY"],
    )?;
    import_endpoint_candidate(data_root, material, "qwen", api_key, base_url, None, None).await
}

async fn import_opencode_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let (api_key, base_url, model_override) = parse_endpoint_json_candidate("opencode", material)?;
    import_endpoint_candidate(
        data_root,
        material,
        "opencode",
        api_key,
        base_url,
        None,
        model_override,
    )
    .await
}

async fn import_amp_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let (api_key, base_url, model_override) = parse_endpoint_json_candidate("amp", material)?;
    import_endpoint_candidate(
        data_root,
        material,
        "amp",
        api_key,
        base_url,
        None,
        model_override,
    )
    .await
}

async fn import_kiro_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        return Ok(import_result(
            material,
            "unsupported",
            None,
            Some("No importable auth material.".to_string()),
        ));
    };
    let auth_token_json =
        String::from_utf8(bytes.to_vec()).context("kiro auth file must be UTF-8 JSON")?;
    let before_len = provider_accounts::load_kiro_registry(data_root)
        .await
        .accounts
        .len();
    let registry = provider_accounts::add_kiro_account(
        data_root,
        material.label.clone(),
        auth_token_json,
        None,
    )
    .await?;
    let imported = registry.accounts.len() > before_len;
    if imported {
        set_subscription_source_if_supported(data_root, "kiro").await?;
    }
    Ok(import_result(
        material,
        if imported {
            "imported"
        } else {
            "already_imported"
        },
        registry.active_account_id,
        Some(if imported {
            "Kiro auth imported.".to_string()
        } else {
            "Matching Kiro auth is already imported.".to_string()
        }),
    ))
}

async fn import_candidate_to_canonical(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    if !material.importable || material.secret_bytes.is_none() {
        return Ok(import_result(
            material,
            "unsupported",
            None,
            material
                .candidate
                .unsupported_reason
                .clone()
                .or_else(|| Some("No importable auth material.".to_string())),
        ));
    }
    match material.candidate.provider_id.as_str() {
        "codex" => import_codex_candidate(data_root, material).await,
        "gemini" => {
            if material.candidate.kind == "env_file" {
                import_gemini_env_candidate(data_root, material).await
            } else {
                import_gemini_auth_file_candidate(data_root, material).await
            }
        }
        "qwen" => import_qwen_candidate(data_root, material).await,
        "opencode" => import_opencode_candidate(data_root, material).await,
        "amp" => import_amp_candidate(data_root, material).await,
        "kiro" => import_kiro_candidate(data_root, material).await,
        _ => Ok(import_result(
            material,
            "unsupported",
            None,
            Some(format!(
                "Provider '{}' import is not wired into canonical auth storage yet.",
                material.candidate.provider_id
            )),
        )),
    }
}

async fn read_legacy_secret_material_bytes(data_root: &Path, profile_id: &str) -> Option<Vec<u8>> {
    let payload = tokio::fs::read_to_string(imported_secret_path(data_root, profile_id))
        .await
        .ok()?;
    let parsed = serde_json::from_str::<StoredSecretMaterial>(&payload).ok()?;
    let content = parsed.content_b64?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content).ok()
}

async fn migrate_legacy_imported_profiles_once(data_root: &Path) -> Result<()> {
    if legacy_migration_marker_exists(data_root).await {
        return Ok(());
    }

    let mut registry = load_imported_registry(data_root).await;
    if registry.profiles.is_empty() {
        write_legacy_migration_marker(data_root).await?;
        return Ok(());
    }

    // Decision: migration must be lossless. Keep any legacy profile that cannot be migrated yet
    // (missing secret material, unsupported provider, transient failure) so future runs can retry.
    let mut remaining_profiles: Vec<ProviderImportedAuthProfile> = Vec::new();
    for profile in registry.profiles.iter().cloned() {
        let Some(secret_bytes) = read_legacy_secret_material_bytes(data_root, &profile.id).await
        else {
            remaining_profiles.push(profile);
            continue;
        };
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: profile.id.clone(),
                provider_id: profile.provider_id.clone(),
                provider_label: profile.provider_label.clone(),
                kind: profile.source_kind.clone(),
                path: profile.source_path.clone(),
                signal_strength: "legacy".to_string(),
                confidence: "legacy".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: profile.account_identity.clone(),
                endpoint: profile.endpoint.clone(),
                auth_type: profile.auth_type.clone(),
                fingerprint: Some(profile.secret_fingerprint.clone()),
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(secret_bytes),
            label: Some(profile.label.clone()),
        };
        let migrated = match import_candidate_to_canonical(data_root, &material).await {
            Ok(result) => matches!(
                result.status.as_str(),
                "imported" | "updated" | "already_imported"
            ),
            Err(_) => false,
        };
        if migrated {
            let _ = tokio::fs::remove_file(imported_secret_path(data_root, &profile.id)).await;
        } else {
            remaining_profiles.push(profile);
        }
    }

    registry.profiles = remaining_profiles;
    save_imported_registry(data_root, &registry).await?;
    if registry.profiles.is_empty() {
        let _ = tokio::fs::remove_dir_all(imported_secrets_dir(data_root)).await;
        write_legacy_migration_marker(data_root).await?;
    }
    Ok(())
}

pub async fn import_provider_auth_candidates(
    data_root: &Path,
    candidate_ids: &[String],
) -> Result<Vec<ProviderAuthImportResult>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    migrate_legacy_imported_profiles_once(data_root).await?;

    let roots = host_roots()?;
    let materials = scan_with_roots(&roots);
    let mut by_id: HashMap<String, CandidateMaterial> = HashMap::new();
    for material in materials {
        by_id.insert(material.candidate.id.clone(), material);
    }

    let mut results = Vec::new();

    for candidate_id in candidate_ids {
        let Some(material) = by_id.get(candidate_id) else {
            results.push(ProviderAuthImportResult {
                candidate_id: candidate_id.clone(),
                provider_id: "unknown".to_string(),
                status: "error".to_string(),
                profile_id: None,
                message: Some("Candidate no longer available; re-scan and retry.".to_string()),
            });
            continue;
        };

        if !material.importable {
            results.push(ProviderAuthImportResult {
                candidate_id: material.candidate.id.clone(),
                provider_id: material.candidate.provider_id.clone(),
                status: "unsupported".to_string(),
                profile_id: None,
                message: material
                    .candidate
                    .unsupported_reason
                    .clone()
                    .or_else(|| Some("Candidate cannot be imported automatically.".to_string())),
            });
            continue;
        }

        match import_candidate_to_canonical(data_root, material).await {
            Ok(result) => results.push(result),
            Err(error) => results.push(ProviderAuthImportResult {
                candidate_id: material.candidate.id.clone(),
                provider_id: material.candidate.provider_id.clone(),
                status: "error".to_string(),
                profile_id: None,
                message: Some(error.to_string()),
            }),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    async fn write_legacy_secret_material(
        data_root: &Path,
        profile_id: &str,
        source_path: &str,
        bytes: &[u8],
    ) {
        let payload = StoredSecretMaterial {
            kind: "auth_file".to_string(),
            source_path: source_path.to_string(),
            content_b64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        };
        let path = imported_secret_path(data_root, profile_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(path, serde_json::to_vec_pretty(&payload).unwrap())
            .await
            .unwrap();
    }

    fn test_roots(base: &Path) -> HostRoots {
        HostRoots {
            home: base.join("home"),
            xdg_config: base.join("home").join(".config"),
            xdg_data: base.join("home").join(".local").join("share"),
            codex_home: base.join("home").join(".codex"),
        }
    }

    #[test]
    fn env_parser_extracts_keys() {
        let parsed = parse_env_file(
            r#"
            # comment
            OPENAI_API_KEY=sk-test
            OPENAI_BASE_URL=https://api.example.com/v1
            "#,
        );
        assert_eq!(parsed.get("OPENAI_API_KEY"), Some(&"sk-test".to_string()));
        assert_eq!(
            parsed.get("OPENAI_BASE_URL"),
            Some(&"https://api.example.com/v1".to_string())
        );
    }

    #[test]
    fn summarize_env_does_not_fill_endpoint_with_auth_type() {
        let env = BTreeMap::from([("OPENAI_API_KEY".to_string(), "sk-test".to_string())]);
        let (_summary, endpoint) = summarize_env("qwen", &env);
        assert_eq!(endpoint, None);
    }

    #[tokio::test]
    async fn legacy_migration_keeps_unmigrated_profiles_without_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let now = Utc::now();

        let codex_profile = ProviderImportedAuthProfile {
            id: "legacy-codex".to_string(),
            provider_id: "codex".to_string(),
            provider_label: "Codex".to_string(),
            label: "Legacy Codex".to_string(),
            account_identity: None,
            endpoint: None,
            auth_type: Some("subscription".to_string()),
            source_path: "/tmp/.codex/auth.json".to_string(),
            source_kind: "auth_file".to_string(),
            secret_fingerprint: "fp-codex".to_string(),
            imported_at: now,
            updated_at: now,
        };
        let unsupported_profile = ProviderImportedAuthProfile {
            id: "legacy-cursor".to_string(),
            provider_id: "cursor".to_string(),
            provider_label: "Cursor".to_string(),
            label: "Legacy Cursor".to_string(),
            account_identity: None,
            endpoint: None,
            auth_type: Some("subscription".to_string()),
            source_path: "/tmp/.cursor/auth.json".to_string(),
            source_kind: "auth_file".to_string(),
            secret_fingerprint: "fp-cursor".to_string(),
            imported_at: now,
            updated_at: now,
        };

        save_imported_registry(
            root,
            &ProviderImportedAuthRegistry {
                profiles: vec![codex_profile.clone(), unsupported_profile.clone()],
            },
        )
        .await
        .unwrap();

        write_legacy_secret_material(
            root,
            &codex_profile.id,
            &codex_profile.source_path,
            br#"{"OPENAI_API_KEY":"sk-legacy"}"#,
        )
        .await;
        write_legacy_secret_material(
            root,
            &unsupported_profile.id,
            &unsupported_profile.source_path,
            br#"{"token":"cursor-legacy"}"#,
        )
        .await;

        migrate_legacy_imported_profiles_once(root).await.unwrap();

        let registry = load_imported_registry(root).await;
        assert_eq!(registry.profiles.len(), 1);
        assert_eq!(registry.profiles[0].id, unsupported_profile.id);

        assert!(tokio::fs::metadata(legacy_migration_marker_path(root))
            .await
            .is_err());
        assert!(
            tokio::fs::metadata(imported_secret_path(root, &codex_profile.id))
                .await
                .is_err()
        );
        assert!(
            tokio::fs::metadata(imported_secret_path(root, &unsupported_profile.id))
                .await
                .is_ok()
        );

        let codex_registry = provider_accounts::load_codex_registry(root).await;
        assert_eq!(codex_registry.accounts.len(), 1);
    }

    #[tokio::test]
    async fn codex_import_dedupes_secret_backed_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let account_id = "acct-1";
        let auth_bytes = br#"{"OPENAI_API_KEY":"sk-test"}"#;
        let account_dir = provider_accounts::ensure_codex_account_dir(root, account_id)
            .await
            .unwrap();
        tokio::fs::write(account_dir.join("auth.json"), auth_bytes)
            .await
            .unwrap();
        provider_accounts::save_codex_registry(
            root,
            &provider_accounts::CodexAccountRegistry {
                active_account_id: Some(account_id.to_string()),
                accounts: vec![provider_accounts::CodexAccountEntry {
                    id: account_id.to_string(),
                    label: "Test".to_string(),
                    kind: provider_accounts::CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
                    email: None,
                    plan_type: None,
                    created_at: Utc::now(),
                    last_used_at: Some(Utc::now()),
                    secret_ref: None,
                    endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
                }],
            },
        )
        .await
        .unwrap();
        provider_accounts::ingest_codex_account_auth_to_secret_store(root, account_id)
            .await
            .unwrap();
        tokio::fs::remove_file(account_dir.join("auth.json"))
            .await
            .unwrap();

        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "codex-candidate".to_string(),
                provider_id: "codex".to_string(),
                provider_label: "Codex".to_string(),
                kind: "json_file".to_string(),
                path: "/tmp/.codex/auth.json".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: None,
                auth_type: Some("subscription".to_string()),
                fingerprint: Some(sha256_hex(auth_bytes)),
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(auth_bytes.to_vec()),
            label: Some("Imported Codex profile".to_string()),
        };

        let result = import_codex_candidate(root, &material).await.unwrap();
        assert_eq!(result.status, "already_imported");
        assert_eq!(result.profile_id.as_deref(), Some(account_id));
    }

    #[test]
    fn scan_detects_codex_auth_file() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(dir.path());
        std::fs::create_dir_all(&roots.codex_home).unwrap();
        std::fs::write(roots.codex_home.join("auth.json"), br#"{"ok":true}"#).unwrap();

        let found = scan_with_roots(&roots)
            .into_iter()
            .find(|c| c.candidate.provider_id == "codex")
            .expect("codex candidate");
        assert_eq!(found.candidate.parse_status, "parsed");
        assert!(found.importable);
    }

    #[test]
    fn scan_detects_only_valid_gemini_oauth_profile() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(dir.path());
        let gemini_dir = roots.home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join("oauth_creds.json"),
            br#"{"access_token":"a","refresh_token":"b"}"#,
        )
        .unwrap();
        std::fs::write(
            gemini_dir.join("google_accounts.json"),
            br#"[{"email":"dev@example.com"}]"#,
        )
        .unwrap();

        let scanned = scan_with_roots(&roots);
        let gemini_oauth = scanned
            .iter()
            .find(|c| c.candidate.path.ends_with("/.gemini/oauth_creds.json"))
            .expect("gemini oauth candidate");
        assert!(gemini_oauth.importable);
        assert_eq!(gemini_oauth.candidate.parse_status, "parsed");
        assert_eq!(
            gemini_oauth.candidate.auth_type.as_deref(),
            Some("subscription")
        );
        assert!(scanned
            .iter()
            .all(|c| !c.candidate.path.ends_with("/.gemini/google_accounts.json")));
    }

    #[test]
    fn scan_rejects_gemini_oauth_when_google_accounts_sidecar_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(dir.path());
        let gemini_dir = roots.home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join("oauth_creds.json"),
            br#"{"access_token":"a","refresh_token":"b"}"#,
        )
        .unwrap();
        std::fs::write(gemini_dir.join("google_accounts.json"), b"not-json").unwrap();

        let scanned = scan_with_roots(&roots);
        let gemini_oauth = scanned
            .iter()
            .find(|c| c.candidate.path.ends_with("/.gemini/oauth_creds.json"))
            .expect("gemini oauth candidate");
        assert!(!gemini_oauth.importable);
        assert_eq!(gemini_oauth.candidate.parse_status, "parse_error");
        assert!(gemini_oauth
            .candidate
            .unsupported_reason
            .as_deref()
            .unwrap_or_default()
            .contains("google_accounts.json"));
    }

    #[test]
    fn scan_ignores_standalone_gemini_google_accounts_file() {
        let dir = tempfile::tempdir().unwrap();
        let roots = test_roots(dir.path());
        let gemini_dir = roots.home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join("google_accounts.json"),
            br#"[{"email":"dev@example.com"}]"#,
        )
        .unwrap();

        let scanned = scan_with_roots(&roots);
        assert!(scanned
            .iter()
            .all(|c| !c.candidate.provider_id.eq("gemini")));
    }

    #[tokio::test]
    async fn gemini_oauth_candidate_import_writes_canonical_account_registry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "gemini-oauth-candidate".to_string(),
                provider_id: "gemini".to_string(),
                provider_label: "Gemini".to_string(),
                kind: "auth_file".to_string(),
                path: "/tmp/.gemini/oauth_creds.json".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: None,
                auth_type: Some("subscription".to_string()),
                fingerprint: None,
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(br#"{"access_token":"a","refresh_token":"b"}"#.to_vec()),
            label: Some("Gemini import".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "imported");
        let registry = provider_accounts::load_gemini_registry(root).await;
        assert_eq!(registry.accounts.len(), 1);
        assert_eq!(registry.active_account_id, result.profile_id);
    }

    #[tokio::test]
    async fn gemini_env_candidate_with_base_url_imports_legacy_bearer_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let base_url = "https://generativelanguage.googleapis.com/v1beta/openai";
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "gemini-env-legacy".to_string(),
                provider_id: "gemini".to_string(),
                provider_label: "Gemini".to_string(),
                kind: "env_file".to_string(),
                path: "/tmp/.gemini/.env".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: Some(base_url.to_string()),
                auth_type: Some("api_key".to_string()),
                fingerprint: None,
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(
                format!("OPENAI_API_KEY=key-legacy\nOPENAI_BASE_URL={base_url}\n").into_bytes(),
            ),
            label: Some("Gemini legacy endpoint".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "imported");
        let config = harness_sources::get_provider_source_config(root, "gemini")
            .await
            .unwrap();
        assert_eq!(
            config.selected_source_kind,
            harness_sources::HarnessSourceKind::Endpoint
        );
        let selected_id = config
            .selected_endpoint_id
            .as_deref()
            .expect("selected endpoint id");
        let endpoint = config
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == selected_id)
            .expect("selected endpoint");
        assert_eq!(endpoint.auth_type, "bearer");
        assert_eq!(endpoint.base_url.as_deref(), Some(base_url));
    }

    #[tokio::test]
    async fn gemini_env_candidate_without_base_url_imports_native_key_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "gemini-env-native".to_string(),
                provider_id: "gemini".to_string(),
                provider_label: "Gemini".to_string(),
                kind: "env_file".to_string(),
                path: "/tmp/.gemini/.env".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: None,
                auth_type: Some("api_key".to_string()),
                fingerprint: None,
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(b"GEMINI_API_KEY=key-native\n".to_vec()),
            label: Some("Gemini native endpoint".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "imported");
        let config = harness_sources::get_provider_source_config(root, "gemini")
            .await
            .unwrap();
        assert_eq!(
            config.selected_source_kind,
            harness_sources::HarnessSourceKind::Endpoint
        );
        let selected_id = config
            .selected_endpoint_id
            .as_deref()
            .expect("selected endpoint id");
        let endpoint = config
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == selected_id)
            .expect("selected endpoint");
        assert_eq!(endpoint.auth_type, "gemini_api_key");
        assert!(endpoint.base_url.is_none());
    }

    #[tokio::test]
    async fn qwen_env_candidate_import_updates_endpoint_store_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "qwen-candidate".to_string(),
                provider_id: "qwen".to_string(),
                provider_label: "Qwen".to_string(),
                kind: "env_file".to_string(),
                path: "/tmp/.qwen/.env".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: Some("https://api.example.com/v1".to_string()),
                auth_type: Some("api_key".to_string()),
                fingerprint: None,
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(
                b"OPENAI_API_KEY=key-1\nOPENAI_BASE_URL=https://api.example.com/v1".to_vec(),
            ),
            label: Some("Qwen endpoint".to_string()),
        };

        let first = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(first.status, "imported");
        let config = harness_sources::get_provider_source_config(root, "qwen")
            .await
            .unwrap();
        assert_eq!(
            config.selected_source_kind,
            harness_sources::HarnessSourceKind::Endpoint
        );
        assert_eq!(config.endpoints.len(), 1);

        let second = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(second.status, "already_imported");
        let config = harness_sources::get_provider_source_config(root, "qwen")
            .await
            .unwrap();
        assert_eq!(config.endpoints.len(), 1);

        let updated_material = CandidateMaterial {
            secret_bytes: Some(
                b"OPENAI_API_KEY=key-2\nOPENAI_BASE_URL=https://api.example.com/v1".to_vec(),
            ),
            ..material
        };
        let updated = import_candidate_to_canonical(root, &updated_material)
            .await
            .unwrap();
        assert_eq!(updated.status, "updated");
        let config = harness_sources::get_provider_source_config(root, "qwen")
            .await
            .unwrap();
        assert_eq!(config.endpoints.len(), 1);
    }

    #[tokio::test]
    async fn qwen_exact_match_import_activates_existing_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let existing = harness_sources::upsert_provider_endpoint(
            root,
            "qwen",
            harness_sources::HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Existing Qwen endpoint".to_string(),
                base_url: Some("https://api.example.com/v1".to_string()),
                api_shape: harness_sources::default_shape_for_provider("qwen"),
                auth_type: None,
                model_override: None,
                api_key: Some("key-1".to_string()),
            },
        )
        .await
        .unwrap();
        let _ = harness_sources::set_provider_source_selection(
            root,
            "qwen",
            harness_sources::HarnessSourceKind::Subscription,
            None,
        )
        .await
        .unwrap();

        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "qwen-candidate-exact".to_string(),
                provider_id: "qwen".to_string(),
                provider_label: "Qwen".to_string(),
                kind: "env_file".to_string(),
                path: "/tmp/.qwen/.env".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "high".to_string(),
                parse_status: "parsed".to_string(),
                unsupported_reason: None,
                summary: None,
                account_identity: None,
                endpoint: Some("https://api.example.com/v1".to_string()),
                auth_type: Some("api_key".to_string()),
                fingerprint: None,
                last_modified: None,
            },
            importable: true,
            secret_bytes: Some(
                b"OPENAI_API_KEY=key-1\nOPENAI_BASE_URL=https://api.example.com/v1".to_vec(),
            ),
            label: Some("Qwen endpoint".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "already_imported");
        assert_eq!(result.profile_id.as_deref(), Some(existing.id.as_str()));

        let config = harness_sources::get_provider_source_config(root, "qwen")
            .await
            .unwrap();
        assert_eq!(
            config.selected_source_kind,
            harness_sources::HarnessSourceKind::Endpoint
        );
        assert_eq!(
            config.selected_endpoint_id.as_deref(),
            Some(existing.id.as_str())
        );
    }
}
