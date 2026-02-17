use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

const REGISTRY_VERSION: u32 = 1;
const SECRET_VERSION: u32 = 1;

const PROVIDER_CODEX: &str = "codex";
const PROVIDER_CLAUDE: &str = "claude-crp";

const CODEX_AUTH_TYPE_BEARER: &str = "bearer";
const CLAUDE_AUTH_TYPE_API_KEY: &str = "api_key";

static REGISTRY_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSourceKind {
    #[default]
    Subscription,
    Endpoint,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessApiShape {
    OpenaiResponses,
    AnthropicMessages,
}

impl HarnessApiShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessEndpointVerificationStatus {
    Unknown,
    Valid,
    Invalid,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessEndpointRecord {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub base_url: String,
    pub api_shape: HarnessApiShape,
    pub auth_type: String,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
    pub last_verification_status: HarnessEndpointVerificationStatus,
    #[serde(default)]
    pub last_verification_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessProviderSourceConfig {
    pub provider_id: String,
    pub selected_source_kind: HarnessSourceKind,
    #[serde(default)]
    pub selected_endpoint_id: Option<String>,
    #[serde(default)]
    pub endpoints: Vec<HarnessEndpointRecord>,
}

#[derive(Debug, Clone)]
pub struct ResolvedHarnessSource {
    pub source_kind: HarnessSourceKind,
    pub endpoint: Option<HarnessEndpointRecord>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct HarnessEndpointUpsert {
    pub endpoint_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_shape: HarnessApiShape,
    pub model_override: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessEndpointRecordInternal {
    id: String,
    provider_id: String,
    name: String,
    base_url: String,
    api_shape: HarnessApiShape,
    auth_type: String,
    #[serde(default)]
    model_override: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_verification_status: HarnessEndpointVerificationStatus,
    #[serde(default)]
    last_verification_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_error: Option<String>,
    secret_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HarnessProviderConfigInternal {
    #[serde(default)]
    selected_source_kind: HarnessSourceKind,
    #[serde(default)]
    selected_endpoint_id: Option<String>,
    #[serde(default)]
    endpoints: Vec<HarnessEndpointRecordInternal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessSourceRegistryInternal {
    version: u32,
    #[serde(default)]
    providers: BTreeMap<String, HarnessProviderConfigInternal>,
}

impl Default for HarnessSourceRegistryInternal {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointSecretEnvelope {
    version: u32,
    api_key: String,
}

fn registry_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json")
}

fn endpoint_secret_dir(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("harness_endpoints")
}

fn endpoint_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    endpoint_secret_dir(data_root).join(secret_ref)
}

fn codex_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("codex")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn normalize_provider_id(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        PROVIDER_CODEX => Some(PROVIDER_CODEX),
        PROVIDER_CLAUDE => Some(PROVIDER_CLAUDE),
        _ => None,
    }
}

pub fn supports_harness_endpoint(provider_id: &str) -> bool {
    normalize_provider_id(provider_id).is_some()
}

pub fn default_shape_for_provider(provider_id: &str) -> Option<HarnessApiShape> {
    match normalize_provider_id(provider_id) {
        Some(PROVIDER_CODEX) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CLAUDE) => Some(HarnessApiShape::AnthropicMessages),
        _ => None,
    }
}

pub fn ensure_shape_compatible(provider_id: &str, shape: HarnessApiShape) -> Result<()> {
    match normalize_provider_id(provider_id) {
        Some(PROVIDER_CODEX) => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "codex requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        Some(PROVIDER_CLAUDE) => {
            if shape != HarnessApiShape::AnthropicMessages {
                anyhow::bail!(
                    "claude-crp requires api_shape=anthropic_messages; found {}",
                    shape.as_str()
                );
            }
        }
        _ => anyhow::bail!("provider does not support harness endpoints: {provider_id}"),
    }
    Ok(())
}

async fn load_registry(data_root: &Path) -> Result<HarnessSourceRegistryInternal> {
    let path = registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing harness source registry {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(HarnessSourceRegistryInternal::default())
        }
        Err(err) => {
            Err(err).with_context(|| format!("reading harness source registry {}", path.display()))
        }
    }
}

async fn save_registry(data_root: &Path, registry: &HarnessSourceRegistryInternal) -> Result<()> {
    let path = registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    let tmp_path = path.with_extension(format!("json.tmp.{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::rename(&tmp_path, &path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

fn normalize_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("base_url is required");
    }
    let parsed = Url::parse(trimmed).context("base_url must be a valid URL")?;
    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        anyhow::bail!("base_url must use http:// or https://");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn normalize_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("endpoint name is required");
    }
    Ok(trimmed.to_string())
}

fn ensure_safe_endpoint_id(endpoint_id: &str) -> Result<()> {
    if endpoint_id.is_empty() {
        anyhow::bail!("endpoint_id is required");
    }
    if endpoint_id.len() > 128 {
        anyhow::bail!("endpoint_id must be 128 characters or fewer");
    }
    if !endpoint_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("endpoint_id may only contain ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn normalize_endpoint_id(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    ensure_safe_endpoint_id(trimmed)?;
    Ok(trimmed.to_string())
}

async fn write_endpoint_secret(data_root: &Path, secret_ref: &str, api_key: &str) -> Result<()> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        anyhow::bail!("api_key is required");
    }
    let dir = endpoint_secret_dir(data_root);
    tokio::fs::create_dir_all(&dir).await?;
    let path = endpoint_secret_path(data_root, secret_ref);
    let payload = serde_json::to_vec_pretty(&EndpointSecretEnvelope {
        version: SECRET_VERSION,
        api_key: trimmed.to_string(),
    })?;
    tokio::fs::write(&path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

async fn read_endpoint_secret(data_root: &Path, secret_ref: &str) -> Result<String> {
    let path = endpoint_secret_path(data_root, secret_ref);
    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading endpoint secret {}", path.display()))?;
    let parsed: EndpointSecretEnvelope = serde_json::from_str(&raw)
        .with_context(|| format!("parsing endpoint secret {}", path.display()))?;
    let key = parsed.api_key.trim().to_string();
    if key.is_empty() {
        anyhow::bail!("endpoint secret has empty api_key");
    }
    Ok(key)
}

fn public_endpoint_from_internal(
    endpoint: &HarnessEndpointRecordInternal,
) -> HarnessEndpointRecord {
    HarnessEndpointRecord {
        id: endpoint.id.clone(),
        provider_id: endpoint.provider_id.clone(),
        name: endpoint.name.clone(),
        base_url: endpoint.base_url.clone(),
        api_shape: endpoint.api_shape,
        auth_type: endpoint.auth_type.clone(),
        model_override: endpoint.model_override.clone(),
        created_at: endpoint.created_at,
        updated_at: endpoint.updated_at,
        last_verification_status: endpoint.last_verification_status,
        last_verification_at: endpoint.last_verification_at,
        last_error: endpoint.last_error.clone(),
        has_api_key: true,
    }
}

pub async fn get_provider_source_config(
    data_root: &Path,
    provider_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .get(canonical)
        .cloned()
        .unwrap_or_default();
    let mut selected_endpoint_id = provider.selected_endpoint_id;
    if provider.selected_source_kind == HarnessSourceKind::Endpoint {
        let exists = selected_endpoint_id
            .as_ref()
            .and_then(|id| provider.endpoints.iter().find(|ep| ep.id == *id))
            .is_some();
        if !exists {
            selected_endpoint_id = None;
        }
    }
    Ok(HarnessProviderSourceConfig {
        provider_id: canonical.to_string(),
        selected_source_kind: if selected_endpoint_id.is_some() {
            provider.selected_source_kind
        } else {
            HarnessSourceKind::Subscription
        },
        selected_endpoint_id,
        endpoints: provider
            .endpoints
            .iter()
            .map(public_endpoint_from_internal)
            .collect(),
    })
}

pub async fn upsert_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    input: HarnessEndpointUpsert,
) -> Result<HarnessEndpointRecord> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    ensure_shape_compatible(canonical, input.api_shape)?;

    let name = normalize_name(&input.name)?;
    let base_url = normalize_base_url(&input.base_url)?;
    let model_override = input
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let now = Utc::now();
    let endpoint_id = match input
        .endpoint_id
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => normalize_endpoint_id(&value)?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    ensure_safe_endpoint_id(&endpoint_id)?;

    let existing_index = provider
        .endpoints
        .iter()
        .position(|ep| ep.id == endpoint_id);
    let secret_ref = existing_index
        .and_then(|idx| provider.endpoints.get(idx).map(|ep| ep.secret_ref.clone()))
        .unwrap_or_else(|| format!("{}-{}.json", canonical, endpoint_id));

    if existing_index.is_none() && input.api_key.is_none() {
        anyhow::bail!("api_key is required when creating an endpoint");
    }

    if let Some(api_key) = input.api_key.as_ref() {
        write_endpoint_secret(data_root, &secret_ref, api_key).await?;
    }

    let auth_type = match canonical {
        PROVIDER_CODEX => CODEX_AUTH_TYPE_BEARER,
        PROVIDER_CLAUDE => CLAUDE_AUTH_TYPE_API_KEY,
        _ => CODEX_AUTH_TYPE_BEARER,
    }
    .to_string();

    let mut next = HarnessEndpointRecordInternal {
        id: endpoint_id.clone(),
        provider_id: canonical.to_string(),
        name,
        base_url,
        api_shape: input.api_shape,
        auth_type,
        model_override,
        created_at: now,
        updated_at: now,
        last_verification_status: HarnessEndpointVerificationStatus::Unknown,
        last_verification_at: None,
        last_error: None,
        secret_ref,
    };

    if let Some(idx) = existing_index {
        if let Some(previous) = provider.endpoints.get(idx) {
            next.created_at = previous.created_at;
        }
        provider.endpoints[idx] = next.clone();
    } else {
        provider.endpoints.push(next.clone());
    }

    save_registry(data_root, &registry).await?;
    Ok(public_endpoint_from_internal(&next))
}

pub async fn delete_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let before = provider.endpoints.len();
    let removed: Vec<(String, String)> = provider
        .endpoints
        .iter()
        .filter(|ep| ep.id == endpoint_id)
        .map(|ep| (ep.id.clone(), ep.secret_ref.clone()))
        .collect();
    provider.endpoints.retain(|ep| ep.id != endpoint_id);

    if provider.selected_endpoint_id.as_deref() == Some(endpoint_id) {
        provider.selected_source_kind = HarnessSourceKind::Subscription;
        provider.selected_endpoint_id = None;
    }

    if before != provider.endpoints.len() {
        for (removed_endpoint_id, secret_ref) in removed {
            let secret_path = endpoint_secret_path(data_root, &secret_ref);
            match tokio::fs::remove_file(&secret_path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("removing endpoint secret {}", secret_path.display())
                    });
                }
            }

            if canonical == PROVIDER_CODEX {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                let endpoint_home = codex_endpoint_home(data_root, &removed_endpoint_id);
                match tokio::fs::remove_dir_all(&endpoint_home).await {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "removing codex endpoint home for endpoint {}",
                                removed_endpoint_id
                            )
                        });
                    }
                }
            }
        }
        save_registry(data_root, &registry).await?;
    }

    get_provider_source_config(data_root, canonical).await
}

