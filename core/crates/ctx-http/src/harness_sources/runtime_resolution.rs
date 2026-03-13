use super::*;
use crate::provider_accounts::{
    apply_gemini_api_key_runtime_auth_env, apply_gemini_vertex_runtime_auth_env,
    write_gemini_auth_settings, GEMINI_AUTH_SELECTED_TYPE_API_KEY,
    GEMINI_AUTH_SELECTED_TYPE_VERTEX_AI, KIMI_SHARE_DIR_ENV,
};

pub(crate) fn droid_cli_model_id_for_endpoint_model(
    model_id: Option<&str>,
    base_url: Option<&str>,
) -> Option<String> {
    let model_id = model_id?;
    let base_url = base_url?;
    let display_name = droid_custom_model_display_name(model_id, base_url)?;
    droid_cli_model_id_from_display_name(&display_name)
}

pub async fn resolve_provider_source_for_probe(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_provider_source_for_probe_with_runtime_root(data_root, provider_id, None).await
}

pub async fn resolve_provider_source_for_probe_with_runtime_root(
    data_root: &Path,
    provider_id: &str,
    runtime_data_root: Option<&Path>,
) -> Result<ResolvedHarnessSource> {
    resolve_internal(data_root, provider_id, false, runtime_data_root).await
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
    let require_verified_endpoint = validation::normalize_provider_id(provider_id)
        .is_some_and(validation::provider_requires_verified_endpoint_for_run);
    resolve_internal(
        data_root,
        provider_id,
        require_verified_endpoint,
        runtime_data_root,
    )
    .await
}

pub(super) struct ProviderRuntimeContext<'a> {
    canonical: &'static str,
    data_root: &'a Path,
    runtime_data_root: Option<&'a Path>,
}

impl<'a> ProviderRuntimeContext<'a> {
    pub(super) fn new(
        canonical: &'static str,
        data_root: &'a Path,
        runtime_data_root: Option<&'a Path>,
    ) -> Self {
        Self {
            canonical,
            data_root,
            runtime_data_root,
        }
    }

