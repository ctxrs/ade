use super::*;

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
        api_key: &str,
    ) -> Result<HashMap<String, String>> {
        let mut env = HashMap::new();

        match self.canonical {
            PROVIDER_CODEX => {
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let codex_home = codex_endpoint_home(self.data_root, &endpoint.id);
                prepare_codex_home_with_api_key(&codex_home, api_key).await?;
                env.insert(
                    "CODEX_HOME".to_string(),
                    codex_home.to_string_lossy().to_string(),
                );
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            PROVIDER_CLAUDE => {
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("ANTHROPIC_API_KEY".to_string(), api_key.to_string());
                env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);
            }
            PROVIDER_GEMINI => {
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let gemini_home = gemini_endpoint_home(self.runtime_data_root(), &endpoint.id);
                tokio::fs::create_dir_all(gemini_home.join(".gemini"))
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
                        env.insert("GOOGLE_API_KEY".to_string(), api_key.to_string());
                        env.insert("GOOGLE_GENAI_USE_VERTEXAI".to_string(), "true".to_string());
                    }
                    GEMINI_AUTH_TYPE_GEMINI_API_KEY => {
                        env.insert("GEMINI_API_KEY".to_string(), api_key.to_string());
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
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("KIMI_API_KEY".to_string(), api_key.to_string());
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
            PROVIDER_QWEN => {
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let qwen_home = qwen_endpoint_home(self.runtime_data_root(), &endpoint.id);
                prepare_qwen_home_with_openai_settings(&qwen_home).await?;
                env.insert("HOME".to_string(), qwen_home.to_string_lossy().to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
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
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                let provider_namespace =
                    model_catalog::infer_endpoint_model_provider_namespace(&base_url)
                        .unwrap_or_else(|| "endpoint".to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
                env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                if provider_namespace == "openrouter" {
                    env.insert("OPENROUTER_API_KEY".to_string(), api_key.to_string());
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
                    "provider".to_string(),
                    serde_json::Value::Object(provider_config),
                );
                env.insert(
                    "OPENCODE_CONFIG_CONTENT".to_string(),
                    serde_json::Value::Object(root).to_string(),
                );
            }
            PROVIDER_GOOSE => {
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
                env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_HOST".to_string(), base_url.clone());
                env.insert("OPENROUTER_API_KEY".to_string(), api_key.to_string());
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
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("MISTRAL_API_KEY".to_string(), api_key.to_string());
                env.insert("MISTRAL_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            PROVIDER_AMP => {
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("AMP_API_KEY".to_string(), api_key.to_string());
            }
            PROVIDER_DROID => {
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                validation::ensure_safe_endpoint_id(&endpoint.id)?;
                let droid_home = droid_endpoint_home(self.runtime_data_root(), &endpoint.id);
                let model_id = endpoint_preferred_model_id(endpoint)
                    .unwrap_or_else(|| "openai/gpt-5.2-codex".to_string());
                let droid_default_model = prepare_droid_home_with_endpoint_settings(
                    &droid_home,
                    &base_url,
                    api_key,
                    &model_id,
                )
                .await?;
                env.insert("HOME".to_string(), droid_home.to_string_lossy().to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
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
                let base_url = validation::endpoint_base_url_or_err(endpoint)?;
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("LLM_API_KEY".to_string(), api_key.to_string());
                env.insert("LLM_BASE_URL".to_string(), base_url.clone());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
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
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("GH_TOKEN".to_string(), api_key.to_string());
                env.insert("GITHUB_TOKEN".to_string(), api_key.to_string());
            }
            PROVIDER_PI => {
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("PI_ACP_PROVIDER".to_string(), "openai".to_string());
                env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
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
                validation::ensure_shape_compatible(self.canonical, endpoint.api_shape)?;
                env.insert("AUGMENT_SESSION_AUTH".to_string(), api_key.to_string());
                env.insert("AUGMENT_API_TOKEN".to_string(), api_key.to_string());
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

    let api_key = secrets::read_endpoint_secret(data_root, &endpoint.secret_ref).await?;
    let env = runtime.endpoint_env(&endpoint, &api_key).await?;

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
