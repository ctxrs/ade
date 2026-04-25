use super::*;
mod dependencies;
mod registry;

use self::dependencies::{
    install_provider_blocking_dependencies, wait_for_provider_readiness_dependencies,
};
pub(crate) use self::registry::repair_install_dir;
use self::registry::update_registry_last_error;

fn apply_managed_provider_install_to_cfg(
    cfg: &mut AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
    managed: &ManagedProviderInstall,
    dependency_ids: &[String],
    implicit_managed_dependencies: &[(String, ManagedInstallMetadata)],
) {
    for (dependency_id, metadata) in implicit_managed_dependencies {
        cfg.managed_installs
            .insert(dependency_id.clone(), metadata.clone());
    }
    cfg.managed_install_targets
        .entry(provider_id.to_string())
        .or_default()
        .insert(target.as_str().to_string(), managed.meta.clone());
    cfg.managed_provider_targets
        .entry(provider_id.to_string())
        .or_default()
        .insert(
            target.as_str().to_string(),
            AgentServerCommand {
                command: managed.command.clone(),
                args: managed.args.clone(),
                dependencies: dependency_ids.to_vec(),
                managed: Some(managed.meta.clone()),
            },
        );
    cfg.providers.remove(provider_id);
}

async fn wait_for_tracked_install(
    state: &AppState,
    install_id: InstallId,
    provider_id: &str,
    target: InstallTarget,
    parent_install_id: Option<InstallId>,
) -> Result<()> {
    loop {
        ensure_install_not_cancelled(state, parent_install_id).await?;
        let info = state.get_install_info(install_id).await.ok_or_else(|| {
            anyhow::anyhow!(
                "tracked install {} for provider '{}' target '{}' is missing",
                install_id,
                provider_id,
                target.as_str()
            )
        })?;
        match info.state {
            InstallStateKind::Running => {
                tokio::time::sleep(INSTALL_REGISTRY_POLL_INTERVAL).await;
            }
            InstallStateKind::Succeeded => return Ok(()),
            InstallStateKind::Failed | InstallStateKind::Cancelled => {
                anyhow::bail!(
                    "tracked install {} for provider '{}' target '{}' {}: {}",
                    install_id,
                    provider_id,
                    target.as_str(),
                    match info.state {
                        InstallStateKind::Failed => "failed",
                        InstallStateKind::Cancelled => "was cancelled",
                        InstallStateKind::Running | InstallStateKind::Succeeded => unreachable!(),
                    },
                    info.error
                        .unwrap_or_else(|| "unknown install failure".to_string())
                );
            }
        }
    }
}

pub(super) async fn run_tracked_provider_install(
    state: &AppState,
    install_id: InstallId,
    provider_id: &str,
    target: InstallTarget,
) -> Result<()> {
    provider_matrix::invalidate_matrix_cache(state.provider_matrix_cache()).await;
    let res = Box::pin(install_provider_impl(
        state,
        provider_id,
        target,
        Some(install_id),
    ))
    .await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("provider_install", e);
            state
                .finish_install(
                    install_id,
                    false,
                    Some(truncate_for_storage(&format!("{e:#}"), 12_000)),
                    Some(code),
                )
                .await
        }
    }
    provider_matrix::invalidate_matrix_cache(state.provider_matrix_cache()).await;
    res
}

