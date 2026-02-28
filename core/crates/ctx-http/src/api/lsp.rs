use super::*;

pub(super) async fn lsp_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LspStatusResp>, StatusCode> {
    let cfg = &state.core.lsp_cfg;
    let enabled = cfg.enabled;
    let edit_plans_enabled = state.core.lsp_edit_plans_enabled;

    let servers = vec![
        ("rust", cfg.rust_command.clone(), cfg.rust_args.clone()),
        ("typescript", cfg.ts_command.clone(), cfg.ts_args.clone()),
        ("python", cfg.py_command.clone(), cfg.py_args.clone()),
        ("go", cfg.go_command.clone(), cfg.go_args.clone()),
        ("html", cfg.html_command.clone(), cfg.html_args.clone()),
        ("css", cfg.css_command.clone(), cfg.css_args.clone()),
        ("json", cfg.json_command.clone(), cfg.json_args.clone()),
        ("yaml", cfg.yaml_command.clone(), cfg.yaml_args.clone()),
        ("bash", cfg.bash_command.clone(), cfg.bash_args.clone()),
        (
            "dockerfile",
            cfg.dockerfile_command.clone(),
            cfg.dockerfile_args.clone(),
        ),
        ("cpp", cfg.clangd_command.clone(), cfg.clangd_args.clone()),
        ("lua", cfg.lua_command.clone(), cfg.lua_args.clone()),
        ("toml", cfg.toml_command.clone(), cfg.toml_args.clone()),
        (
            "markdown",
            cfg.markdown_command.clone(),
            cfg.markdown_args.clone(),
        ),
    ];

    let mut out = Vec::new();
    for (language, command, args) in servers {
        let (found, resolved_path) = resolve_command(&command);
        let version = if found {
            get_command_version(&command, &resolved_path, &args).await
        } else {
            None
        };
        out.push(LspServerStatus {
            language: language.to_string(),
            command,
            args,
            found,
            resolved_path: resolved_path.map(|p| p.to_string_lossy().to_string()),
            version,
            install_hints: install_hints_for(language),
        });
    }

    // Add BYO servers not already represented by built-ins.
    for (language, (command, args)) in cfg.custom_servers.iter() {
        if out.iter().any(|s| s.language == *language) {
            continue;
        }
        let (found, resolved_path) = resolve_command(command);
        let version = if found {
            get_command_version(command, &resolved_path, args).await
        } else {
            None
        };
        out.push(LspServerStatus {
            language: language.clone(),
            command: command.clone(),
            args: args.clone(),
            found,
            resolved_path: resolved_path.map(|p| p.to_string_lossy().to_string()),
            version,
            install_hints: vec![
                "Configured via data_root/lsp/user_servers.json (restart daemon after edits)."
                    .to_string(),
            ],
        });
    }

    Ok(Json(LspStatusResp {
        enabled,
        edit_plans_enabled,
        servers: out,
    }))
}

#[derive(Debug, Serialize)]
pub(super) struct LspCatalogEntryStatusResp {
    id: String,
    title: String,
    language_id: String,
    install_kind: String,
    installed: bool,
    installed_version: Option<String>,
    enabled: bool,
    enabled_command: Option<String>,
}

