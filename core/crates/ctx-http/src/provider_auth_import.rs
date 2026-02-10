use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::provider_accounts;

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
            provider_id: "claude",
            provider_label: "Claude",
            kind: "auth_file",
            signal_strength: "strong",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".claude.json"),
        },
        PathSpec {
            provider_id: "claude",
            provider_label: "Claude",
            kind: "auth_file",
            signal_strength: "strong",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.xdg_config.join("claude-code").join("auth.json"),
        },
        PathSpec {
            provider_id: "claude",
            provider_label: "Claude",
            kind: "auth_file",
            signal_strength: "weak",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".claude").join(".credentials.json"),
        },
        PathSpec {
            provider_id: "amp",
            provider_label: "Amp",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some("Amp auth is often keychain-backed; no canonical importable auth file was found."),
            path: roots.xdg_config.join("amp").join("settings.json"),
        },
        PathSpec {
            provider_id: "copilot",
            provider_label: "Copilot",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some("Copilot auth storage is not a stable canonical file path in available docs."),
            path: roots.home.join(".copilot").join("lsp-config.json"),
        },
        PathSpec {
            provider_id: "cursor",
            provider_label: "Cursor",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some("Cursor credentials are documented as secure local storage without a canonical import file path."),
            path: roots.home.join(".cursor").join("cli-config.json"),
        },
        PathSpec {
            provider_id: "droid",
            provider_label: "Droid",
            kind: "config_file",
            signal_strength: "weak",
            confidence: "low-medium",
            importable: false,
            unsupported_reason: Some("Droid account auth is documented as encrypted/keychain-backed storage."),
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
            provider_id: "gemini",
            provider_label: "Gemini",
            kind: "auth_file",
            signal_strength: "weak",
            confidence: "medium",
            importable: true,
            unsupported_reason: None,
            path: roots.home.join(".gemini").join("google_accounts.json"),
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
    format!("{}", hex::encode(hasher.finalize()))
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

    let auth_type = if env_map
        .keys()
        .any(|k| k.contains("API_KEY") || k.contains("TOKEN"))
    {
        Some("api_key".to_string())
    } else {
        None
    };

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

    (summary, endpoint.or(auth_type.clone()))
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
            let (summary, endpoint_or_auth) = summarize_env(spec.provider_id, &env_map);
            candidate.summary = summary;
            candidate.endpoint = endpoint_or_auth.clone();
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
        return Some("Codex auth session".to_string());
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
    let registry = load_imported_registry(data_root).await;
    Ok(registry.profiles)
}