pub(super) async fn install_provider_impl(
    state: &AppState,
    provider_id: &str,
    target: InstallTarget,
    install_id: Option<InstallId>,
) -> Result<()> {
    if !MANAGED_PROVIDER_INSTALLS_ENABLED {
        anyhow::bail!(
            "managed provider installs are disabled; provider '{provider_id}' must be shipped in bundled harness assets",
        );
    }

    let provider_id = provider_id.to_string();
    let _provider_install_lock = acquire_provider_install_lock(&provider_id, target).await;
    let requested_target_label = target.as_str();
    let mut stage: &'static str = "start";
    let mut error_package: Option<String> = None;
    let mut error_version: Option<String> = None;
    let mut error_install_dir_rel: Option<String> = None;

    let res: Result<()> = async {
        let matrix = provider_matrix::load_matrix_cached(
            state.data_root(),
            state.provider_matrix_cache(),
        )
        .await;
        let current_ctx_version_raw = state
            .current_ctx_version()
            .ok_or_else(|| anyhow::anyhow!("current ctx build version unavailable"))?;
        let current_ctx_version = provider_matrix::parse_version_loose(&current_ctx_version_raw)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "current ctx build version is not valid semver: {current_ctx_version_raw}"
                )
            })?;
        let install_cfg = load_agent_server_config(state.data_root())
            .await
            .unwrap_or_default();
        let install_contract = provider_install_contract::resolve_provider_install_contract(
            state.data_root(),
            &install_cfg,
            &matrix,
            &provider_id,
            target,
            Some(current_ctx_version_raw.as_str()),
        )
        .map_err(anyhow::Error::new)?;
        let resolved_target_key = install_contract.resolved_target_key;
        let blocking_dependencies = install_contract.dependencies_for_role(
            provider_install_contract::ProviderInstallDependencyRoleKind::Prerequisite,
        );
        let readiness_dependencies = install_contract.dependencies_for_role(
            provider_install_contract::ProviderInstallDependencyRoleKind::Readiness,
        );

        ensure_install_not_cancelled(state, install_id).await?;
        if let Some(install_id) = install_id {
            state
                .update_install_start_event(
                    install_id,
                    &provider_id,
                    Some(target),
                    format!(
                        "Installing managed provider: {provider_id} (target: {requested_target_label}, resolved: {resolved_target_key})"
                    ),
                    false,
                )
                .await;
        }
        install_provider_blocking_dependencies(
            state,
            &provider_id,
            &blocking_dependencies,
            install_id,
        )
        .await?;

        let entry = provider_matrix::get_entry(&matrix, &provider_id)
            .ok_or_else(|| anyhow::anyhow!("unsupported provider for install: {provider_id}"))?;
        let Some(install) = entry.managed_install.as_ref() else {
            anyhow::bail!("provider has no managed install: {provider_id}");
        };
        let release = provider_matrix::recommended_release(entry, Some(&current_ctx_version))
            .ok_or_else(|| anyhow::anyhow!("no compatible release for provider: {provider_id}"))?;

        let mut dependency_ids: Vec<String> = install_contract
            .dependencies
            .iter()
            .map(|dependency| dependency.provider_id.clone())
            .collect();
        let mut implicit_managed_dependencies: Vec<(String, ManagedInstallMetadata)> = Vec::new();
        if !entry.dependencies.is_empty() {
            stage = "dependencies";
            ensure_install_not_cancelled(state, install_id).await?;
            emit_install(
                state,
                install_id,
                &provider_id,
                InstallEventLevel::Info,
                "dependencies",
                format!("Installing dependencies for {provider_id}"),
                None,
                None,
                None,
            )
            .await;

            for dep in &entry.dependencies {
                ensure_install_not_cancelled(state, install_id).await?;
                dependency_ids.push(dep.id.clone());
                let managed = match &dep.install {
                    provider_matrix::DependencyInstall::Npm { package, version } => {
                        if !matches!(target, InstallTarget::Host) {
                            anyhow::bail!(
                                "target '{}' is not supported for npm dependency '{}' (provider '{}'); use target host",
                                requested_target_label,
                                dep.id,
                                provider_id
                            );
                        }
                        error_package = Some(package.clone());
                        error_version = Some(version.clone());
                        error_install_dir_rel =
                            Some(format!("providers/agent-servers/{}/{}", dep.id, version));
                        install_managed_npm_dependency(
                            state,
                            install_id,
                            &provider_id,
                            &dep.id,
                            package,
                            version,
                            &mut stage,
                        )
                        .await?
                    }
                    provider_matrix::DependencyInstall::Archive { version, targets } => {
                        let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                            anyhow::anyhow!(
                                "unsupported dependency target {}: {}",
                                dep.id,
                                resolved_target_key
                            )
                        })?;
                        error_package = Some(target_entry.url.clone());
                        error_version = Some(version.clone());
                        error_install_dir_rel =
                            Some(format!("providers/agent-servers/{}/{}", dep.id, version));
                        install_managed_archive_dependency(
                            state,
                            install_id,
                            &provider_id,
                            &dep.id,
                            version,
                            &target_entry.url,
                            target_entry.sha256.as_deref(),
                            map_archive_kind(target_entry.archive),
                            &target_entry.bin_path,
                            target,
                            &mut stage,
                        )
                        .await?
                    }
                };

                mutate_agent_server_config(state.data_root(), |cfg| {
                    cfg.managed_installs
                        .insert(dep.id.clone(), managed.meta.clone());
                })
                    .await
                    .context("saving managed install registry")?;
            }
        }

        ensure_install_not_cancelled(state, install_id).await?;
        let managed = match install {
            provider_matrix::ProviderInstall::Npm {
                package,
                version,
                entrypoint,
                args,
                targets,
            } => {
                if !matches!(
                    target,
                    InstallTarget::Host
                        | InstallTarget::Container
                        | InstallTarget::LinuxAarch64
                        | InstallTarget::LinuxX8664
                ) {
                    anyhow::bail!(
                        "target '{requested_target_label}' is not supported for npm provider '{provider_id}' installs; use target host, container, linux-aarch64, or linux-x86_64"
                    );
                }
                if provider_matrix::normalize_version(version)
                    != provider_matrix::normalize_version(&release.version)
                {
                    anyhow::bail!(
                        "provider matrix version mismatch for {provider_id}: release={release_version} install={version}",
                        release_version = release.version,
                    );
                }
                let version = release.version.clone();
                error_package = Some(package.clone());
                error_version = Some(version.clone());
                error_install_dir_rel =
                    Some(format!("providers/agent-servers/{provider_id}/{version}"));
                if matches!(target, InstallTarget::Host)
                    && !targets.contains_key(resolved_target_key)
                {
                    install_managed_npm_provider(
                        state,
                        install_id,
                        &provider_id,
                        package,
                        &version,
                        entrypoint,
                        resolve_install_args(args),
                        target,
                        &mut stage,
                    )
                    .await?
                } else {
                    let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unsupported provider target {provider_id}: {resolved_target_key}"
                        )
                    })?;
                    let mut managed = install_managed_archive_provider(
                        state,
                        install_id,
                        &provider_id,
                        &version,
                        &target_entry.url,
                        target_entry.sha256.as_deref(),
                        map_archive_kind(target_entry.archive),
                        &target_entry.bin_path,
                        resolve_install_args(args),
                        target,
                        &mut stage,
                    )
                    .await?;
                    if archive_bin_requires_node_runtime(
                        &target_entry.bin_path,
                        Path::new(&managed.command),
                    ) {
                        stage = "node";
                        for dependency_target in node_runtime_dependency_targets_for_install_target(
                            target,
                            std::env::consts::OS,
                        ) {
                            let node = ensure_node_runtime(
                                state,
                                install_id,
                                &provider_id,
                                state.data_root(),
                                dependency_target,
                            )
                            .await
                            .context(
                                "ensuring managed Node runtime for archive-backed npm provider",
                            )?;
                            let dep_id = node_runtime_dependency_id(dependency_target);
                            if !dependency_ids.contains(&dep_id) {
                                dependency_ids.push(dep_id.clone());
                            }
                            implicit_managed_dependencies.push((
                                dep_id,
                                node_runtime_dependency_metadata(
                                    state.data_root(),
                                    &node,
                                    dependency_target,
                                ),
                            ));
                            if provider_id == "gemini" && dependency_target == target {
                                let entrypoint_path = managed.command.clone();
                                managed.command = node.node_bin.to_string_lossy().to_string();
                                managed.args.insert(0, entrypoint_path);
                            }
                        }
                    }
                    managed
                }
            }
            provider_matrix::ProviderInstall::Python {
                package,
                version,
                entrypoint,
                args,
                targets,
                python_version,
                python_build_tag,
            } => {
                if !matches!(
                    target,
                    InstallTarget::Host
                        | InstallTarget::Container
                        | InstallTarget::LinuxAarch64
                        | InstallTarget::LinuxX8664
                ) {
                    anyhow::bail!(
                        "target '{requested_target_label}' is not supported for python provider '{provider_id}' installs; use target host, container, linux-aarch64, or linux-x86_64",
                    );
                }
                if provider_matrix::normalize_version(version)
                    != provider_matrix::normalize_version(&release.version)
                {
                    anyhow::bail!(
                        "provider matrix version mismatch for {provider_id}: release={release_version} install={version}",
                        release_version = release.version,
                    );
                }
                error_package = Some(package.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{provider_id}/{version}",
                ));
                if matches!(target, InstallTarget::Host)
                    && !targets.contains_key(resolved_target_key)
                {
                    install_managed_python_provider(
                        state,
                        install_id,
                        &provider_id,
                        package,
                        version,
                        entrypoint,
                        python_version.as_deref(),
                        python_build_tag.as_deref(),
                        resolve_install_args(args),
                        target,
                        &mut stage,
                    )
                    .await?
                } else {
                    let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unsupported provider target {provider_id}: {resolved_target_key}"
                        )
                    })?;
                    install_managed_archive_provider(
                        state,
                        install_id,
                        &provider_id,
                        version,
                        &target_entry.url,
                        target_entry.sha256.as_deref(),
                        map_archive_kind(target_entry.archive),
                        &target_entry.bin_path,
                        resolve_install_args(args),
                        target,
                        &mut stage,
                    )
                    .await?
                }
            }
            provider_matrix::ProviderInstall::Archive {
                version,
                args,
                targets,
            } => {
                if provider_matrix::normalize_version(version)
                    != provider_matrix::normalize_version(&release.version)
                {
                    anyhow::bail!(
                        "provider matrix version mismatch for {provider_id}: release={} install={}",
                        release.version,
                        version
                    );
                }
                let target_entry = targets.get(resolved_target_key).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unsupported provider target {provider_id}: {resolved_target_key}"
                    )
                })?;
                error_package = Some(target_entry.url.clone());
                error_version = Some(version.clone());
                error_install_dir_rel = Some(format!(
                    "providers/agent-servers/{provider_id}/{version}",
                ));
                let managed = install_managed_archive_provider(
                    state,
                    install_id,
                    &provider_id,
                    version,
                    &target_entry.url,
                    target_entry.sha256.as_deref(),
                    map_archive_kind(target_entry.archive),
                    &target_entry.bin_path,
                    resolve_install_args(args),
                    target,
                    &mut stage,
                )
                .await?;
                if archive_bin_requires_node_runtime(
                    &target_entry.bin_path,
                    Path::new(&managed.command),
                ) {
                    stage = "node";
                    for dependency_target in
                        node_runtime_dependency_targets_for_install_target(target, std::env::consts::OS)
                    {
                        let node = ensure_node_runtime(
                            state,
                            install_id,
                            &provider_id,
                            state.data_root(),
                            dependency_target,
                        )
                        .await
                        .context("ensuring managed Node runtime for archive provider")?;
                        let dep_id = node_runtime_dependency_id(dependency_target);
                        if !dependency_ids.contains(&dep_id) {
                            dependency_ids.push(dep_id.clone());
                        }
                        implicit_managed_dependencies.push((
                            dep_id,
                            node_runtime_dependency_metadata(
                                state.data_root(),
                                &node,
                                dependency_target,
                            ),
                        ));
                    }
                }
                managed
            }
        };

        stage = "inspect";
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "inspect",
            "Verifying provider install".to_string(),
            None,
            None,
            None,
        )
        .await;

        let adapter_cfg = load_agent_server_config(state.data_root())
            .await
            .context("loading managed install registry for provider install verification")?;
        let bridge_cmd = if state.is_acp_provider_id(&provider_id) {
            resolve_runtime_provider_command_for_target(
                &adapter_cfg,
                "acp-crp-bridge",
                Some(target),
            )?
            .map(|resolved| AgentServerCommand {
                command: resolved.command_abs_path,
                args: resolved.args,
                dependencies: resolved.dependencies,
                managed: None,
            })
        } else {
            None
        };
        let runtime_cmd = managed_provider_runtime_command(
            state.data_root(),
            &provider_id,
            AgentServerCommand {
                command: managed.command.clone(),
                args: managed.args.clone(),
                dependencies: Vec::new(),
                managed: None,
            },
            bridge_cmd.as_ref(),
        )?;
        let adapter: std::sync::Arc<Tier1CrpAdapter> = std::sync::Arc::new(
            Tier1CrpAdapter::from_provider_runtime(&provider_id, runtime_cmd.command, runtime_cmd.args),
        );

        // Refresh the in-memory adapter so new Sessions use the managed install.
        if matches!(target, InstallTarget::Host) {
            let mut map = state.provider_adapters().lock().await;
            map.insert(provider_id.clone(), adapter.clone());
        } else {
            let mut map = state.target_provider_adapters().lock().await;
            map.insert(
                format!("{provider_id}@{}", target.as_str()),
                adapter.clone(),
            );
        }

        stage = "refresh";
        ensure_install_not_cancelled(state, install_id).await?;
        let mut status_cfg = load_agent_server_config(state.data_root())
            .await
            .context("loading managed install registry for provider status refresh")?;
        apply_managed_provider_install_to_cfg(
            &mut status_cfg,
            &provider_id,
            target,
            &managed,
            &dependency_ids,
            &implicit_managed_dependencies,
        );
        let mut verified_status = ctx_providers::adapters::ProviderAdapter::inspect(adapter.as_ref())
            .await
            .context("inspecting provider after managed install")?;
        apply_managed_install_details_for_target(&mut verified_status, &status_cfg, Some(target));
        apply_install_target_status(&mut verified_status, target);
        validate_post_install_status(&verified_status, &provider_id, target)?;
        refresh_provider_statuses_with_cfg(state, status_cfg).await?;

        stage = "registry";
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "registry",
            "Writing managed install registry".to_string(),
            None,
            None,
            None,
        )
        .await;

        mutate_agent_server_config(state.data_root(), |cfg| {
            apply_managed_provider_install_to_cfg(
                cfg,
                &provider_id,
                target,
                &managed,
                &dependency_ids,
                &implicit_managed_dependencies,
            );
        })
        .await
        .context("saving managed install registry")?;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "registry",
            "Wrote managed install registry".to_string(),
            None,
            None,
            None,
        )
        .await;

        wait_for_provider_readiness_dependencies(
            state,
            &provider_id,
            &readiness_dependencies,
            install_id,
        )
        .await?;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "done",
            "Install complete".to_string(),
            None,
            None,
            None,
        )
        .await;
        Ok(())
    }
    .await;

    if let Err(e) = &res {
        let error_code = classify_install_error(stage, e);
        emit_install_with_code(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Error,
            "error",
            truncate_for_storage(&format!("{e:#}"), INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            None,
            Some(error_code),
        )
        .await;
        update_registry_last_error(
            state.data_root(),
            &provider_id,
            stage,
            e,
            error_code,
            error_package.as_deref(),
            error_version.as_deref(),
            error_install_dir_rel.clone(),
            Some(target),
        )
        .await;
    }

    res
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn emit_install(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    level: InstallEventLevel,
    stage: &str,
    message: String,
    bytes: Option<u64>,
    total_bytes: Option<u64>,
    attempt: Option<u32>,
) {
    emit_install_with_code(
        state,
        install_id,
        provider_id,
        level,
        stage,
        message,
        bytes,
        total_bytes,
        attempt,
        None,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn emit_install_with_code(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    level: InstallEventLevel,
    stage: &str,
    message: String,
    bytes: Option<u64>,
    total_bytes: Option<u64>,
    attempt: Option<u32>,
    error_code: Option<InstallErrorCode>,
) {
    let Some(install_id) = install_id else {
        return;
    };
    let target = state
        .get_install_info(install_id)
        .await
        .and_then(|info| info.target);
    state
        .emit_install_event(
            install_id,
            InstallProgressEvent {
                install_id,
                provider_id: provider_id.to_string(),
                target,
                at: Utc::now(),
                stage: stage.to_string(),
                message,
                level,
                bytes,
                total_bytes,
                attempt,
                error_code,
            },
        )
        .await;
}

pub(super) async fn ensure_install_not_cancelled(
    state: &AppState,
    install_id: Option<InstallId>,
) -> Result<()> {
    let Some(install_id) = install_id else {
        return Ok(());
    };
    if state.is_install_cancelled(install_id).await {
        anyhow::bail!("install canceled by user");
    }
    Ok(())
}

pub(super) fn classify_install_error(stage: &str, err: &anyhow::Error) -> InstallErrorCode {
    let text = format!("{err:#}").to_ascii_lowercase();
    if text.contains("install canceled by user") {
        return InstallErrorCode::Cancelled;
    }
    if text.contains("invalid install target") {
        return InstallErrorCode::InvalidTarget;
    }
    if text.contains("unsupported provider target")
        || text.contains("unsupported dependency target")
        || text.contains("is not supported for")
    {
        return InstallErrorCode::UnsupportedTarget;
    }
    if text.contains("checksum mismatch") {
        return InstallErrorCode::ChecksumMismatch;
    }
    if text.contains("timed out") {
        return InstallErrorCode::Timeout;
    }
    if stage == "refresh" || text.contains("not healthy") {
        return InstallErrorCode::HealthCheckFailed;
    }
    if stage == "registry"
        || text.contains("managed install registry")
    {
        return InstallErrorCode::RegistryWriteFailed;
    }
    if text.contains("matrix version mismatch") || text.contains("no compatible release") {
        return InstallErrorCode::MatrixMismatch;
    }
    if text.contains("download")
        || text.contains("http error")
        || text.contains("sending request")
        || text.contains("streaming download")
    {
        return InstallErrorCode::DownloadFailed;
    }
    if text.contains("install failed")
        || text.contains("process")
        || text.contains("command")
        || text.contains("pip")
        || text.contains("npm")
    {
        return InstallErrorCode::CommandFailed;
    }
    InstallErrorCode::Unknown
}

pub async fn refresh_provider_statuses(state: &AppState) -> Result<()> {
    let cfg = load_agent_server_config(state.data_root())
        .await
        .context("loading managed install registry for provider status refresh")?;
    refresh_provider_statuses_with_cfg(state, cfg).await
}

async fn refresh_provider_statuses_with_cfg(
    state: &AppState,
    cfg: AgentServerConfigFile,
) -> Result<()> {
    let matrix =
        provider_matrix::load_matrix_cached(state.data_root(), state.provider_matrix_cache()).await;
    let current_ctx_version = state.current_ctx_version();

    let map = state.provider_adapters().lock().await;
    let mut statuses = HashMap::new();
    for (id, adapter) in map.iter() {
        match adapter.inspect().await {
            Ok(mut status) => {
                apply_managed_install_details(&mut status, &cfg);
                if let Some(entry) = provider_matrix::get_entry(&matrix, id) {
                    crate::provider_status_matrix::apply_matrix_to_status(
                        state.data_root(),
                        &cfg,
                        entry,
                        &mut status,
                        current_ctx_version.as_deref(),
                    )
                    .await;
                }
                statuses.insert(id.clone(), status);
            }
            Err(e) => {
                statuses.insert(
                    id.clone(),
                    ctx_providers::adapters::ProviderStatus {
                        provider_id: id.clone(),
                        installed: false,
                        detected_path: None,
                        version: None,
                        capabilities: None,
                        health: ctx_providers::adapters::ProviderHealth::Error,
                        diagnostics: vec![e.to_string()],
                        details: HashMap::new(),
                        usability: ctx_providers::adapters::ProviderUsability::default(),
                    },
                );
            }
        }
    }
    drop(map);
    *state.provider_statuses().lock().await = statuses;
    Ok(())
}