pub async fn set_provider_source_selection(
    data_root: &Path,
    provider_id: &str,
    source_kind: HarnessSourceKind,
    endpoint_id: Option<String>,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    match source_kind {
        HarnessSourceKind::Subscription => {
            provider.selected_source_kind = HarnessSourceKind::Subscription;
            provider.selected_endpoint_id = None;
        }
        HarnessSourceKind::Endpoint => {
            let endpoint_id = endpoint_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("endpoint_id is required for endpoint source"))?;
            let exists = provider.endpoints.iter().any(|ep| ep.id == endpoint_id);
            if !exists {
                anyhow::bail!("unknown endpoint_id: {}", endpoint_id);
            }
            provider.selected_source_kind = HarnessSourceKind::Endpoint;
            provider.selected_endpoint_id = Some(endpoint_id);
        }
    }

    save_registry(data_root, &registry).await?;
    get_provider_source_config(data_root, canonical).await
}

pub async fn mark_endpoint_verification(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
    status: HarnessEndpointVerificationStatus,
    error: Option<String>,
) -> Result<()> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let Some(provider) = registry.providers.get_mut(canonical) else {
        return Ok(());
    };
    let Some(endpoint) = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
    else {
        return Ok(());
    };
    endpoint.last_verification_status = status;
    endpoint.last_verification_at = Some(Utc::now());
    endpoint.last_error = error;
    endpoint.updated_at = Utc::now();
    save_registry(data_root, &registry).await
}

