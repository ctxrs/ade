use super::shared::{
    import_endpoint_candidate, import_result, set_subscription_source_if_supported,
};
use super::*;

pub(crate) async fn import_codex_candidate(
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
    let mut registry = provider_accounts::load_codex_registry(data_root).await?;

    for account in &registry.accounts {
        provider_accounts::hydrate_codex_account_home_from_secret(data_root, &account.id).await?;
        let auth_path =
            provider_accounts::codex_account_dir(data_root, &account.id).join("auth.json");
        match tokio::fs::read(&auth_path).await {
            Ok(existing) => {
                let matches_auth = if let Some(imported_auth) = imported_auth.as_ref() {
                    let existing_auth = serde_json::from_slice::<serde_json::Value>(&existing)
                        .with_context(|| {
                            format!("invalid codex auth JSON at {}", auth_path.display())
                        })?;
                    existing_auth == *imported_auth
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
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("reading existing codex auth at {}", auth_path.display())
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

pub(super) async fn import_gemini_auth_file_candidate(
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
        .await?
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

pub(super) async fn import_gemini_env_candidate(
    data_root: &Path,
    material: &CandidateMaterial,
) -> Result<ProviderAuthImportResult> {
    let (api_key, base_url, auth_type) = {
        let Some(bytes) = material.secret_bytes.as_ref() else {
            anyhow::bail!("No importable auth material.");
        };
        let env_map = parsers::parse_env_file(&String::from_utf8_lossy(bytes));
        if let Some(key) = parsers::env_value_case_insensitive(&env_map, &["GOOGLE_API_KEY"]) {
            if parsers::gemini_env_uses_vertex_ai(&env_map) {
                anyhow::bail!(
                    "Gemini Vertex env imports require service_account_json; re-enter Vertex AI credentials in Settings"
                );
            }
            (key, None, Some("gemini_api_key".to_string()))
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

pub(super) async fn import_qwen_candidate(
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

pub(super) async fn import_opencode_candidate(
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

pub(super) async fn import_amp_candidate(
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