    fn runtime_data_root(&self) -> &'a Path {
        self.runtime_data_root.unwrap_or(self.data_root)
    }

    fn subscription_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        if self.canonical == PROVIDER_AMP {
            let home = amp_subscription_home(self.data_root, self.runtime_data_root);
            env.insert("HOME".to_string(), home.to_string_lossy().to_string());
            env.insert(
                "XDG_CONFIG_HOME".to_string(),
                home.join(".config").to_string_lossy().to_string(),
            );
            env.insert(
                "XDG_CACHE_HOME".to_string(),
                home.join(".cache").to_string_lossy().to_string(),
            );
        }
        env
    }

    pub(super) async fn cleanup_endpoint_runtime(&self, endpoint_id: &str) -> Result<()> {
        let Some(endpoint_home) = (match self.canonical {
            PROVIDER_CODEX => Some(codex_endpoint_home(self.data_root, endpoint_id)),
            PROVIDER_KIMI => Some(kimi_endpoint_home(self.data_root, endpoint_id)),
            PROVIDER_QWEN => Some(qwen_endpoint_home(self.data_root, endpoint_id)),
            PROVIDER_GEMINI => Some(gemini_endpoint_home(self.data_root, endpoint_id)),
            PROVIDER_DROID => Some(droid_endpoint_home(self.data_root, endpoint_id)),
            _ => None,
        }) else {
            return Ok(());
        };

        validation::ensure_safe_endpoint_id(endpoint_id)?;
        match tokio::fs::remove_dir_all(&endpoint_home).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "removing {} endpoint home for endpoint {}",
                        self.canonical, endpoint_id
                    )
                });
            }
        }
        Ok(())
    }

    async fn endpoint_env(
        &self,
        endpoint: &HarnessEndpointRecordInternal,
        secret: &secrets::EndpointSecretMaterial,
    ) -> Result<HashMap<String, String>> {
        let mut env = HashMap::new();

        match self.canonical {
            PROVIDER_CODEX => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let codex_home = codex_endpoint_home(self.data_root, &endpoint.id);
                prepare_codex_home_with_api_key(&codex_home, &api_key).await?;
                env.insert(
                    "CODEX_HOME".to_string(),
                    codex_home.to_string_lossy().to_string(),
                );
                env.insert("OPENAI_API_KEY".to_string(), api_key);
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            PROVIDER_CLAUDE => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("ANTHROPIC_API_KEY".to_string(), api_key);
                env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);
            }
            PROVIDER_GEMINI => {
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let gemini_home = gemini_endpoint_home(self.runtime_data_root(), &endpoint.id);
                let gemini_dir = gemini_home.join(".gemini");
                tokio::fs::create_dir_all(&gemini_dir)
                    .await
                    .with_context(|| {
                        format!("creating gemini endpoint home for endpoint {}", endpoint.id)
                    })?;
                env.insert(
                    "HOME".to_string(),
                    gemini_home.to_string_lossy().to_string(),
                );
                env.insert(
                    "GEMINI_CLI_HOME".to_string(),
                    gemini_home.to_string_lossy().to_string(),
                );
                env.insert("GEMINI_FORCE_FILE_STORAGE".to_string(), "true".to_string());
                match endpoint.auth_type.as_str() {
                    GEMINI_AUTH_TYPE_VERTEX_AI => {
                        let vertex_secret = secrets::endpoint_secret_gemini_vertex(secret)?;
                        let credentials_path = gemini_dir.join("vertex-service-account.json");
                        tokio::fs::write(
                            &credentials_path,
                            vertex_secret.service_account_json.as_bytes(),
                        )
                        .await
                        .with_context(|| {
                            format!(
                                "writing Gemini Vertex service account JSON {}",
                                credentials_path.display()
                            )
                        })?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = tokio::fs::set_permissions(
                                &credentials_path,
                                std::fs::Permissions::from_mode(0o600),
                            )
                            .await;
                        }
                        apply_gemini_vertex_runtime_auth_env(
                            &mut env,
                            credentials_path,
                            vertex_secret.project_id,
                            vertex_secret.location,
                        );
                        write_gemini_auth_settings(
                            &gemini_dir,
                            GEMINI_AUTH_SELECTED_TYPE_VERTEX_AI,
                        )
                        .await?;
                    }
                    GEMINI_AUTH_TYPE_GEMINI_API_KEY => {
                        let api_key = secrets::endpoint_secret_api_key(secret)?;
                        apply_gemini_api_key_runtime_auth_env(&mut env, api_key);
                        write_gemini_auth_settings(&gemini_dir, GEMINI_AUTH_SELECTED_TYPE_API_KEY)
                            .await?;
                    }
                    _ => {
                        anyhow::bail!(
                            "unsupported gemini endpoint auth_type '{}' (use '{}' or '{}')",
                            endpoint.auth_type,
                            GEMINI_AUTH_TYPE_GEMINI_API_KEY,
                            GEMINI_AUTH_TYPE_VERTEX_AI
                        );
                    }
                }
            }
            PROVIDER_KIMI => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                let model_id = endpoint_preferred_model_id(endpoint).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Kimi endpoint '{}' requires a model override or discovered model catalog before launch",
                        endpoint.name
                    )
                })?;
                let kimi_share_dir =
                    prepare_kimi_share_dir(self.runtime_data_root(), &endpoint.id).await?;
                env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
                env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_MODEL".to_string(), model_id.clone());
                env.insert("KIMI_API_KEY".to_string(), api_key);
                env.insert("KIMI_BASE_URL".to_string(), base_url);
                env.insert("KIMI_MODEL_NAME".to_string(), model_id);
                env.insert(
                    KIMI_SHARE_DIR_ENV.to_string(),
                    kimi_share_dir.to_string_lossy().to_string(),
                );
                env.insert(
                    "CTX_CRP_DISABLE_MODEL_OVERRIDE".to_string(),
                    "1".to_string(),
                );
            }
            PROVIDER_QWEN => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let qwen_home = qwen_endpoint_home(self.runtime_data_root(), &endpoint.id);
                prepare_qwen_home_with_openai_settings(&qwen_home).await?;
                env.insert("HOME".to_string(), qwen_home.to_string_lossy().to_string());
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
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                let provider_namespace =
                    model_catalog::infer_endpoint_model_provider_namespace(&base_url)
                        .unwrap_or_else(|| "endpoint".to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
                env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                if provider_namespace == "openrouter" {
                    env.insert("OPENROUTER_API_KEY".to_string(), api_key.clone());
                    env.insert("OPENROUTER_BASE_URL".to_string(), base_url.clone());
                }

                let mut provider_config = serde_json::Map::new();
                provider_config.insert(
                    provider_namespace.clone(),
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
                    root.insert(
                        "model".to_string(),
                        serde_json::Value::String(
                            model_catalog::normalize_namespaced_model_override(
                                &model,
                                Some(provider_namespace.as_str()),
                            ),
                        ),
                    );
                }
                root.insert(
                    "permission".to_string(),
                    serde_json::json!({
                        "edit": "deny",
                        "bash": "allow",
                    }),
                );
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
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
                env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_HOST".to_string(), base_url.clone());
                env.insert("OPENROUTER_API_KEY".to_string(), api_key);
                env.insert("OPENROUTER_BASE_URL".to_string(), base_url);
                env.insert("GOOSE_PROVIDER".to_string(), "openrouter".to_string());
                env.insert("GOOSE_DISABLE_KEYRING".to_string(), "1".to_string());
                if let Some(model) = endpoint
                    .model_override
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                {
                    env.insert("GOOSE_MODEL".to_string(), model.clone());
                    env.insert("OPENAI_MODEL".to_string(), model.clone());
                    env.insert("OPENROUTER_MODEL".to_string(), model);
                }
            }
            PROVIDER_MISTRAL => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("MISTRAL_API_KEY".to_string(), api_key.clone());
                env.insert("MISTRAL_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_API_KEY".to_string(), api_key);
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            PROVIDER_AMP => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("AMP_API_KEY".to_string(), api_key);
            }
            PROVIDER_DROID => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let droid_home = droid_endpoint_home(self.runtime_data_root(), &endpoint.id);
                let model_id = endpoint_preferred_model_id(endpoint).ok_or_else(|| {
                    anyhow::anyhow!(
                        "selected endpoint '{}' for {} is missing a concrete model id",
                        endpoint.name,
                        self.canonical
                    )
                })?;
                let droid_default_model = prepare_droid_home_with_endpoint_settings(
                    &droid_home,
                    &base_url,
                    &api_key,
                    &model_id,
                )
                .await?;
                env.insert("HOME".to_string(), droid_home.to_string_lossy().to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key);
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
                if let Ok(factory_api_key) = std::env::var("FACTORY_API_KEY") {
                    let trimmed = factory_api_key.trim();
                    if !trimmed.is_empty() {
                        env.insert("FACTORY_API_KEY".to_string(), trimmed.to_string());
                    }
                }
                if let Some(model) = droid_default_model {
                    env.insert("DROID_DEFAULT_MODEL".to_string(), model);
                }
            }
            PROVIDER_OPENHANDS => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
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
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("GH_TOKEN".to_string(), api_key.clone());
                env.insert("GITHUB_TOKEN".to_string(), api_key);
            }
            PROVIDER_PI => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                let provider =
                    model_catalog::infer_endpoint_model_provider_namespace(&endpoint.base_url)
                        .unwrap_or_else(|| "openai".to_string());
                env.insert("PI_ACP_PROVIDER".to_string(), provider);
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
            PROVIDER_AUGGIE => {
                let api_key = secrets::endpoint_secret_api_key(secret)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("AUGMENT_SESSION_AUTH".to_string(), api_key.clone());
                env.insert("AUGMENT_API_TOKEN".to_string(), api_key);
            }
            _ => {}
        }

        Ok(env)
    }
}