async fn prepare_codex_home_with_api_key(codex_home: &Path, api_key: &str) -> Result<()> {
    tokio::fs::create_dir_all(codex_home).await?;
    let auth_path = codex_home.join("auth.json");
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "OPENAI_API_KEY": api_key,
    }))?;
    tokio::fs::write(&auth_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(auth_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

fn provider_store<'a>(
    registry: &'a HarnessSourceRegistryInternal,
    provider_id: &str,
) -> Option<&'a HarnessProviderConfigInternal> {
    registry.providers.get(provider_id)
}

async fn resolve_internal(
    data_root: &Path,
    provider_id: &str,
    require_verified_endpoint: bool,
) -> Result<ResolvedHarnessSource> {
    let canonical = match normalize_provider_id(provider_id) {
        Some(id) => id,
        None => {
            return Ok(ResolvedHarnessSource {
                source_kind: HarnessSourceKind::Subscription,
                endpoint: None,
                env: HashMap::new(),
            });
        }
    };

    let registry = load_registry(data_root).await?;
    let provider = provider_store(&registry, canonical)
        .cloned()
        .unwrap_or_default();

    if provider.selected_source_kind != HarnessSourceKind::Endpoint {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: HashMap::new(),
        });
    }

    let endpoint_id = provider
        .selected_endpoint_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no endpoint selected for provider {}", canonical))?;
    let endpoint = provider
        .endpoints
        .iter()
        .find(|ep| ep.id == endpoint_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("selected endpoint not found for provider {}", canonical))?;

    if require_verified_endpoint
        && endpoint.last_verification_status != HarnessEndpointVerificationStatus::Valid
    {
        anyhow::bail!(
            "selected endpoint '{}' for {} is not verified; verify it in Settings before running",
            endpoint.name,
            canonical
        );
    }

    let api_key = read_endpoint_secret(data_root, &endpoint.secret_ref).await?;
    let mut env = HashMap::new();

    match canonical {
        PROVIDER_CODEX => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let codex_home = codex_endpoint_home(data_root, &endpoint.id);
            prepare_codex_home_with_api_key(&codex_home, &api_key).await?;
            env.insert(
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().to_string(),
            );
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), endpoint.base_url.clone());
        }
        PROVIDER_CLAUDE => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("ANTHROPIC_API_KEY".to_string(), api_key);
            env.insert("ANTHROPIC_BASE_URL".to_string(), endpoint.base_url.clone());
        }
        _ => {}
    }

    let public = public_endpoint_from_internal(&endpoint);
    Ok(ResolvedHarnessSource {
        source_kind: HarnessSourceKind::Endpoint,
        endpoint: Some(public),
        env,
    })
}

