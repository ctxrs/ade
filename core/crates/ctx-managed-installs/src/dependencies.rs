use super::*;

pub(super) async fn install_managed_npm_dependency(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    dependency_id: &str,
    package: &str,
    version: &str,
    stage: &mut &'static str,
) -> Result<ManagedDependencyInstall> {
    let data_root = state.data_root().to_path_buf();
    let install_dir = data_root
        .join("providers")
        .join("agent-servers")
        .join(dependency_id)
        .join(version);
    let install_dir_rel_value = install_dir_rel(&data_root, &install_dir);
    let bin_dir = install_dir.join("node_modules").join(".bin");
    let bin_dir_rel_value = install_dir_rel(&data_root, &bin_dir);

    if install_dir.exists() {
        if npm_dependency_matches(&install_dir, package, version)
            .await
            .unwrap_or(false)
            && bin_dir.exists()
        {
            let meta = ManagedInstallMetadata {
                package: Some(package.to_string()),
                version: Some(version.to_string()),
                artifact_fingerprint: npm_artifact_fingerprint(package, version),
                archive_sha256: None,
                target: Some(InstallTarget::Host),
                install_dir_rel: Some(install_dir_rel_value.clone()),
                bin_dir_rel: Some(bin_dir_rel_value.clone()),
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            return Ok(ManagedDependencyInstall { meta });
        }
        tokio::fs::remove_dir_all(&install_dir).await.ok();
    }

    *stage = "dependency_node";
    let node = ensure_node_runtime(
        state,
        install_id,
        provider_id,
        &data_root,
        InstallTarget::Host,
    )
    .await
    .context("ensuring managed Node runtime")?;

    *stage = "dependency_prepare";
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "dependency_prepare",
        format!("Preparing dependency {dependency_id}…"),
        None,
        None,
        None,
    )
    .await;
    tokio::fs::create_dir_all(&install_dir)
        .await
        .with_context(|| format!("creating install dir: {}", install_dir.display()))?;

    *stage = "dependency_npm_install";
    npm_install(
        state,
        install_id,
        provider_id,
        &node,
        &install_dir,
        &format!("{package}@{version}"),
        InstallTarget::Host,
    )
    .await
    .context("running package install for dependency")?;

    if !bin_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await.ok();
        anyhow::bail!(
            "dependency install completed but bin dir missing: {}",
            bin_dir.display()
        );
    }

    let meta = ManagedInstallMetadata {
        package: Some(package.to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: npm_artifact_fingerprint(package, version),
        archive_sha256: None,
        target: Some(InstallTarget::Host),
        install_dir_rel: Some(install_dir_rel_value),
        bin_dir_rel: Some(bin_dir_rel_value),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedDependencyInstall { meta })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn install_managed_archive_dependency(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    dependency_id: &str,
    version: &str,
    url: &str,
    expected_sha256: Option<&str>,
    archive: AgentServerArchive,
    bin_path: &str,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<ManagedDependencyInstall> {
    let data_root = state.data_root().to_path_buf();
    let install_dir = install_dir_for_provider(&data_root, dependency_id, version, target);

    let existing = if install_dir.exists() {
        let direct = install_dir.join(bin_path);
        if direct.exists() {
            Some(direct)
        } else {
            find_unique_path_ending_with(&install_dir, bin_path).ok()
        }
    } else {
        None
    };

    let bin = if let Some(bin) = existing {
        ensure_executable(&bin)?;
        bin
    } else {
        install_agent_server_url_binary(
            state,
            install_id,
            dependency_id,
            provider_id,
            version,
            url,
            expected_sha256,
            archive,
            bin_path,
            target,
            stage,
        )
        .await
        .context("installing dependency binary")?
    };

    let bin_dir = bin.parent().unwrap_or(&install_dir).to_path_buf();
    let meta = ManagedInstallMetadata {
        package: Some(url.to_string()),
        version: Some(version.to_string()),
        artifact_fingerprint: expected_sha256.map(str::to_string),
        archive_sha256: expected_sha256.map(str::to_string),
        target: Some(target),
        install_dir_rel: Some(install_dir_rel(&data_root, &install_dir)),
        bin_dir_rel: Some(install_dir_rel(&data_root, &bin_dir)),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    };

    Ok(ManagedDependencyInstall { meta })
}

pub(super) fn resolve_install_args(args: &[String]) -> Vec<String> {
    args.to_vec()
}

pub(super) fn map_archive_kind(kind: provider_matrix::ProviderArchiveKind) -> AgentServerArchive {
    match kind {
        provider_matrix::ProviderArchiveKind::None => AgentServerArchive::None,
        provider_matrix::ProviderArchiveKind::TarGz => AgentServerArchive::TarGz,
        provider_matrix::ProviderArchiveKind::TarBz2 => AgentServerArchive::TarBz2,
        provider_matrix::ProviderArchiveKind::Zip => AgentServerArchive::Zip,
        provider_matrix::ProviderArchiveKind::Dmg => AgentServerArchive::Dmg,
    }
}
