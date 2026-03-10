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
const CTX_PROVIDER_AUTH_IMPORT_HOME_ENV: &str = "CTX_PROVIDER_AUTH_IMPORT_HOME";
const CTX_PROVIDER_AUTH_IMPORT_XDG_CONFIG_HOME_ENV: &str =
    "CTX_PROVIDER_AUTH_IMPORT_XDG_CONFIG_HOME";
const CTX_PROVIDER_AUTH_IMPORT_XDG_DATA_HOME_ENV: &str = "CTX_PROVIDER_AUTH_IMPORT_XDG_DATA_HOME";
const CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME_ENV: &str = "CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME";

mod catalog;
mod importers;
mod legacy;
mod parsers;

#[cfg(test)]
use catalog::{host_roots, scan_with_roots, sha256_hex, summarize_env};
#[cfg(test)]
use importers::import_codex_candidate;
#[cfg(test)]
use legacy::{imported_secret_path, legacy_migration_marker_path};
#[cfg(test)]
use parsers::parse_env_file;

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

#[derive(Debug, Clone)]
struct AuthImportScanner {
    roots: HostRoots,
}

struct CanonicalAuthImporter<'a> {
    data_root: &'a Path,
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

pub async fn load_imported_registry(data_root: &Path) -> ProviderImportedAuthRegistry {
    legacy::load_imported_registry(data_root).await
}

pub async fn save_imported_registry(
    data_root: &Path,
    registry: &ProviderImportedAuthRegistry,
) -> Result<()> {
    legacy::save_imported_registry(data_root, registry).await
}

pub async fn list_provider_auth_import_candidates() -> Result<Vec<ProviderAuthImportCandidate>> {
    catalog::list_provider_auth_import_candidates().await
}

pub async fn list_provider_auth_profiles(
    data_root: &Path,
) -> Result<Vec<ProviderImportedAuthProfile>> {
    importers::list_provider_auth_profiles(data_root).await
}

#[cfg(test)]
async fn import_candidate_to_canonical(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    importers::import_candidate_to_canonical(data_root, material).await
}

#[cfg(test)]
async fn migrate_legacy_imported_profiles_once(data_root: &Path) -> Result<()> {
    importers::migrate_legacy_imported_profiles_once(data_root).await
}

pub async fn import_provider_auth_candidates(
    data_root: &Path,
    candidate_ids: &[String],
) -> Result<Vec<ProviderAuthImportResult>> {
    importers::import_provider_auth_candidates(data_root, candidate_ids).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

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

    #[tokio::test]
    async fn host_roots_honors_auth_import_override_envs() {
        let _env_lock = lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let override_root = dir.path().join("override");
        let _home = EnvGuard::set(
            CTX_PROVIDER_AUTH_IMPORT_HOME_ENV,
            override_root.join("home").to_string_lossy().as_ref(),
        );
        let _config = EnvGuard::set(
            CTX_PROVIDER_AUTH_IMPORT_XDG_CONFIG_HOME_ENV,
            override_root.join("config").to_string_lossy().as_ref(),
        );
        let _data = EnvGuard::set(
            CTX_PROVIDER_AUTH_IMPORT_XDG_DATA_HOME_ENV,
            override_root.join("data").to_string_lossy().as_ref(),
        );
        let _codex = EnvGuard::set(
            CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME_ENV,
            override_root.join("codex").to_string_lossy().as_ref(),
        );

        let roots = host_roots().unwrap();
        assert_eq!(roots.home, override_root.join("home"));
        assert_eq!(roots.xdg_config, override_root.join("config"));
        assert_eq!(roots.xdg_data, override_root.join("data"));
        assert_eq!(roots.codex_home, override_root.join("codex"));
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
        let profiles = list_provider_auth_profiles(root).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, account_id);
        assert_eq!(profiles[0].provider_id, "codex");
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
        let profiles = list_provider_auth_profiles(root).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].provider_id, "gemini");
        assert_eq!(profiles[0].id, registry.active_account_id.unwrap());
    }

    #[tokio::test]
    async fn gemini_env_candidate_with_base_url_is_rejected() {
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

        let err = import_candidate_to_canonical(root, &material)
            .await
            .expect_err("import should fail");
        assert!(err
            .to_string()
            .contains("OpenAI-compatible endpoint imports are not supported"));
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
    async fn gemini_env_candidate_with_google_api_key_imports_native_key_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "gemini-env-google-api-key".to_string(),
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
            secret_bytes: Some(b"GOOGLE_API_KEY=key-google\n".to_vec()),
            label: Some("Gemini Google API key".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "imported");
        let config = harness_sources::get_provider_source_config(root, "gemini")
            .await
            .unwrap();
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
    async fn gemini_env_candidate_with_google_api_key_and_vertex_markers_imports_vertex_ai() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let material = CandidateMaterial {
            candidate: ProviderAuthImportCandidate {
                id: "gemini-env-vertex".to_string(),
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
            secret_bytes: Some(
                b"GOOGLE_API_KEY=key-google\nGOOGLE_GENAI_USE_VERTEXAI=true\n".to_vec(),
            ),
            label: Some("Gemini Vertex AI".to_string()),
        };

        let result = import_candidate_to_canonical(root, &material)
            .await
            .unwrap();
        assert_eq!(result.status, "imported");
        let config = harness_sources::get_provider_source_config(root, "gemini")
            .await
            .unwrap();
        let selected_id = config
            .selected_endpoint_id
            .as_deref()
            .expect("selected endpoint id");
        let endpoint = config
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == selected_id)
            .expect("selected endpoint");
        assert_eq!(endpoint.auth_type, "vertex_ai");
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
        let profiles = list_provider_auth_profiles(root).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].provider_id, "qwen");
        assert_eq!(profiles[0].id, first.profile_id.clone().unwrap());

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
        let profiles = list_provider_auth_profiles(root).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, updated.profile_id.unwrap());
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