pub async fn resolve_provider_source_for_probe(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_internal(data_root, provider_id, false).await
}

pub async fn resolve_provider_source_for_run(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_internal(data_root, provider_id, true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_to_subscription_without_registry() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = get_provider_source_config(root.path(), PROVIDER_CODEX)
            .await
            .expect("config");
        assert_eq!(cfg.selected_source_kind, HarnessSourceKind::Subscription);
        assert!(cfg.selected_endpoint_id.is_none());
        assert!(cfg.endpoints.is_empty());
    }

    #[tokio::test]
    async fn invalid_registry_json_returns_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = registry_path(root.path());
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&path, b"{not-json")
            .await
            .expect("write invalid");

        let err = get_provider_source_config(root.path(), PROVIDER_CODEX)
            .await
            .expect_err("invalid registry should fail");
        assert!(err.to_string().contains("parsing harness source registry"));
    }

    #[tokio::test]
    async fn codex_endpoint_requires_verify_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_shape: HarnessApiShape::OpenaiResponses,
                model_override: None,
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CODEX,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let err = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
            .await
            .expect_err("expected verify gate error");
        assert!(err.to_string().contains("not verified"));

        mark_endpoint_verification(
            root.path(),
            PROVIDER_CODEX,
            &endpoint.id,
            HarnessEndpointVerificationStatus::Valid,
            None,
        )
        .await
        .expect("mark verified");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert!(resolved.env.contains_key("CODEX_HOME"));
        assert_eq!(
            resolved.env.get("OPENAI_BASE_URL"),
            Some(&"https://openrouter.ai/api/v1".to_string())
        );
    }

    #[tokio::test]
    async fn deleting_codex_endpoint_removes_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_shape: HarnessApiShape::OpenaiResponses,
                model_override: None,
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CODEX,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        resolve_provider_source_for_probe(root.path(), PROVIDER_CODEX)
            .await
            .expect("resolve probe");

        let endpoint_home = codex_endpoint_home(root.path(), &endpoint.id);
        assert!(endpoint_home.join("auth.json").exists());

        delete_provider_endpoint(root.path(), PROVIDER_CODEX, &endpoint.id)
            .await
            .expect("delete endpoint");

        assert!(!endpoint_home.exists());
    }

    #[tokio::test]
    async fn shape_compatibility_rejects_mismatch() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CLAUDE,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "wrong".to_string(),
                base_url: "https://example.com".to_string(),
                api_shape: HarnessApiShape::OpenaiResponses,
                model_override: None,
                api_key: Some("k".to_string()),
            },
        )
        .await
        .expect_err("expected shape mismatch");
        assert!(err
            .to_string()
            .contains("claude-crp requires api_shape=anthropic_messages"));
    }

    #[tokio::test]
    async fn unsafe_endpoint_id_is_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: Some("../escape".to_string()),
                name: "bad".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_shape: HarnessApiShape::OpenaiResponses,
                model_override: None,
                api_key: Some("k".to_string()),
            },
        )
        .await
        .expect_err("unsafe endpoint id should fail");
        assert!(err
            .to_string()
            .contains("endpoint_id may only contain ASCII letters"));
    }
}
