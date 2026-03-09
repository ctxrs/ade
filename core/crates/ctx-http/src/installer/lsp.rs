use super::*;

async fn enable_managed_lsp_server(
    data_root: &Path,
    language_id: &str,
    command: &str,
    args: Vec<String>,
    managed_meta_key: &str,
    meta: ManagedInstallMetadata,
) -> Result<()> {
    let mut cfg = load_lsp_server_config(data_root).await.unwrap_or_default();
    cfg.managed_installs
        .insert(managed_meta_key.to_string(), meta.clone());
    cfg.servers.insert(
        language_id.to_string(),
        AgentServerCommand {
            command: command.to_string(),
            args,
            dependencies: Vec::new(),
            managed: Some(meta),
        },
    );
    save_lsp_server_config(data_root, &cfg).await?;
    Ok(())
}

async fn upsert_managed_extra_server(data_root: &Path, spec: UserLspServerSpec) -> Result<()> {
    let mut cfg = load_lsp_server_config(data_root).await.unwrap_or_default();
    if let Some(id) = spec.id.as_deref() {
        cfg.extra_servers.retain(|s| s.id.as_deref() != Some(id));
    }
    cfg.extra_servers.push(spec);
    save_lsp_server_config(data_root, &cfg).await?;
    Ok(())
}

