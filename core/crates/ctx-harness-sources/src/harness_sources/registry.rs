use super::*;

pub(super) async fn load_registry(data_root: &Path) -> Result<HarnessSourceRegistryInternal> {
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

pub(super) async fn save_registry(
    data_root: &Path,
    registry: &HarnessSourceRegistryInternal,
) -> Result<()> {
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

pub(super) fn provider_store<'a>(
    registry: &'a HarnessSourceRegistryInternal,
    provider_id: &str,
) -> Option<&'a HarnessProviderConfigInternal> {
    registry.providers.get(provider_id)
}

pub(super) fn registry_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json")
}