async fn resolve_internal(
    data_root: &Path,
    provider_id: &str,
    require_verified_endpoint: bool,
    runtime_data_root: Option<&Path>,
) -> Result<ResolvedHarnessSource> {
    if provider_id == "fake" {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: HashMap::new(),
        });
    }
    let canonical = match validation::normalize_provider_id(provider_id) {
        Some(id) => id,
        None => {
            return Ok(ResolvedHarnessSource {
                source_kind: HarnessSourceKind::Subscription,
                endpoint: None,
                env: HashMap::new(),
            });
        }
    };
    let runtime = ProviderRuntimeContext::new(canonical, data_root, runtime_data_root);

    let registry = registry::load_registry(data_root).await?;
    let provider = registry::provider_store(&registry, canonical)
        .cloned()
        .unwrap_or_default();

    if provider.selected_source_kind != HarnessSourceKind::Endpoint {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: runtime.subscription_env(),
        });
    }

    if !validation::provider_supports_harness_endpoint(canonical) {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: runtime.subscription_env(),
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

    let secret = secrets::read_endpoint_secret(data_root, &endpoint.secret_ref).await?;
    let env = runtime.endpoint_env(&endpoint, &secret).await?;

    let public = selection::public_endpoint_from_internal(&endpoint);
    Ok(ResolvedHarnessSource {
        source_kind: HarnessSourceKind::Endpoint,
        endpoint: Some(public),
        env,
    })
}

