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
const PROVIDER_GEMINI: &str = "gemini";
const PROVIDER_KIMI: &str = "kimi";
const PROVIDER_QWEN: &str = "qwen";
const PROVIDER_OPENCODE: &str = "opencode";
const PROVIDER_MISTRAL: &str = "mistral";
const PROVIDER_GOOSE: &str = "goose";
const PROVIDER_CAGENT: &str = "cagent";
const PROVIDER_AMP: &str = "amp";
const PROVIDER_DROID: &str = "droid";
const PROVIDER_CODY: &str = "cody";
const PROVIDER_CONTINUE: &str = "continue";
const PROVIDER_CLINE: &str = "cline";
const PROVIDER_SWE_AGENT: &str = "swe-agent";
const PROVIDER_OPENHANDS: &str = "openhands";
const PROVIDER_COPILOT: &str = "copilot";
const PROVIDER_KIRO: &str = "kiro";
const PROVIDER_ROVO: &str = "rovo";
const PROVIDER_AUGGIE: &str = "auggie";
const PROVIDER_PI: &str = "pi";
const PROVIDER_CURSOR: &str = "cursor";

const CODEX_AUTH_TYPE_BEARER: &str = "bearer";
const CLAUDE_AUTH_TYPE_API_KEY: &str = "api_key";
const KIRO_AUTH_TOKEN_RELATIVE_PATH: &str = ".aws/sso/cache/kiro-auth-token.json";

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
    #[serde(default)]
    pub base_url: Option<String>,
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
    pub base_url: Option<String>,
    pub api_shape: Option<HarnessApiShape>,
    pub model_override: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessEndpointImportMatchKind {
    ExactCredentials,
    SameConfig,
}

#[derive(Debug, Clone)]
pub struct HarnessEndpointImportMatch {
    pub endpoint_id: String,
    pub kind: HarnessEndpointImportMatchKind,
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

fn kiro_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("kiro")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn cursor_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("cursor")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn container_workspaces_root(data_root: &Path) -> PathBuf {
    data_root.join("containers").join("workspaces")
}

async fn container_runtime_data_roots(data_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut entries = match tokio::fs::read_dir(container_workspaces_root(data_root)).await {
        Ok(entries) => entries,
        Err(_) => return roots,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let runtime_root = entry.path().join("data");
        match tokio::fs::metadata(&runtime_root).await {
            Ok(metadata) if metadata.is_dir() => roots.push(runtime_root),
            _ => {}
        }
    }

    roots
}

async fn remove_kiro_endpoint_home_for_root(root: &Path, endpoint_id: &str) -> Result<()> {
    let endpoint_home = kiro_endpoint_home(root, endpoint_id);
    match tokio::fs::remove_dir_all(&endpoint_home).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("removing kiro endpoint home for endpoint {}", endpoint_id)),
    }
}

async fn remove_kiro_endpoint_homes_for_runtime_roots(
    data_root: &Path,
    endpoint_id: &str,
) -> Result<()> {
    for runtime_root in container_runtime_data_roots(data_root).await {
        remove_kiro_endpoint_home_for_root(&runtime_root, endpoint_id).await?;
    }
    Ok(())
}

async fn remove_cursor_endpoint_home_for_root(root: &Path, endpoint_id: &str) -> Result<()> {
    let endpoint_home = cursor_endpoint_home(root, endpoint_id);
    match tokio::fs::remove_dir_all(&endpoint_home).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("removing cursor endpoint home for endpoint {}", endpoint_id)),
    }
}

async fn remove_cursor_endpoint_homes_for_runtime_roots(
    data_root: &Path,
    endpoint_id: &str,
) -> Result<()> {
    for runtime_root in container_runtime_data_roots(data_root).await {
        remove_cursor_endpoint_home_for_root(&runtime_root, endpoint_id).await?;
    }
    Ok(())
}

