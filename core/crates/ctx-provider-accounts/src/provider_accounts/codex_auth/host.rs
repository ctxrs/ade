use super::secret_store::{
    codex_auth_has_supported_shape, codex_auth_kind, project_auth_value_to_home,
};
use super::*;

pub fn host_codex_auth_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var(CTX_CODEX_HOST_AUTH_PATH_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    let base = directories::BaseDirs::new().ok_or_else(|| anyhow!("missing home dir"))?;
    Ok(base.home_dir().join(".codex").join("auth.json"))
}

pub fn seeding_codex_auth_from_host_enabled() -> bool {
    matches!(
        std::env::var(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV)
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

pub async fn seed_codex_auth_from_host(codex_home: &Path) -> Result<bool> {
    if !seeding_codex_auth_from_host_enabled() {
        return Ok(false);
    }
    let src = host_codex_auth_path()?;
    if !src.exists() {
        anyhow::bail!(
            "Codex auth seeding is enabled ({CTX_SEED_CODEX_AUTH_FROM_HOST_ENV}=1) but host auth file is missing at {}",
            src.display()
        );
    }
    let payload = tokio::fs::read_to_string(&src)
        .await
        .with_context(|| format!("reading host codex auth at {}", src.display()))?;
    if payload.trim().is_empty() {
        anyhow::bail!(
            "Codex auth seeding is enabled ({CTX_SEED_CODEX_AUTH_FROM_HOST_ENV}=1) but host auth file is empty at {}",
            src.display()
        );
    }
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", src.display()))?;
    if !codex_auth_has_supported_shape(&auth) {
        anyhow::bail!(
            "codex auth file at {} has no OPENAI_API_KEY or tokens.access_token/tokens.refresh_token",
            src.display()
        );
    }
    project_auth_value_to_home(codex_home, &auth).await
}

pub async fn probe_host_codex_auth_candidate() -> CodexHostImportProbe {
    let path = match host_codex_auth_path() {
        Ok(path) => path,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: None,
                auth_kind: None,
                error: Some(err.to_string()),
            };
        }
    };
    if !path.exists() {
        return CodexHostImportProbe {
            available: false,
            path: Some(path.display().to_string()),
            auth_kind: None,
            error: None,
        };
    }
    let payload = match tokio::fs::read_to_string(&path).await {
        Ok(payload) => payload,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: Some(path.display().to_string()),
                auth_kind: None,
                error: Some(err.to_string()),
            };
        }
    };
    let auth: serde_json::Value = match serde_json::from_str(&payload) {
        Ok(auth) => auth,
        Err(err) => {
            return CodexHostImportProbe {
                available: false,
                path: Some(path.display().to_string()),
                auth_kind: None,
                error: Some(format!("invalid JSON: {err}")),
            };
        }
    };
    let auth_kind = codex_auth_kind(&auth);
    if auth_kind.is_none() {
        return CodexHostImportProbe {
            available: false,
            path: Some(path.display().to_string()),
            auth_kind: None,
            error: Some(
                "unsupported auth shape; expected OPENAI_API_KEY or tokens.access_token+tokens.refresh_token"
                    .to_string(),
            ),
        };
    }
    CodexHostImportProbe {
        available: true,
        path: Some(path.display().to_string()),
        auth_kind,
        error: None,
    }
}

pub(super) async fn mirror_host_codex_auth_to_runtime_root(runtime_root: &Path) -> Result<bool> {
    let src = host_codex_auth_path()?;
    let payload = match tokio::fs::read_to_string(&src).await {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let auth: serde_json::Value = serde_json::from_str(&payload)
        .with_context(|| format!("invalid codex auth JSON at {}", src.display()))?;
    if !codex_auth_has_supported_shape(&auth) {
        return Ok(false);
    }
    let projected = project_auth_value_to_home(&codex_runtime_home(runtime_root), &auth).await?;
    let owner_path = codex_runtime_owner_path(runtime_root);
    match tokio::fs::remove_file(&owner_path).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(projected)
}