pub(super) fn codex_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("codex")
        .join("endpoint-homes")
        .join(endpoint_id)
}

pub(super) fn qwen_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("qwen")
        .join("endpoint-homes")
        .join(endpoint_id)
}

pub(super) fn kimi_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("kimi")
        .join("endpoint-homes")
        .join(endpoint_id)
}

pub(super) fn gemini_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("gemini")
        .join("endpoint-homes")
        .join(endpoint_id)
}

pub(super) fn droid_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("droid")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn amp_subscription_home(data_root: &Path, runtime_data_root: Option<&Path>) -> PathBuf {
    runtime_data_root
        .unwrap_or(data_root)
        .join("providers")
        .join("amp")
        .join("home")
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

async fn prepare_qwen_home_with_openai_settings(qwen_home: &Path) -> Result<()> {
    let qwen_config = qwen_home.join(".qwen");
    tokio::fs::create_dir_all(&qwen_config).await?;
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "$version": 2,
        "security": {
            "auth": {
                "selectedType": "openai"
            }
        }
    }))?;
    tokio::fs::write(qwen_config.join("settings.json"), payload).await?;
    Ok(())
}

async fn prepare_kimi_share_dir(runtime_data_root: &Path, endpoint_id: &str) -> Result<PathBuf> {
    let share_dir = kimi_endpoint_home(runtime_data_root, endpoint_id).join(".kimi");
    let credentials_dir = share_dir.join("credentials");
    tokio::fs::create_dir_all(&credentials_dir).await?;
    // Kimi currently refuses endpoint/API-key sessions unless a file-backed token exists.
    // Seed a benign token in the isolated endpoint runtime home so the CLI reaches the
    // actual endpoint-auth path instead of aborting with auth_required before turn start.
    let token_path = credentials_dir.join("kimi-code.json");
    let token = serde_json::json!({
        "access_token": "ctx-endpoint-access-token",
        "refresh_token": "ctx-endpoint-refresh-token",
        "expires_at": 4_102_444_800.0,
        "scope": "openid profile",
        "token_type": "Bearer",
    });
    tokio::fs::write(&token_path, token.to_string()).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            tokio::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(share_dir)
}