fn normalize_provider_id(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        PROVIDER_CODEX => Some(PROVIDER_CODEX),
        PROVIDER_CLAUDE => Some(PROVIDER_CLAUDE),
        PROVIDER_GEMINI => Some(PROVIDER_GEMINI),
        PROVIDER_KIMI => Some(PROVIDER_KIMI),
        PROVIDER_QWEN => Some(PROVIDER_QWEN),
        PROVIDER_OPENCODE => Some(PROVIDER_OPENCODE),
        PROVIDER_MISTRAL => Some(PROVIDER_MISTRAL),
        PROVIDER_GOOSE => Some(PROVIDER_GOOSE),
        PROVIDER_CAGENT => Some(PROVIDER_CAGENT),
        PROVIDER_AMP => Some(PROVIDER_AMP),
        PROVIDER_DROID => Some(PROVIDER_DROID),
        PROVIDER_CODY => Some(PROVIDER_CODY),
        PROVIDER_CONTINUE => Some(PROVIDER_CONTINUE),
        PROVIDER_CLINE => Some(PROVIDER_CLINE),
        PROVIDER_SWE_AGENT => Some(PROVIDER_SWE_AGENT),
        PROVIDER_OPENHANDS => Some(PROVIDER_OPENHANDS),
        PROVIDER_COPILOT => Some(PROVIDER_COPILOT),
        PROVIDER_KIRO => Some(PROVIDER_KIRO),
        PROVIDER_ROVO => Some(PROVIDER_ROVO),
        PROVIDER_AUGGIE => Some(PROVIDER_AUGGIE),
        PROVIDER_PI => Some(PROVIDER_PI),
        PROVIDER_CURSOR => Some(PROVIDER_CURSOR),
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
        Some(PROVIDER_GEMINI) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_KIMI) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_QWEN) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_OPENCODE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_MISTRAL) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_GOOSE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CAGENT) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_AMP) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_DROID) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CODY) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CONTINUE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CLINE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_SWE_AGENT) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_OPENHANDS) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_COPILOT) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_KIRO) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_ROVO) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_AUGGIE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_PI) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CURSOR) => Some(HarnessApiShape::OpenaiResponses),
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
        Some(PROVIDER_GEMINI) => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "gemini requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        Some(PROVIDER_KIMI) => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "kimi requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        Some(PROVIDER_QWEN)
        | Some(PROVIDER_OPENCODE)
        | Some(PROVIDER_MISTRAL)
        | Some(PROVIDER_GOOSE)
        | Some(PROVIDER_CAGENT)
        | Some(PROVIDER_AMP)
        | Some(PROVIDER_DROID)
        | Some(PROVIDER_CODY)
        | Some(PROVIDER_CONTINUE)
        | Some(PROVIDER_CLINE)
        | Some(PROVIDER_SWE_AGENT)
        | Some(PROVIDER_OPENHANDS)
        | Some(PROVIDER_COPILOT)
        | Some(PROVIDER_KIRO)
        | Some(PROVIDER_ROVO)
        | Some(PROVIDER_AUGGIE)
        | Some(PROVIDER_PI)
        | Some(PROVIDER_CURSOR) => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "{} requires api_shape=openai_responses; found {}",
                    provider_id,
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

fn provider_requires_endpoint_base_url(provider_id: &str) -> bool {
    matches!(
        provider_id,
        PROVIDER_CODEX
            | PROVIDER_CLAUDE
            | PROVIDER_GEMINI
            | PROVIDER_KIMI
            | PROVIDER_QWEN
            | PROVIDER_OPENCODE
            | PROVIDER_MISTRAL
            | PROVIDER_GOOSE
            | PROVIDER_CAGENT
            | PROVIDER_CLINE
            | PROVIDER_SWE_AGENT
            | PROVIDER_OPENHANDS
    )
}

