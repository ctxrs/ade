use super::*;

mod handlers;

pub(super) use handlers::*;

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
) -> Result<(SessionId, WorkspaceId, WorktreeId, PathBuf, PathBuf, bool), StatusCode> {
    let (sid, workspace_id, worktree_id, root, is_container_file) =
        resolve_session_root(state, session_id).await?;
    if is_container_file {
        let file = crate::buffers::BufferStore::resolve_path_lexical(&root, path)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, workspace_id, worktree_id, root, file, true))
    } else {
        let file = crate::buffers::BufferStore::resolve_path(&root, path)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, workspace_id, worktree_id, root, file, false))
    }
}

pub(super) async fn resolve_session_root(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<(SessionId, WorkspaceId, WorktreeId, PathBuf, bool), StatusCode> {
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

    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(state, &wt)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let root = data_plane.live_worktree_root;
    let is_container_file = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );
    let root = if is_container_file {
        root
    } else {
        root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?
    };

    Ok((
        sid,
        session.workspace_id,
        session.worktree_id,
        root,
        is_container_file,
    ))
}

pub(super) async fn open_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferOpenReq>,
) -> Result<Json<BufferOpenResp>, StatusCode> {
    let (sid, workspace_id, worktree_id, root, file, is_container_file) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path).await?;
    let text = if is_container_file {
        let fs = crate::container_fs::ContainerFs::for_worktree(&state, workspace_id, worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        if !is_container_file {
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
    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(&state, &wt)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BufferConflictResp {
                    error: "failed to resolve worktree data plane".to_string(),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;
    let is_container_file = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );

    let new_sha = if req.persist {
        // Detect external changes on disk.
        let disk_text = if is_container_file {
            let fs = crate::container_fs::ContainerFs::for_worktree(&state, wt.workspace_id, wt.id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to ensure sandbox filesystem".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
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
            let fs = crate::container_fs::ContainerFs::for_worktree(&state, wt.workspace_id, wt.id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to ensure sandbox filesystem".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
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

    if state.core.lsp.enabled() && !is_container_file {
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
