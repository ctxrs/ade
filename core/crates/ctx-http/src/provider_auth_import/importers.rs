use super::*;

pub async fn list_provider_auth_profiles(
    data_root: &Path,
) -> Result<Vec<ProviderImportedAuthProfile>> {
    CanonicalAuthImporter::new(data_root).list_profiles().await
}

pub async fn import_provider_auth_candidates(
    data_root: &Path,
    candidate_ids: &[String],
) -> Result<Vec<ProviderAuthImportResult>> {
    let scanner = AuthImportScanner::discover()?;
    CanonicalAuthImporter::new(data_root)
        .import_candidates(&scanner, candidate_ids)
        .await
}

impl<'a> CanonicalAuthImporter<'a> {
    pub(super) fn new(data_root: &'a Path) -> Self {
        Self { data_root }
    }

    async fn list_profiles(&self) -> Result<Vec<ProviderImportedAuthProfile>> {
        self.migrate_legacy_imported_profiles_once().await?;
        let registry = legacy::load_imported_registry(self.data_root).await;
        Ok(registry.profiles)
    }

    async fn import_candidates(
        &self,
        scanner: &AuthImportScanner,
        candidate_ids: &[String],
    ) -> Result<Vec<ProviderAuthImportResult>> {
        if candidate_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.migrate_legacy_imported_profiles_once().await?;

        let materials = scanner.scan();
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
                    message: material.candidate.unsupported_reason.clone().or_else(|| {
                        Some("Candidate cannot be imported automatically.".to_string())
                    }),
                });
                continue;
            }

            match self.import_candidate_to_canonical(material).await {
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

    async fn import_candidate_to_canonical(
        &self,
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
            "codex" => import_codex_candidate(self.data_root, material).await,
            "gemini" => {
                if material.candidate.kind == "env_file" {
                    import_gemini_env_candidate(self.data_root, material).await
                } else {
                    import_gemini_auth_file_candidate(self.data_root, material).await
                }
            }
            "qwen" => import_qwen_candidate(self.data_root, material).await,
            "opencode" => import_opencode_candidate(self.data_root, material).await,
            "amp" => import_amp_candidate(self.data_root, material).await,
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

    pub(super) async fn migrate_legacy_imported_profiles_once(&self) -> Result<()> {
        if legacy::legacy_migration_marker_exists(self.data_root).await {
            return Ok(());
        }

        let mut registry = legacy::load_imported_registry(self.data_root).await;
        if registry.profiles.is_empty() {
            legacy::write_legacy_migration_marker(self.data_root).await?;
            return Ok(());
        }

        let mut remaining_profiles: Vec<ProviderImportedAuthProfile> = Vec::new();
        for profile in registry.profiles.iter().cloned() {
            let Some(secret_bytes) =
                legacy::read_legacy_secret_material_bytes(self.data_root, &profile.id).await
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
            let migrated = match self.import_candidate_to_canonical(&material).await {
                Ok(result) => matches!(
                    result.status.as_str(),
                    "imported" | "updated" | "already_imported"
                ),
                Err(_) => false,
            };
            if migrated {
                let _ = tokio::fs::remove_file(legacy::imported_secret_path(
                    self.data_root,
                    &profile.id,
                ))
                .await;
            } else {
                remaining_profiles.push(profile);
            }
        }

        registry.profiles = remaining_profiles;
        legacy::save_imported_registry(self.data_root, &registry).await?;
        if registry.profiles.is_empty() {
            let _ = tokio::fs::remove_dir_all(legacy::imported_secrets_dir(self.data_root)).await;
            legacy::write_legacy_migration_marker(self.data_root).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) async fn import_candidate_to_canonical(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    CanonicalAuthImporter::new(data_root)
        .import_candidate_to_canonical(material)
        .await
}

#[cfg(test)]
pub(super) async fn migrate_legacy_imported_profiles_once(data_root: &Path) -> Result<()> {
    CanonicalAuthImporter::new(data_root)
        .migrate_legacy_imported_profiles_once()
        .await
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
            | "amp"
            | "droid"
            | "cline"
            | "openhands"
            | "copilot"
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
            legacy::upsert_imported_profile_metadata(
                data_root,
                material,
                &found.endpoint_id,
                base_url.clone(),
                auth_type.clone(),
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
    legacy::upsert_imported_profile_metadata(
        data_root,
        material,
        &endpoint.id,
        endpoint.base_url.clone(),
        Some(endpoint.auth_type.clone()),
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

pub(super) async fn import_codex_candidate(
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

    let imported_fingerprint = catalog::sha256_hex(bytes);
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
                catalog::sha256_hex(&existing) == imported_fingerprint
            };
            if matches_auth {
                legacy::upsert_imported_profile_metadata(
                    data_root,
                    material,
                    &account.id,
                    None,
                    Some(account.kind.clone()),
                )
                .await?;
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
        kind: kind.clone(),
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
    legacy::upsert_imported_profile_metadata(data_root, material, &account_id, None, Some(kind))
        .await?;

    Ok(ProviderAuthImportResult {
        candidate_id: material.candidate.id.clone(),
        provider_id: "codex".to_string(),
        status: "imported".to_string(),
        profile_id: Some(account_id),
        message: Some("Codex auth imported and available for new turns.".to_string()),
    })
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
            Ok(contents) => parsers::trim_to_option(&contents),
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
            .and_then(|raw| parsers::trim_to_option(&raw))
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
    if let Some(profile_id) = registry.active_account_id.clone() {
        legacy::upsert_imported_profile_metadata(
            data_root,
            material,
            &profile_id,
            None,
            Some("subscription".to_string()),
        )
        .await?;
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
        let env_map = parsers::parse_env_file(&String::from_utf8_lossy(bytes));
        if let Some(key) = parsers::env_value_case_insensitive(&env_map, &["GOOGLE_API_KEY"]) {
            let auth_type = if parsers::gemini_env_uses_vertex_ai(&env_map) {
                "vertex_ai"
            } else {
                "gemini_api_key"
            };
            (key, None, Some(auth_type.to_string()))
        } else if let Some(key) = parsers::env_value_case_insensitive(&env_map, &["GEMINI_API_KEY"])
        {
            (key, None, Some("gemini_api_key".to_string()))
        } else {
            let base_url = parsers::env_value_case_insensitive(
                &env_map,
                &["OPENAI_BASE_URL", "BASE_URL", "CTX_GATEWAY_BASE_URL"],
            );
            if base_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                anyhow::bail!(
                    "Gemini OpenAI-compatible endpoint imports are not supported; use Gemini OAuth or GEMINI_API_KEY"
                );
            }
            if let Some(key) = parsers::env_value_case_insensitive(&env_map, &["OPENAI_API_KEY"]) {
                (key, None, Some("gemini_api_key".to_string()))
            } else {
                anyhow::bail!(
                    "No importable Gemini API key found (expected GEMINI_API_KEY, GOOGLE_API_KEY, or OPENAI_API_KEY)"
                );
            }
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
    let (api_key, base_url) = parsers::parse_endpoint_env_candidate(
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
    let (api_key, base_url, model_override) =
        parsers::parse_endpoint_json_candidate("opencode", material)?;
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
    let (api_key, base_url, model_override) =
        parsers::parse_endpoint_json_candidate("amp", material)?;
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