fn normalize_base_url_for_provider(provider_id: &str, raw: Option<&str>) -> Result<String> {
    match raw {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                if provider_requires_endpoint_base_url(provider_id) {
                    anyhow::bail!("base_url is required");
                }
                Ok(String::new())
            } else {
                normalize_base_url(trimmed)
            }
        }
        None => {
            if provider_requires_endpoint_base_url(provider_id) {
                anyhow::bail!("base_url is required");
            }
            Ok(String::new())
        }
    }
}

fn endpoint_base_url_or_err(endpoint: &HarnessEndpointRecordInternal) -> Result<String> {
    let trimmed = endpoint.base_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!(
            "selected endpoint '{}' for {} is missing base_url",
            endpoint.name,
            endpoint.provider_id
        );
    }
    Ok(trimmed.to_string())
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

fn parse_required_json_object(raw: &str, field_name: &str) -> Result<serde_json::Value> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).with_context(|| format!("{field_name} must be valid JSON"))?;
    if !parsed.is_object() {
        anyhow::bail!("{field_name} must be a JSON object");
    }
    Ok(parsed)
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
        base_url: if endpoint.base_url.trim().is_empty() {
            None
        } else {
            Some(endpoint.base_url.clone())
        },
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

pub async fn find_provider_endpoint_import_match(
    data_root: &Path,
    provider_id: &str,
    base_url: Option<String>,
    api_shape: HarnessApiShape,
    model_override: Option<String>,
    api_key: &str,
) -> Result<Option<HarnessEndpointImportMatch>> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    ensure_shape_compatible(canonical, api_shape)?;
    let normalized_base_url = normalize_base_url_for_provider(canonical, base_url.as_deref())?;
    let normalized_model_override = model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let registry = load_registry(data_root).await?;
    let Some(provider) = registry.providers.get(canonical) else {
        return Ok(None);
    };

    let mut config_match_endpoint_id: Option<String> = None;
    for endpoint in &provider.endpoints {
        if endpoint.base_url != normalized_base_url || endpoint.api_shape != api_shape {
            continue;
        }
        let endpoint_model_override = endpoint
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if endpoint_model_override != normalized_model_override {
            continue;
        }
        if config_match_endpoint_id.is_none() {
            config_match_endpoint_id = Some(endpoint.id.clone());
        }
        if let Ok(existing_api_key) = read_endpoint_secret(data_root, &endpoint.secret_ref).await {
            if existing_api_key == api_key {
                return Ok(Some(HarnessEndpointImportMatch {
                    endpoint_id: endpoint.id.clone(),
                    kind: HarnessEndpointImportMatchKind::ExactCredentials,
                }));
            }
        }
    }

    Ok(
        config_match_endpoint_id.map(|endpoint_id| HarnessEndpointImportMatch {
            endpoint_id,
            kind: HarnessEndpointImportMatchKind::SameConfig,
        }),
    )
}