async fn write_secret_material(
    data_root: &Path,
    profile_id: &str,
    source_path: &str,
    source_kind: &str,
    bytes: &[u8],
) -> Result<()> {
    let dir = imported_secrets_dir(data_root);
    tokio::fs::create_dir_all(&dir).await?;
    let payload = StoredSecretMaterial {
        kind: source_kind.to_string(),
        source_path: source_path.to_string(),
        content_b64: Some(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            bytes,
        )),
    };
    let path = imported_secret_path(data_root, profile_id);
    tokio::fs::write(path, serde_json::to_vec_pretty(&payload)?).await?;
    Ok(())
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
    let mut registry = provider_accounts::load_codex_registry(data_root).await;

    for account in &registry.accounts {
        let auth_path =
            provider_accounts::codex_account_dir(data_root, &account.id).join("auth.json");
        if let Ok(existing) = tokio::fs::read(&auth_path).await {
            if sha256_hex(&existing) == imported_fingerprint {
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

    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.clone(),
        label,
        email: None,
        plan_type: None,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
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

    Ok(ProviderAuthImportResult {
        candidate_id: material.candidate.id.clone(),
        provider_id: "codex".to_string(),
        status: "imported".to_string(),
        profile_id: Some(account_id),
        message: Some("Codex auth imported and available for new turns.".to_string()),
    })
}

async fn import_generic_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
    registry: &mut ProviderImportedAuthRegistry,
) -> Result<ProviderAuthImportResult> {
    let Some(bytes) = material.secret_bytes.as_ref() else {
        return Ok(ProviderAuthImportResult {
            candidate_id: material.candidate.id.clone(),
            provider_id: material.candidate.provider_id.clone(),
            status: "unsupported".to_string(),
            profile_id: None,
            message: Some("No importable auth material.".to_string()),
        });
    };

    let fingerprint = sha256_hex(bytes);
    let provider_id = material.candidate.provider_id.clone();
    let account_identity = material.candidate.account_identity.clone();
    let endpoint = material.candidate.endpoint.clone();
    let auth_type = material.candidate.auth_type.clone();

    if let Some(existing) = registry.profiles.iter().find(|p| {
        p.provider_id == provider_id
            && p.account_identity == account_identity
            && p.endpoint == endpoint
            && p.auth_type == auth_type
            && p.secret_fingerprint == fingerprint
    }) {
        return Ok(ProviderAuthImportResult {
            candidate_id: material.candidate.id.clone(),
            provider_id,
            status: "already_imported".to_string(),
            profile_id: Some(existing.id.clone()),
            message: Some("Matching credential already imported.".to_string()),
        });
    }

    if let Some(existing) = registry.profiles.iter_mut().find(|p| {
        p.provider_id == provider_id
            && p.account_identity == account_identity
            && p.endpoint == endpoint
            && p.auth_type == auth_type
    }) {
        existing.secret_fingerprint = fingerprint.clone();
        existing.updated_at = Utc::now();
        write_secret_material(
            data_root,
            &existing.id,
            &material.candidate.path,
            &material.candidate.kind,
            bytes,
        )
        .await?;
        return Ok(ProviderAuthImportResult {
            candidate_id: material.candidate.id.clone(),
            provider_id,
            status: "updated".to_string(),
            profile_id: Some(existing.id.clone()),
            message: Some("Credential updated.".to_string()),
        });
    }

    let profile_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    registry.profiles.push(ProviderImportedAuthProfile {
        id: profile_id.clone(),
        provider_id: provider_id.clone(),
        provider_label: material.candidate.provider_label.clone(),
        label: material
            .label
            .clone()
            .unwrap_or_else(|| format!("{} import", material.candidate.provider_label)),
        account_identity,
        endpoint,
        auth_type,
        source_path: material.candidate.path.clone(),
        source_kind: material.candidate.kind.clone(),
        secret_fingerprint: fingerprint,
        imported_at: now,
        updated_at: now,
    });
    write_secret_material(
        data_root,
        &profile_id,
        &material.candidate.path,
        &material.candidate.kind,
        bytes,
    )
    .await?;

    Ok(ProviderAuthImportResult {
        candidate_id: material.candidate.id.clone(),
        provider_id,
        status: "imported".to_string(),
        profile_id: Some(profile_id),
        message: Some("Credential imported.".to_string()),
    })
}

pub async fn import_provider_auth_candidates(
    data_root: &Path,
    candidate_ids: &[String],
) -> Result<Vec<ProviderAuthImportResult>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let roots = host_roots()?;
    let materials = scan_with_roots(&roots);
    let mut by_id: HashMap<String, CandidateMaterial> = HashMap::new();
    for material in materials {
        by_id.insert(material.candidate.id.clone(), material);
    }

    let mut registry = load_imported_registry(data_root).await;
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

        let result = if material.candidate.provider_id == "codex" {
            import_codex_candidate(data_root, material).await?
        } else {
            import_generic_candidate(data_root, material, &mut registry).await?
        };
        results.push(result);
    }

    save_imported_registry(data_root, &registry).await?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn generic_import_dedupes_and_updates() {
        let dir = tempfile::tempdir().unwrap();
        let data_root = dir.path().join("data");
        tokio::fs::create_dir_all(&data_root).await.unwrap();

        let now = Utc::now();
        let mut registry = ProviderImportedAuthRegistry {
            profiles: vec![ProviderImportedAuthProfile {
                id: "p1".to_string(),
                provider_id: "gemini".to_string(),
                provider_label: "Gemini".to_string(),
                label: "Gemini API key".to_string(),
                account_identity: None,
                endpoint: None,
                auth_type: Some("api_key".to_string()),
                source_path: "/tmp/.gemini/.env".to_string(),
                source_kind: "env_file".to_string(),
                secret_fingerprint: "old".to_string(),
                imported_at: now,
                updated_at: now,
            }],
        };

        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "c1".to_string(),
                provider_id: "gemini".to_string(),
                provider_label: "Gemini".to_string(),
                kind: "env_file".to_string(),
                path: "/tmp/.gemini/.env".to_string(),
                signal_strength: "strong".to_string(),
                confidence: "medium".to_string(),
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
            secret_bytes: Some(b"OPENAI_API_KEY=abc".to_vec()),
            label: Some("Gemini API key".to_string()),
        };

        let out = import_generic_candidate(&data_root, &material, &mut registry)
            .await
            .unwrap();
        assert_eq!(out.status, "updated");
        assert_eq!(registry.profiles.len(), 1);
    }
}