pub(super) async fn lsp_catalog_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<LspCatalogEntryStatusResp>>, StatusCode> {
    let catalog = crate::lsp_catalog::load_catalog(&state.core.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let installed = installer::load_lsp_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();

    let mut out = Vec::new();
    for entry in catalog.servers {
        let (install_kind, installed_key) = match &entry.install {
            crate::lsp_catalog::LspCatalogInstall::ManagedNode { server_id } => {
                ("managed_node".to_string(), server_id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::UrlBinary { .. } => {
                ("url_binary".to_string(), entry.id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::GoInstall { .. } => {
                ("go_install".to_string(), entry.id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::System { .. } => {
                ("system".to_string(), entry.id.clone())
            }
        };

        let meta = installed.managed_installs.get(&installed_key);
        let installed_version = meta.and_then(|m| m.version.clone());
        let enabled_cmd = installed
            .servers
            .get(&entry.language_id)
            .map(|c| c.command.clone());
        out.push(LspCatalogEntryStatusResp {
            id: entry.id,
            title: entry.title,
            language_id: entry.language_id,
            install_kind,
            installed: meta.is_some(),
            installed_version,
            enabled: enabled_cmd.is_some(),
            enabled_command: enabled_cmd,
        });
    }

    Ok(Json(out))
}

#[derive(Debug, Serialize)]
pub(super) struct LspCatalogInstallStartResponse {
    catalog_id: String,
    install_id: InstallId,
}

pub(super) async fn install_lsp_catalog_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LspCatalogInstallStartResponse>, StatusCode> {
    // Validate id exists.
    if crate::lsp_catalog::get_entry(&state.core.data_root, &id)
        .await
        .is_err()
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let install_key = format!("lsp:{id}");
    let (install_id, started_new) = state.start_install(install_key, None).await;
    if started_new {
        let state2 = state.clone();
        let catalog_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_lsp_catalog_server_with_progress(
                state2.clone(),
                install_id,
                catalog_id.clone(),
            )
            .await
            {
                tracing::error!("lsp catalog install failed ({catalog_id}): {e:#}");
            }
        });
    }

    Ok(Json(LspCatalogInstallStartResponse {
        catalog_id: id,
        install_id,
    }))
}

fn resolve_command(command: &str) -> (bool, Option<PathBuf>) {
    if command.trim().is_empty() {
        return (false, None);
    }
    let path = PathBuf::from(command);
    if path.components().count() > 1 {
        return (path.exists(), Some(path));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return (false, None);
    };
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(command);
        if candidate.exists() {
            return (true, Some(candidate));
        }
    }
    (false, None)
}

pub(super) async fn get_command_version(
    command: &str,
    resolved: &Option<PathBuf>,
    _args: &[String],
) -> Option<String> {
    let exe = resolved
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| command.to_string());
    let base = StdPath::new(&exe)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let candidates: Vec<Vec<&'static str>> = if base.contains("gopls") {
        vec![vec!["version"], vec!["--version"]]
    } else {
        vec![vec!["--version"], vec!["version"]]
    };

    for args in candidates {
        let fut = Command::new(&exe).args(args.iter()).output();
        if let Ok(Ok(output)) = tokio::time::timeout(std::time::Duration::from_secs(2), fut).await {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let s = if s.is_empty() {
                    String::from_utf8_lossy(&output.stderr).trim().to_string()
                } else {
                    s
                };
                if !s.is_empty() {
                    return Some(s.lines().next().unwrap_or("").trim().to_string());
                }
            }
        }
    }
    None
}

fn install_hints_for(language: &str) -> Vec<String> {
    let os = std::env::consts::OS;
    match (language, os) {
        ("rust", "darwin") => vec![
            "brew install rust-analyzer".to_string(),
            "or: rustup component add rust-analyzer (if available)".to_string(),
        ],
        ("rust", "linux") => vec![
            "rustup component add rust-analyzer (if available)".to_string(),
            "or: install rust-analyzer from your distro/package manager".to_string(),
        ],
        ("typescript", _) => vec![
            "managed: POST /api/lsp/servers/typescript/install (restart daemon after install)".to_string(),
            "or: npm i -g typescript typescript-language-server".to_string(),
        ],
        ("python", _) => vec![
            "managed: POST /api/lsp/servers/python/install (restart daemon after install)".to_string(),
            "or: npm i -g pyright".to_string(),
        ],
        ("go", _) => vec!["go install golang.org/x/tools/gopls@latest".to_string()],
        ("html" | "css" | "json", _) => vec![
            "managed: POST /api/lsp/servers/html/install (installs html+css+json; restart daemon after install)".to_string(),
            "or: npm i -g vscode-langservers-extracted".to_string(),
        ],
        ("yaml", _) => vec![
            "managed: POST /api/lsp/servers/yaml/install (restart daemon after install)".to_string(),
            "or: npm i -g yaml-language-server".to_string(),
        ],
        ("bash", _) => vec![
            "managed: POST /api/lsp/servers/bash/install (restart daemon after install)".to_string(),
            "or: npm i -g bash-language-server".to_string(),
        ],
        ("dockerfile", _) => vec![
            "managed: POST /api/lsp/servers/dockerfile/install (restart daemon after install)".to_string(),
            "or: npm i -g dockerfile-language-server-nodejs".to_string(),
        ],
        ("cpp", "darwin") => vec!["brew install llvm (clangd)".to_string()],
        ("cpp", "linux") => vec!["sudo apt-get install clangd (or distro equivalent)".to_string()],
        ("lua", "darwin") => vec!["brew install lua-language-server".to_string()],
        ("lua", "linux") => vec!["install lua-language-server via your distro/package manager".to_string()],
        ("toml", "darwin") => vec!["brew install taplo".to_string()],
        ("toml", "linux") => vec!["cargo install taplo-cli --locked".to_string()],
        ("markdown", "darwin") => vec!["brew install marksman".to_string()],
        ("markdown", "linux") => vec!["install marksman via your distro/package manager".to_string()],
        _ => vec![],
    }
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn plan_paths_match(a_old: &str, a_new: &str, b_old: &str, b_new: &str) -> bool {
    (!a_new.is_empty() || a_old == b_old) && a_new == b_new
}

pub(super) async fn lsp_diagnostics(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let (root, file) = resolve_lsp_target(&state, req).await?;
    let diags = state
        .core
        .lsp
        .diagnostics_for_file(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = diags
        .into_iter()
        .filter_map(|d| serde_json::to_value(d).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

pub(super) async fn resolve_lsp_target(
    state: &Arc<AppState>,
    req: LspFileReq,
) -> Result<(PathBuf, PathBuf), StatusCode> {
    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;
    let candidate = if PathBuf::from(&req.path).is_absolute() {
        PathBuf::from(&req.path)
    } else {
        root.join(&req.path)
    };
    let file = candidate
        .canonicalize()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if !file.starts_with(&root) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((root, file))
}

pub(super) async fn resolve_session_root_and_file(
    state: &Arc<AppState>,
    session_id: &str,
    path: &str,
) -> Result<(SessionId, WorkspaceId, WorktreeId, PathBuf, PathBuf), StatusCode> {
    let sid = SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(sid)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let root = PathBuf::from(&wt.root_path);
    if crate::container_fs::is_container_path(&root) {
        let file = crate::buffers::BufferStore::resolve_path_lexical(&root, path)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, session.workspace_id, session.worktree_id, root, file))
    } else {
        let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;
        let file = crate::buffers::BufferStore::resolve_path(&root, path)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, session.workspace_id, session.worktree_id, root, file))
    }
}

pub(super) async fn ensure_harness_container_for_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), StatusCode> {
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let settings = crate::execution_effective::effective_execution_settings(state, workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .execution
        .harness
        .ensure_workspace_container(&workspace, &settings, &state.core.daemon_url)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

pub(super) async fn open_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferOpenReq>,
) -> Result<Json<BufferOpenResp>, StatusCode> {
    let (sid, workspace_id, worktree_id, root, file) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path).await?;
    let text = if crate::container_fs::is_container_path(&file) {
        ensure_harness_container_for_workspace(&state, workspace_id).await?;
        let container_id = format!("ctx-harness-{}", workspace_id.0);
        let fs = crate::container_fs::ContainerFs::new(state.core.data_root.clone(), container_id);
        fs.read_to_string(&file)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
    } else {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let disk_sha = sha256_hex(&text);
    let st = state
        .core
        .buffers
        .open_or_reuse(
            sid,
            worktree_id,
            root.clone(),
            file.clone(),
            text.clone(),
            disk_sha.clone(),
        )
        .await;
    if state.core.lsp.enabled() {
        // Container-only paths are not available on the host filesystem; skip host LSP sync.
        if !crate::container_fs::is_container_path(&file) {
            if let Some(lang) = ctx_lsp::Language::detect(&file, &state.core.lsp_cfg) {
                state
                    .ensure_lsp_diagnostics_forwarder(root.clone(), lang)
                    .await;
            }
            let _ = state
                .core
                .lsp
                .sync_document_text(&root, &file, st.text.clone())
                .await;
        }
    }
    Ok(Json(BufferOpenResp {
        buffer_id: st.id.0.to_string(),
        path: req.path,
        version: st.version,
        text: st.text,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

pub(super) async fn update_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferUpdateReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<BufferConflictResp>)> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(BufferConflictResp {
                error: "invalid buffer_id".to_string(),
                disk_sha256: "".to_string(),
                disk_text: "".to_string(),
            }),
        )
    })?);
    let current = state.core.buffers.get(bid).await.ok_or((
        StatusCode::NOT_FOUND,
        Json(BufferConflictResp {
            error: "buffer not found".to_string(),
            disk_sha256: "".to_string(),
            disk_text: "".to_string(),
        }),
    ))?;

    let new_sha = if req.persist {
        let is_container_file = crate::container_fs::is_container_path(&current.path);

        // Detect external changes on disk.
        let disk_text = if is_container_file {
            let store = state
                .store_for_worktree(current.worktree_id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::NOT_FOUND,
                        Json(BufferConflictResp {
                            error: "worktree not found".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            let wt = store
                .get_worktree(current.worktree_id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to load worktree".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?
                .ok_or((
                    StatusCode::NOT_FOUND,
                    Json(BufferConflictResp {
                        error: "worktree not found".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                ))?;
            ensure_harness_container_for_workspace(&state, wt.workspace_id)
                .await
                .map_err(|code| {
                    (
                        code,
                        Json(BufferConflictResp {
                            error: "failed to ensure harness container".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            let container_id = format!("ctx-harness-{}", wt.workspace_id.0);
            let fs =
                crate::container_fs::ContainerFs::new(state.core.data_root.clone(), container_id);
            fs.read_to_string(&current.path).await.map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(BufferConflictResp {
                        error: "failed to read file".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                )
            })?
        } else {
            tokio::fs::read_to_string(&current.path)
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: "failed to read file".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?
        };
        let disk_sha = sha256_hex(&disk_text);
        if !req.force && disk_sha != current.last_disk_sha256 {
            return Err((
                StatusCode::CONFLICT,
                Json(BufferConflictResp {
                    error: "file changed on disk while buffer was open".to_string(),
                    disk_sha256: disk_sha,
                    disk_text,
                }),
            ));
        }

        // Write to disk (autosave).
        if is_container_file {
            let store = state
                .store_for_worktree(current.worktree_id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::NOT_FOUND,
                        Json(BufferConflictResp {
                            error: "worktree not found".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            let wt = store
                .get_worktree(current.worktree_id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to load worktree".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?
                .ok_or((
                    StatusCode::NOT_FOUND,
                    Json(BufferConflictResp {
                        error: "worktree not found".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                ))?;
            ensure_harness_container_for_workspace(&state, wt.workspace_id)
                .await
                .map_err(|code| {
                    (
                        code,
                        Json(BufferConflictResp {
                            error: "failed to ensure harness container".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            let container_id = format!("ctx-harness-{}", wt.workspace_id.0);
            let fs =
                crate::container_fs::ContainerFs::new(state.core.data_root.clone(), container_id);
            fs.write_string(&current.path, &req.text)
                .await
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: logs::redact_sensitive(&e.to_string()),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
        } else {
            tokio::fs::write(&current.path, req.text.as_bytes())
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: "failed to write file".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
        }
        Some(sha256_hex(&req.text))
    } else {
        None
    };
    let st = state
        .core
        .buffers
        .update(bid, req.version, req.text, new_sha.clone())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(BufferConflictResp {
                    error: logs::redact_sensitive(&e.to_string()),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;

    if state.core.lsp.enabled() && !crate::container_fs::is_container_path(&st.path) {
        if let Some(lang) = ctx_lsp::Language::detect(&st.path, &state.core.lsp_cfg) {
            state
                .ensure_lsp_diagnostics_forwarder(st.root.clone(), lang)
                .await;
        }
        let _ = state
            .core
            .lsp
            .sync_document_text(&st.root, &st.path, st.text.clone())
            .await;
    }

    Ok(Json(BufferUpdateResp {
        buffer_id: st.id.0.to_string(),
        version: st.version,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

pub(super) async fn close_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferCloseReq>,
) -> Result<StatusCode, StatusCode> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sid =
        SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state.core.buffers.close(bid, sid).await;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub(super) struct LspPosReq {
    #[serde(flatten)]
    file: LspFileReq,
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspRefsReq {
    #[serde(flatten)]
    pos: LspPosReq,
    include_declaration: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspWorkspaceSymbolsReq {
    session_id: Option<String>,
    root_path: Option<String>,
    query: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspCodeActionsReq {
    #[serde(flatten)]
    file: LspFileReq,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspRangeReq {
    #[serde(flatten)]
    file: LspFileReq,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspResolveReq {
    #[serde(flatten)]
    file: LspFileReq,
    item: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspLineChar {
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspSelectionRangesReq {
    #[serde(flatten)]
    file: LspFileReq,
    positions: Vec<LspLineChar>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspSemanticTokensDeltaReq {
    #[serde(flatten)]
    file: LspFileReq,
    previous_result_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspWorkspaceSymbolResolveReq {
    session_id: Option<String>,
    root_path: Option<String>,
    item: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspExecuteCommandReq {
    #[serde(flatten)]
    file: LspFileReq,
    command: String,
    arguments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspCodeActionsByDiagnosticPlanReq {
    session_id: String,
    path: String,
    diagnostic: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspRenamePlanReq {
    session_id: String,
    path: String,
    line: u32,
    character: u32,
    new_name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspFormatPlanReq {
    session_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspOrganizeImportsPlanReq {
    session_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LspCodeActionPlanReq {
    session_id: String,
    action: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct EditPlanApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}

pub(super) async fn lsp_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_type_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_implementation(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .implementation(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_references(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRefsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.pos.file).await?;
    let out = state
        .core
        .lsp
        .references(
            &root,
            &file,
            lsp_types::Position {
                line: req.pos.line,
                character: req.pos.character,
            },
            req.include_declaration.unwrap_or(true),
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|l| serde_json::to_value(l).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

pub(super) async fn lsp_hover(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .hover(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_signature_help(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .signature_help(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_completion(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_completion_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_code_action_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_action_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_inlay_hints(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRangeReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .inlay_hints(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_document_highlight(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_highlight(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_selection_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSelectionRangesReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let positions = req
        .positions
        .into_iter()
        .map(|p| lsp_types::Position {
            line: p.line,
            character: p.character,
        })
        .collect::<Vec<_>>();
    let v = state
        .core
        .lsp
        .selection_ranges(&root, &file, positions)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_call_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_call_hierarchy_incoming(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_incoming(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_call_hierarchy_outgoing(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_outgoing(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_code_lens(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .code_lens(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_code_lens_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_lens_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_prepare_rename(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .prepare_rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_document_links(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_links(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_document_link_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_link_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_semantic_tokens_full(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_full(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_semantic_tokens_delta(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSemanticTokensDeltaReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_delta(&root, &file, req.previous_result_id)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_folding_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .folding_ranges(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_linked_editing_range(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .linked_editing_range(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_type_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_type_hierarchy_supertypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_supertypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_type_hierarchy_subtypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_subtypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_code_actions_by_diagnostic_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsByDiagnosticPlanReq>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = crate::buffers::BufferStore::resolve_path(&root, &req.path)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid path".to_string(),
                }),
            )
        })?;

    let diag: lsp_types::Diagnostic =
        serde_json::from_value(req.diagnostic.clone()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            diag.range,
            vec![diag.clone()],
            Some(vec![lsp_types::CodeActionKind::QUICKFIX]),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let mut plans: Vec<crate::edit_plans::EditPlanSummary> = Vec::new();
    for action in actions {
        let (title, edit, preferred) = match action {
            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                let edit = ca.edit.or_else(|| {
                    ca.command
                        .as_ref()
                        .and_then(extract_workspace_edit_from_command)
                });
                (ca.title, edit, ca.is_preferred.unwrap_or(false))
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                let edit = extract_workspace_edit_from_command(&cmd);
                (cmd.title, edit, false)
            }
        };
        let Some(edit) = edit else { continue };
        let plan =
            crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        let summary = plan.to_summary();
        state.persist_edit_plan(&plan);
        state
            .workspaces
            .edit_plans
            .lock()
            .await
            .insert(plan.id, plan);
        if preferred {
            plans.insert(0, summary);
        } else {
            plans.push(summary);
        }
    }

    Ok(Json(plans))
}
pub(super) async fn lsp_execute_command(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(&root, &file, req.command, req.arguments.unwrap_or_default())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let edit = edit
        .and_then(|e| serde_json::to_value(e).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(Json(serde_json::json!({
        "result": result,
        "workspace_edit": edit
    })))
}

pub(super) async fn lsp_execute_command_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let session_id = req.file.session_id.clone().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "session_id required".to_string(),
        }),
    ))?;
    let sid = SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;

    let (root, file) = resolve_lsp_target(&state, req.file).await.map_err(|sc| {
        (
            sc,
            Json(ApiErrorResp {
                error: "invalid LSP target".to_string(),
            }),
        )
    })?;

    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(
            &root,
            &file,
            req.command.clone(),
            req.arguments.unwrap_or_default(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "execute_command returned no WorkspaceEdit (result={})",
                    result
                ),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        format!("Execute command: {}", req.command),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(super) async fn lsp_document_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_symbols(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_workspace_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = state
        .core
        .lsp
        .workspace_symbols(&root, req.query)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

pub(super) async fn lsp_workspace_symbol_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = state
        .core
        .lsp
        .workspace_symbol_resolve(&root, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(out))
}

pub(super) async fn lsp_code_actions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_actions(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
            vec![],
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

pub(super) async fn lsp_rename_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRenamePlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);
    let edit = state
        .core
        .lsp
        .rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
            req.new_name,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Rename".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(super) async fn lsp_format_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFormatPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);
    let edits = state
        .core
        .lsp
        .format_document(&root, &file)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let rel = file
        .strip_prefix(&root)
        .unwrap_or(&file)
        .to_string_lossy()
        .to_string();
    let plan = crate::edit_plans::text_edits_to_plan(
        &root,
        sid,
        worktree_id,
        "Format document".to_string(),
        rel,
        edits,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(super) async fn lsp_code_actions_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;

    let action: lsp_types::CodeActionOrCommand = serde_json::from_value(req.action.clone())
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let (title, edit) = match action {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => {
            let edit = ca.edit.or_else(|| {
                ca.command
                    .as_ref()
                    .and_then(extract_workspace_edit_from_command)
            });
            (ca.title, edit)
        }
        lsp_types::CodeActionOrCommand::Command(cmd) => {
            let edit = extract_workspace_edit_from_command(&cmd);
            (cmd.title, edit)
        }
    };
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "code action has no edit".to_string(),
            }),
        ));
    };

    let plan =
        crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(super) async fn lsp_organize_imports_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspOrganizeImportsPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);

    let text = tokio::fs::read_to_string(&file).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let (end_line, end_character) = text
        .split('\n')
        .enumerate()
        .fold((0u32, 0u32), |(_l, _c), (i, line)| {
            (i as u32, line.chars().count() as u32)
        });

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: 0,
                    character: 0,
                },
                end: lsp_types::Position {
                    line: end_line,
                    character: end_character,
                },
            },
            vec![],
            Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let edit = actions.into_iter().find_map(|a| match a {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => ca.edit.or_else(|| {
            ca.command
                .as_ref()
                .and_then(extract_workspace_edit_from_command)
        }),
        lsp_types::CodeActionOrCommand::Command(cmd) => extract_workspace_edit_from_command(&cmd),
    });
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "no organize-imports edit returned".to_string(),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Organize imports".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

pub(super) async fn list_edit_plans_for_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let map = state.workspaces.edit_plans.lock().await;
    let mut out = map
        .values()
        .filter(|p| p.worktree_id == worktree_id)
        .map(|p| p.to_summary())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(out))
}

pub(super) async fn get_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let map = state.workspaces.edit_plans.lock().await;
    let Some(plan) = map.get(&pid) else {
        return Err(StatusCode::NOT_FOUND);
    };
    Ok(Json(plan.to_summary()))
}

pub(super) async fn apply_edit_plan_patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<EditPlanApplyReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }
    let pid = crate::edit_plans::EditPlanId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid edit plan id".to_string(),
            }),
        )
    })?);
    if req.patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    let action = req.action.trim().to_lowercase();
    let patch = req.patch;

    match action.as_str() {
        "accept" => {
            let parsed = crate::edit_plans::parse_unified_diff(&patch);

            let (worktree_root, plan_files) = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                (plan.worktree_root.clone(), plan.files.clone())
            };

            // Stale-plan check: ensure files match the base used to create the plan.
            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    &pf.new_path
                } else {
                    &pf.old_path
                };
                let Some(base) = plan_files
                    .iter()
                    .find(|f| {
                        plan_paths_match(&pf.old_path, &pf.new_path, &f.old_path, &f.new_path)
                    })
                    .map(|f| f.base_sha256.clone())
                else {
                    continue;
                };
                if base.trim().is_empty() {
                    continue;
                }
                let abs = worktree_root.join(rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                let current_sha = sha256_hex(&current);
                if current_sha != base {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: format!("edit plan is stale for {}; regenerate the plan", rel),
                        }),
                    ));
                }
            }

            let worktree_root = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.worktree_root.clone()
            };

            ctx_fs::git::git_apply_patch(
                worktree_root.to_string_lossy().as_ref(),
                &patch,
                ctx_fs::git::ApplyPatchTarget::Worktree,
                false,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

            // After applying, update base hashes for affected files so subsequent partial applies don't always look stale.
            let mut updated_bases: Vec<(String, String)> = Vec::new();
            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    pf.new_path.clone()
                } else {
                    pf.old_path.clone()
                };
                let abs = worktree_root.join(&rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                updated_bases.push((rel, sha256_hex(&current)));
            }

            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                for (rel, sha) in &updated_bases {
                    for f in &mut plan.files {
                        let f_rel = if !f.new_path.is_empty() {
                            &f.new_path
                        } else {
                            &f.old_path
                        };
                        if f_rel == rel {
                            f.base_sha256 = sha.clone();
                        }
                    }
                }
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        "reject" => {
            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "action must be accept or reject".to_string(),
            }),
        )),
    }
}

pub(super) async fn discard_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    state.workspaces.edit_plans.lock().await.remove(&pid);
    state.delete_edit_plan_file(pid);
    Ok(StatusCode::NO_CONTENT)
}