pub async fn upsert_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    input: HarnessEndpointUpsert,
) -> Result<HarnessEndpointRecord> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let api_shape = input
        .api_shape
        .or_else(|| default_shape_for_provider(canonical))
        .ok_or_else(|| anyhow::anyhow!("api_shape is required"))?;
    ensure_shape_compatible(canonical, api_shape)?;

    let name = normalize_name(&input.name)?;
    let base_url = normalize_base_url_for_provider(canonical, input.base_url.as_deref())?;
    let model_override = input
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if canonical == PROVIDER_KIRO {
        if let Some(api_key) = input.api_key.as_ref() {
            let _ = parse_required_json_object(api_key, "api_key")?;
        }
    }

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
        api_shape,
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
            } else if canonical == PROVIDER_KIRO {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                remove_kiro_endpoint_home_for_root(data_root, &removed_endpoint_id).await?;
                remove_kiro_endpoint_homes_for_runtime_roots(data_root, &removed_endpoint_id)
                    .await?;
            } else if canonical == PROVIDER_CURSOR {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                remove_cursor_endpoint_home_for_root(data_root, &removed_endpoint_id).await?;
                remove_cursor_endpoint_homes_for_runtime_roots(data_root, &removed_endpoint_id)
                    .await?;
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

async fn prepare_kiro_home_with_auth_token_json(
    kiro_home: &Path,
    auth_token_json: &str,
) -> Result<()> {
    let auth_token = parse_required_json_object(auth_token_json, "api_key")?;
    tokio::fs::create_dir_all(kiro_home).await?;
    let token_path = kiro_home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
    if let Some(parent) = token_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(&auth_token)?;
    tokio::fs::write(&token_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            tokio::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::create_dir_all(kiro_home.join(".config")).await?;
    tokio::fs::create_dir_all(kiro_home.join(".cache")).await?;
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
    runtime_data_root: Option<&Path>,
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
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let codex_home = codex_endpoint_home(data_root, &endpoint.id);
            prepare_codex_home_with_api_key(&codex_home, &api_key).await?;
            env.insert(
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().to_string(),
            );
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
        }
        PROVIDER_CLAUDE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("ANTHROPIC_API_KEY".to_string(), api_key);
            env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);
        }
        PROVIDER_GEMINI => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
        }
        PROVIDER_KIMI => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("KIMI_API_KEY".to_string(), api_key);
            env.insert("KIMI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("KIMI_MODEL_NAME".to_string(), model);
            }
        }
        PROVIDER_QWEN | PROVIDER_CAGENT | PROVIDER_CLINE | PROVIDER_SWE_AGENT => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_OPENCODE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
            env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
            env.insert("OPENROUTER_API_KEY".to_string(), api_key.clone());
            env.insert("OPENROUTER_BASE_URL".to_string(), base_url.clone());

            let mut provider_config = serde_json::Map::new();
            provider_config.insert(
                "openrouter".to_string(),
                serde_json::json!({
                    "options": {
                        "baseURL": base_url,
                        "apiKey": api_key,
                    }
                }),
            );
            let mut root = serde_json::Map::new();
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                root.insert("model".to_string(), serde_json::Value::String(model));
            }
            root.insert(
                "provider".to_string(),
                serde_json::Value::Object(provider_config),
            );
            env.insert(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                serde_json::Value::Object(root).to_string(),
            );
        }
        PROVIDER_GOOSE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            env.insert("GOOSE_PROVIDER".to_string(), "openai".to_string());
            env.insert("GOOSE_DISABLE_KEYRING".to_string(), "1".to_string());
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("GOOSE_MODEL".to_string(), model);
            }
        }
        PROVIDER_MISTRAL => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("MISTRAL_API_KEY".to_string(), api_key.clone());
            env.insert("MISTRAL_BASE_URL".to_string(), base_url.clone());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
        }
        PROVIDER_AMP => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("AMP_API_KEY".to_string(), api_key);
        }
        PROVIDER_DROID => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("FACTORY_API_KEY".to_string(), api_key);
        }
        PROVIDER_CODY => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("SRC_ACCESS_TOKEN".to_string(), api_key);
            let base_url = endpoint.base_url.trim().to_string();
            if !base_url.is_empty() {
                env.insert("SRC_ENDPOINT".to_string(), base_url);
            }
        }
        PROVIDER_CONTINUE => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("CONTINUE_API_KEY".to_string(), api_key);
        }
        PROVIDER_OPENHANDS => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("LLM_API_KEY".to_string(), api_key.clone());
            env.insert("LLM_BASE_URL".to_string(), base_url.clone());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("LLM_MODEL".to_string(), model.clone());
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_COPILOT => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("GH_TOKEN".to_string(), api_key.clone());
            env.insert("GITHUB_TOKEN".to_string(), api_key);
        }
        PROVIDER_PI => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("PI_ACP_PROVIDER".to_string(), "openai".to_string());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            let base_url = endpoint.base_url.trim().to_string();
            if !base_url.is_empty() {
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("PI_ACP_MODEL".to_string(), model);
            }
        }
        PROVIDER_CURSOR => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let cursor_home_root = runtime_data_root.unwrap_or(data_root);
            let cursor_home = cursor_endpoint_home(cursor_home_root, &endpoint.id);
            tokio::fs::create_dir_all(&cursor_home).await?;
            env.insert("CURSOR_API_KEY".to_string(), api_key);
            env.insert(
                "CURSOR_CONFIG_DIR".to_string(),
                cursor_home.to_string_lossy().to_string(),
            );
            let base_url = endpoint.base_url.trim().to_string();
            if !base_url.is_empty() {
                env.insert("CURSOR_API_BASE_URL".to_string(), base_url.clone());
                env.insert("CURSOR_API_ENDPOINT".to_string(), base_url);
            }
        }
        PROVIDER_KIRO => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let kiro_home_root = runtime_data_root.unwrap_or(data_root);
            let kiro_home = kiro_endpoint_home(kiro_home_root, &endpoint.id);
            prepare_kiro_home_with_auth_token_json(&kiro_home, &api_key).await?;
            env.insert("HOME".to_string(), kiro_home.to_string_lossy().to_string());
            env.insert(
                "XDG_CONFIG_HOME".to_string(),
                kiro_home.join(".config").to_string_lossy().to_string(),
            );
            env.insert(
                "XDG_CACHE_HOME".to_string(),
                kiro_home.join(".cache").to_string_lossy().to_string(),
            );
        }
        PROVIDER_ROVO => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("ATLASSIAN_API_TOKEN".to_string(), api_key.clone());
            env.insert("ROVO_DEV_API_TOKEN".to_string(), api_key);
        }
        PROVIDER_AUGGIE => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("AUGMENT_SESSION_AUTH".to_string(), api_key.clone());
            env.insert("AUGMENT_API_TOKEN".to_string(), api_key);
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
    resolve_internal(data_root, provider_id, false, None).await
}