pub async fn install_lsp_catalog_server_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    catalog_id: String,
) -> Result<()> {
    let res = install_lsp_catalog_server_impl(state.as_ref(), &catalog_id, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("lsp_catalog_install", e);
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
    res
}

async fn install_lsp_catalog_server_impl(
    state: &AppState,
    catalog_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let data_root = state.core.data_root.clone();
    let provider_id = format!("lsp:{catalog_id}");
    let entry = crate::lsp_catalog::get_entry(&data_root, catalog_id).await?;
    let extra_id = entry.id.clone();
    let extra_language_id = entry.language_id.clone();
    let extra_args = entry.args.clone();
    let extra_extensions = entry.extensions.clone();
    let extra_filenames = entry.filenames.clone();

    emit_install(
        state,
        install_id,
        &provider_id,
        InstallEventLevel::Info,
        "start",
        format!("Installing LSP server: {}", entry.title),
        None,
        None,
        None,
    )
    .await;

    match entry.install.clone() {
        LspCatalogInstall::ManagedNode { server_id } => {
            install_lsp_server_impl(state, &server_id, install_id).await?;
            return Ok(());
        }
        LspCatalogInstall::System { command } => {
            let (found, resolved) = resolve_command_path(&command);
            if !found {
                anyhow::bail!("system command not found on PATH: {command}");
            }
            let cmd = resolved
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(command);
            let meta = ManagedInstallMetadata {
                package: Some("system".to_string()),
                version: None,
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
        LspCatalogInstall::GoInstall {
            module,
            version,
            binary,
        } => {
            let install_dir = data_root
                .join("lsp")
                .join("go")
                .join(&entry.id)
                .join(&version)
                .join("bin");
            tokio::fs::create_dir_all(&install_dir).await.ok();
            emit_install(
                state,
                install_id,
                &provider_id,
                InstallEventLevel::Info,
                "go_install",
                format!("go install {module}@{version}"),
                None,
                None,
                None,
            )
            .await;
            let status = Command::new("go")
                .arg("install")
                .arg(format!("{module}@{version}"))
                .env("GOBIN", &install_dir)
                .status()
                .await
                .context("running go install")?;
            if !status.success() {
                anyhow::bail!("go install failed");
            }
            let bin = install_dir.join(&binary);
            if !bin.exists() {
                anyhow::bail!("go install completed but binary missing: {}", bin.display());
            }
            ensure_executable(&bin)?;
            let meta = ManagedInstallMetadata {
                package: Some(module),
                version: Some(version.clone()),
                target: None,
                install_dir_rel: Some(install_dir_rel(&data_root, &install_dir)),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            let cmd = bin.to_string_lossy().to_string();
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
        LspCatalogInstall::UrlBinary { version, targets } => {
            let target_key = catalog_target_key();
            let t = targets
                .get(target_key)
                .ok_or_else(|| anyhow::anyhow!("no catalog target for {target_key}"))?;
            let bin = install_url_binary(
                state,
                install_id,
                &provider_id,
                &entry.id,
                &version,
                &t.url,
                t.archive,
                &t.bin_path,
            )
            .await?;
            let meta = ManagedInstallMetadata {
                package: Some(t.url.clone()),
                version: Some(version.clone()),
                target: None,
                install_dir_rel: Some(install_dir_rel(&data_root, bin.parent().unwrap_or(&bin))),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            let cmd = bin.to_string_lossy().to_string();
            enable_managed_lsp_server(
                &data_root,
                &entry.language_id,
                &cmd,
                entry.args.clone(),
                &entry.id,
                meta,
            )
            .await?;
            upsert_managed_extra_server(
                &data_root,
                UserLspServerSpec {
                    id: Some(extra_id.clone()),
                    language_id: extra_language_id.clone(),
                    command: cmd,
                    args: extra_args.clone(),
                    extensions: extra_extensions.clone(),
                    filenames: extra_filenames.clone(),
                },
            )
            .await?;
        }
    }

    emit_install(
        state,
        install_id,
        &provider_id,
        InstallEventLevel::Success,
        "done",
        "Install complete (restart daemon to apply)".to_string(),
        None,
        None,
        None,
    )
    .await;
    Ok(())
}

pub async fn install_lsp_server_with_progress(
    state: std::sync::Arc<AppState>,
    install_id: InstallId,
    server_id: String,
) -> Result<()> {
    let res = install_lsp_server_impl(state.as_ref(), &server_id, Some(install_id)).await;
    match &res {
        Ok(()) => state.finish_install(install_id, true, None, None).await,
        Err(e) => {
            let code = classify_install_error("lsp_install", e);
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
    res
}

async fn install_lsp_server_impl(
    state: &AppState,
    server_id: &str,
    install_id: Option<InstallId>,
) -> Result<()> {
    let data_root = state.core.data_root.clone();
    let provider_id = format!("lsp:{server_id}");

    if !is_supported_managed_lsp_server(server_id) {
        anyhow::bail!("unsupported managed lsp server: {server_id}");
    }

    let mut stage: &'static str = "start";

    let res: Result<()> = async {
        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Info,
            "start",
            format!("Installing managed LSP server: {server_id}"),
            None,
            None,
            None,
        )
        .await;

        stage = "node";
        let node = ensure_node_runtime(
            state,
            install_id,
            &provider_id,
            &data_root,
            InstallTarget::Host,
        )
        .await
        .context("ensuring managed Node runtime")?;

        stage = "registry_load";
        let mut cfg = load_lsp_server_config(&data_root).await.unwrap_or_default();

        let mut register = |key: &str,
                            package: &str,
                            version: &str,
                            install_dir: &Path,
                            entry: PathBuf,
                            args: Vec<String>| {
            let install_dir_rel = install_dir_rel(&data_root, install_dir);
            let meta = ManagedInstallMetadata {
                package: Some(package.to_string()),
                version: Some(version.to_string()),
                target: None,
                install_dir_rel: Some(install_dir_rel),
                bin_dir_rel: None,
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_error: None,
            };
            cfg.managed_installs.insert(key.to_string(), meta.clone());
            cfg.servers.insert(
                key.to_string(),
                AgentServerCommand {
                    command: node.node_bin.to_string_lossy().to_string(),
                    args: std::iter::once(entry.to_string_lossy().to_string())
                        .chain(args.into_iter())
                        .collect(),
                    dependencies: Vec::new(),
                    managed: Some(meta),
                },
            );
        };

        stage = "install";
        match server_id {
            "typescript" => {
                let package = "typescript-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(TYPESCRIPT_LS_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{TYPESCRIPT_LS_VERSION} (+ typescript@{TYPESCRIPT_VERSION})"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    "typescript",
                    TYPESCRIPT_VERSION,
                )
                .await
                .context("installing typescript")?;
                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    TYPESCRIPT_LS_VERSION,
                )
                .await
                .context("installing typescript-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("typescript-language-server"))
                        .await
                        .context("resolving typescript-language-server bin")?;
                register(
                    "typescript",
                    package,
                    TYPESCRIPT_LS_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "python" => {
                let package = "pyright";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(PYRIGHT_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{PYRIGHT_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    PYRIGHT_VERSION,
                )
                .await
                .context("installing pyright")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("pyright-langserver"))
                        .await
                        .context("resolving pyright-langserver bin")?;
                register(
                    "python",
                    package,
                    PYRIGHT_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "html" | "css" | "json" => {
                let package = "vscode-langservers-extracted";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(VSCODE_LANGSERVERS_EXTRACTED_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{VSCODE_LANGSERVERS_EXTRACTED_VERSION} (html/css/json)"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                )
                .await
                .context("installing vscode-langservers-extracted")?;

                let html_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-html-language-server"))
                        .await
                        .context("resolving vscode-html-language-server")?;
                register(
                    "html",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    html_entry,
                    vec!["--stdio".to_string()],
                );

                let css_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-css-language-server"))
                        .await
                        .context("resolving vscode-css-language-server")?;
                register(
                    "css",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    css_entry,
                    vec!["--stdio".to_string()],
                );

                let json_entry =
                    resolve_node_package_bin(&package_dir, package, Some("vscode-json-language-server"))
                        .await
                        .context("resolving vscode-json-language-server")?;
                register(
                    "json",
                    package,
                    VSCODE_LANGSERVERS_EXTRACTED_VERSION,
                    &package_dir,
                    json_entry,
                    vec!["--stdio".to_string()],
                );
            }
            "yaml" => {
                let package = "yaml-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(YAML_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{YAML_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    YAML_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing yaml-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("yaml-language-server"))
                        .await
                        .context("resolving yaml-language-server")?;
                register(
                    "yaml",
                    package,
                    YAML_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            "bash" => {
                let package = "bash-language-server";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(BASH_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{BASH_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    BASH_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing bash-language-server")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("bash-language-server"))
                        .await
                        .context("resolving bash-language-server")?;
                register(
                    "bash",
                    package,
                    BASH_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["start".to_string(), "--stdio".to_string()],
                );
            }
            "dockerfile" => {
                let package = "dockerfile-language-server-nodejs";
                let package_dir = data_root
                    .join("lsp")
                    .join("node")
                    .join(sanitize_npm_package_for_path(package))
                    .join(DOCKERFILE_LANGUAGE_SERVER_VERSION);
                tokio::fs::create_dir_all(&package_dir).await.ok();

                emit_install(
                    state,
                    install_id,
                    &provider_id,
                    InstallEventLevel::Info,
                    "npm_install",
                    format!("Installing {package}@{DOCKERFILE_LANGUAGE_SERVER_VERSION}"),
                    None,
                    None,
                    None,
                )
                .await;

                npm_install_one(
                    state,
                    install_id,
                    &provider_id,
                    &node,
                    &package_dir,
                    package,
                    DOCKERFILE_LANGUAGE_SERVER_VERSION,
                )
                .await
                .context("installing dockerfile-language-server-nodejs")?;

                let entry =
                    resolve_node_package_bin(&package_dir, package, Some("docker-langserver"))
                        .await
                        .context("resolving dockerfile language server bin")?;
                register(
                    "dockerfile",
                    package,
                    DOCKERFILE_LANGUAGE_SERVER_VERSION,
                    &package_dir,
                    entry,
                    vec!["--stdio".to_string()],
                );
            }
            _ => unreachable!(),
        }

        stage = "registry_save";
        save_lsp_server_config(&data_root, &cfg)
            .await
            .context("saving lsp server config")?;

        emit_install(
            state,
            install_id,
            &provider_id,
            InstallEventLevel::Success,
            "done",
            "Install complete (restart daemon to apply)".to_string(),
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

        let mut cfg = load_lsp_server_config(&data_root).await.unwrap_or_default();
        cfg.managed_installs.insert(
            server_id.to_string(),
            ManagedInstallMetadata {
                package: None,
                version: None,
                target: None,
                install_dir_rel: None,
                bin_dir_rel: None,
                last_success_at: None,
                last_error: Some(ManagedInstallError {
                    at: Utc::now().to_rfc3339(),
                    stage: stage.to_string(),
                    message: truncate_for_storage(&format!("{e:#}"), LAST_ERROR_MAX_LEN),
                    code: Some(error_code),
                }),
            },
        );
        let _ = save_lsp_server_config(&data_root, &cfg).await;
    }

    res
}