fn endpoint_preferred_model_id(endpoint: &HarnessEndpointRecordInternal) -> Option<String> {
    endpoint
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            endpoint.manual_model_ids.iter().find_map(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
        })
        .or_else(|| {
            endpoint.model_catalog_models.iter().find_map(|record| {
                let trimmed = record.id.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
        })
}

fn droid_custom_model_name(model_id: &str) -> Option<String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_prefix = trimmed
        .strip_prefix("custom:")
        .map(str::trim)
        .unwrap_or(trimmed);
    if without_prefix.is_empty() {
        None
    } else {
        Some(without_prefix.to_string())
    }
}

fn droid_backend_model_id(model_id: &str) -> Option<String> {
    droid_custom_model_name(model_id)
}

fn droid_custom_model_display_name(model_id: &str, base_url: &str) -> Option<String> {
    let backend_model_id = droid_backend_model_id(model_id)?;
    let namespace = model_catalog::infer_endpoint_model_provider_namespace(base_url)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "endpoint".to_string());
    Some(format!("{backend_model_id} [{namespace}]"))
}

fn droid_cli_model_id_from_display_name(display_name: &str) -> Option<String> {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "custom:{}-0",
        trimmed.split_whitespace().collect::<Vec<_>>().join("-")
    ))
}

fn droid_host_auth_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var(CTX_DROID_HOST_AUTH_PATH_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    let base = BaseDirs::new().ok_or_else(|| anyhow::anyhow!("missing home dir"))?;
    Ok(base.home_dir().join(".factory").join("auth.encrypted"))
}

pub(super) async fn seed_droid_auth_from_host_path(
    droid_home: &Path,
    host_auth_path: &Path,
) -> Result<bool> {
    if !host_auth_path.exists() {
        return Ok(false);
    }
    let bytes = tokio::fs::read(host_auth_path).await?;
    if bytes.is_empty() {
        anyhow::bail!(
            "host droid auth file is empty: {}",
            host_auth_path.display()
        );
    }

    let droid_config = droid_home.join(".factory");
    tokio::fs::create_dir_all(&droid_config).await?;
    let dest = droid_config.join("auth.encrypted");
    let should_write = match tokio::fs::read(&dest).await {
        Ok(existing) => existing != bytes,
        Err(_) => true,
    };
    if should_write {
        tokio::fs::write(&dest, &bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600)).await;
        }
    }

    Ok(should_write)
}

async fn maybe_seed_droid_auth_from_host(droid_home: &Path) -> Result<bool> {
    let host_auth_path = droid_host_auth_path()?;
    seed_droid_auth_from_host_path(droid_home, &host_auth_path).await
}

async fn prepare_droid_home_with_endpoint_settings(
    droid_home: &Path,
    base_url: &str,
    api_key: &str,
    model_id: &str,
) -> Result<Option<String>> {
    let Some(display_name) = droid_custom_model_display_name(model_id, base_url) else {
        return Ok(None);
    };
    let Some(backend_model_id) = droid_backend_model_id(model_id) else {
        return Ok(None);
    };
    let droid_config = droid_home.join(".factory");
    tokio::fs::create_dir_all(&droid_config).await?;
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "customModels": [
            {
                "model": backend_model_id,
                "displayName": display_name,
                "provider": "generic-chat-completion-api",
                "baseUrl": base_url,
                "apiKey": api_key,
            }
        ]
    }))?;
    let settings_path = droid_config.join("settings.json");
    tokio::fs::write(&settings_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&settings_path, std::fs::Permissions::from_mode(0o600))
            .await;
    }
    let _ = maybe_seed_droid_auth_from_host(droid_home).await?;
    Ok(droid_cli_model_id_from_display_name(&display_name))
}