pub async fn resolve_provider_source_for_run(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_provider_source_for_run_with_runtime_root(data_root, provider_id, None).await
}

pub async fn resolve_provider_source_for_run_with_runtime_root(
    data_root: &Path,
    provider_id: &str,
    runtime_data_root: Option<&Path>,
) -> Result<ResolvedHarnessSource> {
    let require_verified_endpoint = matches!(
        normalize_provider_id(provider_id),
        Some(PROVIDER_CODEX) | Some(PROVIDER_CLAUDE)
    );
    resolve_internal(
        data_root,
        provider_id,
        require_verified_endpoint,
        runtime_data_root,
    )
    .await
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
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
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
    async fn gemini_endpoint_does_not_require_verify_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_GEMINI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Gemini Key".to_string(),
                base_url: Some(
                    "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
                ),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                model_override: None,
                api_key: Some("gemini-key".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_GEMINI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_GEMINI)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(
            resolved.env.get("OPENAI_API_KEY"),
            Some(&"gemini-key".to_string())
        );
        assert_eq!(
            resolved.env.get("OPENAI_BASE_URL"),
            Some(&"https://generativelanguage.googleapis.com/v1beta/openai".to_string())
        );
    }

    #[tokio::test]
    async fn kimi_endpoint_projects_kimi_env_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIMI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kimi Key".to_string(),
                base_url: Some("https://api.moonshot.ai/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                model_override: Some("kimi-k2".to_string()),
                api_key: Some("kimi-key".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIMI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_KIMI)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(
            resolved.env.get("KIMI_API_KEY"),
            Some(&"kimi-key".to_string())
        );
        assert_eq!(
            resolved.env.get("KIMI_BASE_URL"),
            Some(&"https://api.moonshot.ai/v1".to_string())
        );
        assert_eq!(
            resolved.env.get("KIMI_MODEL_NAME"),
            Some(&"kimi-k2".to_string())
        );
    }

    #[tokio::test]
    async fn additional_provider_endpoint_env_projection_smoke() {
        let root = tempfile::tempdir().expect("tempdir");
        let cases: &[(&str, &[&str])] = &[
            (PROVIDER_QWEN, &["OPENAI_API_KEY", "OPENAI_BASE_URL"]),
            (
                PROVIDER_OPENCODE,
                &[
                    "OPENAI_API_KEY",
                    "OPENROUTER_API_KEY",
                    "OPENCODE_CONFIG_CONTENT",
                ],
            ),
            (
                PROVIDER_GOOSE,
                &[
                    "OPENAI_API_KEY",
                    "GOOSE_PROVIDER",
                    "GOOSE_DISABLE_KEYRING",
                    "GOOSE_MODEL",
                ],
            ),
            (PROVIDER_CAGENT, &["OPENAI_API_KEY"]),
            (PROVIDER_MISTRAL, &["MISTRAL_API_KEY", "MISTRAL_BASE_URL"]),
            (PROVIDER_AMP, &["AMP_API_KEY"]),
            (PROVIDER_DROID, &["FACTORY_API_KEY"]),
            (PROVIDER_CODY, &["SRC_ACCESS_TOKEN", "SRC_ENDPOINT"]),
            (PROVIDER_CONTINUE, &["CONTINUE_API_KEY"]),
            (PROVIDER_COPILOT, &["GH_TOKEN", "GITHUB_TOKEN"]),
            (
                PROVIDER_KIRO,
                &["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"],
            ),
            (
                PROVIDER_ROVO,
                &["ATLASSIAN_API_TOKEN", "ROVO_DEV_API_TOKEN"],
            ),
            (
                PROVIDER_AUGGIE,
                &["AUGMENT_SESSION_AUTH", "AUGMENT_API_TOKEN"],
            ),
            (
                PROVIDER_PI,
                &["OPENAI_API_KEY", "PI_ACP_PROVIDER", "PI_ACP_MODEL"],
            ),
            (PROVIDER_CURSOR, &["CURSOR_API_KEY", "CURSOR_CONFIG_DIR"]),
            (PROVIDER_CLINE, &["OPENAI_API_KEY"]),
            (PROVIDER_SWE_AGENT, &["OPENAI_API_KEY"]),
            (
                PROVIDER_OPENHANDS,
                &["LLM_API_KEY", "LLM_BASE_URL", "LLM_MODEL"],
            ),
        ];

        for (provider_id, required_keys) in cases {
            let endpoint = upsert_provider_endpoint(
                root.path(),
                provider_id,
                HarnessEndpointUpsert {
                    endpoint_id: None,
                    name: format!("{provider_id} endpoint"),
                    base_url: if *provider_id == PROVIDER_COPILOT
                        || *provider_id == PROVIDER_KIRO
                        || *provider_id == PROVIDER_ROVO
                        || *provider_id == PROVIDER_AUGGIE
                        || *provider_id == PROVIDER_PI
                        || *provider_id == PROVIDER_CURSOR
                    {
                        None
                    } else {
                        Some("https://openrouter.ai/api/v1".to_string())
                    },
                    api_shape: if *provider_id == PROVIDER_COPILOT
                        || *provider_id == PROVIDER_KIRO
                        || *provider_id == PROVIDER_ROVO
                        || *provider_id == PROVIDER_AUGGIE
                        || *provider_id == PROVIDER_PI
                        || *provider_id == PROVIDER_CURSOR
                    {
                        None
                    } else {
                        Some(HarnessApiShape::OpenaiResponses)
                    },
                    model_override: Some("test-model".to_string()),
                    api_key: Some(if *provider_id == PROVIDER_KIRO {
                        r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()
                    } else {
                        "test-key".to_string()
                    }),
                },
            )
            .await
            .expect("upsert endpoint");

            set_provider_source_selection(
                root.path(),
                provider_id,
                HarnessSourceKind::Endpoint,
                Some(endpoint.id.clone()),
            )
            .await
            .expect("select endpoint");

            let resolved = resolve_provider_source_for_run(root.path(), provider_id)
                .await
                .expect("resolve run");
            for key in *required_keys {
                assert!(
                    resolved.env.contains_key(*key),
                    "{provider_id} missing env key {key}"
                );
            }
        }
    }

    #[tokio::test]
    async fn copilot_endpoint_allows_token_only_upsert() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_COPILOT,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Copilot token".to_string(),
                base_url: None,
                api_shape: None,
                model_override: None,
                api_key: Some("ghp_test".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        assert!(endpoint.base_url.is_none());
        assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

        let cfg = get_provider_source_config(root.path(), PROVIDER_COPILOT)
            .await
            .expect("get source config");
        let stored = cfg
            .endpoints
            .iter()
            .find(|candidate| candidate.id == endpoint.id)
            .expect("stored endpoint");
        assert!(stored.base_url.is_none());
        assert_eq!(stored.api_shape, HarnessApiShape::OpenaiResponses);
    }

    #[tokio::test]
    async fn pi_endpoint_allows_token_only_upsert_and_optional_base_url() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_PI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Pi token".to_string(),
                base_url: None,
                api_shape: None,
                model_override: Some("gpt-5".to_string()),
                api_key: Some("pi-key".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        assert!(endpoint.base_url.is_none());
        assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

        set_provider_source_selection(
            root.path(),
            PROVIDER_PI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_PI)
            .await
            .expect("resolve run");
        assert_eq!(
            resolved.env.get("OPENAI_API_KEY"),
            Some(&"pi-key".to_string())
        );
        assert_eq!(
            resolved.env.get("PI_ACP_PROVIDER"),
            Some(&"openai".to_string())
        );
        assert_eq!(resolved.env.get("PI_ACP_MODEL"), Some(&"gpt-5".to_string()));
        assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
    }

    #[tokio::test]
    async fn cursor_endpoint_allows_token_only_upsert_and_sets_cursor_env() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CURSOR,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Cursor key".to_string(),
                base_url: None,
                api_shape: None,
                model_override: None,
                api_key: Some("cursor-key".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CURSOR,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CURSOR)
            .await
            .expect("resolve run");
        assert_eq!(
            resolved.env.get("CURSOR_API_KEY"),
            Some(&"cursor-key".to_string())
        );
        let cursor_config_dir = resolved
            .env
            .get("CURSOR_CONFIG_DIR")
            .expect("CURSOR_CONFIG_DIR should be set");
        assert!(Path::new(cursor_config_dir).exists());
    }

    #[tokio::test]
    async fn kiro_endpoint_uses_runtime_root_for_run_resolution_when_provided() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime_root = tempfile::tempdir().expect("runtime tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIRO,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kiro token".to_string(),
                base_url: None,
                api_shape: None,
                model_override: None,
                api_key: Some(r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIRO,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run_with_runtime_root(
            root.path(),
            PROVIDER_KIRO,
            Some(runtime_root.path()),
        )
        .await
        .expect("resolve run with runtime root");

        let home = PathBuf::from(
            resolved
                .env
                .get("HOME")
                .expect("HOME should be set for kiro endpoint"),
        );
        assert!(home.starts_with(runtime_root.path()));

        let token_path = home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
        assert!(token_path.exists());
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
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
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
    async fn deleting_kiro_endpoint_removes_runtime_root_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime_root = root
            .path()
            .join("containers")
            .join("workspaces")
            .join("workspace-kiro")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root)
            .await
            .expect("runtime root");

        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIRO,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kiro token".to_string(),
                base_url: None,
                api_shape: None,
                model_override: None,
                api_key: Some(r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIRO,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        resolve_provider_source_for_run_with_runtime_root(
            root.path(),
            PROVIDER_KIRO,
            Some(&runtime_root),
        )
        .await
        .expect("resolve run with runtime root");

        let endpoint_home = kiro_endpoint_home(&runtime_root, &endpoint.id);
        assert!(endpoint_home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH).exists());

        delete_provider_endpoint(root.path(), PROVIDER_KIRO, &endpoint.id)
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
                base_url: Some("https://example.com".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
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
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
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
