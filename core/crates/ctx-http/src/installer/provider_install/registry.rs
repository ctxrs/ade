use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn update_registry_last_error(
    data_root: &Path,
    provider_id: &str,
    stage: &str,
    err: &anyhow::Error,
    code: InstallErrorCode,
    package: Option<&str>,
    version: Option<&str>,
    install_dir_rel: Option<String>,
    target: Option<InstallTarget>,
) {
    let install_dir_rel_clone = install_dir_rel.clone();
    let _ = mutate_agent_server_config(data_root, move |cfg| {
        let mut meta =
            cfg.managed_installs
                .get(provider_id)
                .cloned()
                .unwrap_or(ManagedInstallMetadata {
                    package: package.map(|s| s.to_string()),
                    version: version.map(|s| s.to_string()),
                    target,
                    install_dir_rel: install_dir_rel_clone,
                    bin_dir_rel: None,
                    last_success_at: None,
                    last_error: None,
                });
        if meta.package.is_none() {
            meta.package = package.map(|s| s.to_string());
        }
        if meta.version.is_none() {
            meta.version = version.map(|s| s.to_string());
        }
        if meta.install_dir_rel.is_none() {
            meta.install_dir_rel = install_dir_rel;
        }
        if meta.target.is_none() {
            meta.target = target;
        }

        meta.last_error = Some(ManagedInstallError {
            at: Utc::now().to_rfc3339(),
            stage: stage.to_string(),
            message: truncate_for_storage(&format!("{err:#}"), LAST_ERROR_MAX_LEN),
            code: Some(code),
        });
        cfg.managed_installs
            .insert(provider_id.to_string(), meta.clone());

        if let Some(entry) = cfg.providers.get_mut(provider_id) {
            entry.managed = Some(meta);
        }
    })
    .await;
}

pub(in crate::installer) async fn repair_install_dir(
    install_id: Option<InstallId>,
    state: &AppState,
    provider_id: &str,
    install_dir: &Path,
    expected_entrypoint_rel: &str,
) -> Result<()> {
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "prepare",
        format!("Preparing install dir: {}", install_dir.display()),
        None,
        None,
        None,
    )
    .await;

    // Repair semantics: remove obviously-corrupted partial installs.
    if install_dir.exists() {
        let expected = install_dir.join(expected_entrypoint_rel);
        let node_modules = install_dir.join("node_modules");
        if !node_modules.exists() || !expected.exists() {
            tokio::fs::remove_dir_all(install_dir).await.ok();
        }
    }
    tokio::fs::create_dir_all(install_dir)
        .await
        .with_context(|| format!("creating install dir: {}", install_dir.display()))?;
    Ok(())
}
