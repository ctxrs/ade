use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointSecretEnvelope {
    version: u32,
    api_key: String,
}

pub(super) fn endpoint_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    endpoint_secret_dir(data_root).join(secret_ref)
}

pub(super) async fn write_endpoint_secret(
    data_root: &Path,
    secret_ref: &str,
    api_key: &str,
) -> Result<()> {
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

pub(super) async fn read_endpoint_secret(data_root: &Path, secret_ref: &str) -> Result<String> {
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

fn endpoint_secret_dir(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("harness_endpoints")
}
