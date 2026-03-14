use super::*;

pub(crate) fn target_uses_windows_layout(target: InstallTarget) -> bool {
    match target {
        InstallTarget::Host => cfg!(windows),
        InstallTarget::Container | InstallTarget::LinuxAarch64 | InstallTarget::LinuxX8664 => false,
    }
}

pub(crate) fn venv_bin_dir(venv_dir: &Path, target: InstallTarget) -> PathBuf {
    if target_uses_windows_layout(target) {
        venv_dir.join("Scripts")
    } else {
        venv_dir.join("bin")
    }
}

pub(crate) fn venv_exe(venv_dir: &Path, name: &str, target: InstallTarget) -> PathBuf {
    let bin = venv_bin_dir(venv_dir, target);
    if target_uses_windows_layout(target) {
        bin.join(format!("{name}.exe"))
    } else {
        bin.join(name)
    }
}

pub struct NodeRuntime {
    pub node_root: PathBuf,
    pub node_bin: PathBuf,
    pub npm_cli_js: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeRuntimeTarget {
    pub(crate) dist_target: &'static str,
    pub(crate) is_windows: bool,
}

#[derive(Debug, Clone)]
pub struct PythonRuntime {
    #[allow(dead_code)]
    pub python_root: PathBuf,
    pub python_bin: PathBuf,
}

pub(crate) fn install_dir_rel(data_root: &Path, install_dir: &Path) -> String {
    install_dir
        .strip_prefix(data_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| install_dir.to_string_lossy().to_string())
}

fn install_target_dir_component(target: InstallTarget) -> Option<&'static str> {
    match target {
        InstallTarget::Host => None,
        InstallTarget::Container => Some("container"),
        InstallTarget::LinuxAarch64 => Some("linux-aarch64"),
        InstallTarget::LinuxX8664 => Some("linux-x86_64"),
    }
}

pub(crate) fn install_dir_for_provider(
    data_root: &Path,
    provider_id: &str,
    version: &str,
    target: InstallTarget,
) -> PathBuf {
    let mut out = data_root
        .join("providers")
        .join("agent-servers")
        .join(provider_id)
        .join(version);
    if let Some(component) = install_target_dir_component(target) {
        out = out.join(component);
    }
    out
}

pub(crate) async fn ensure_node_runtime(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
    target: InstallTarget,
) -> Result<NodeRuntime> {
    let node_target = node_runtime_target_for_install_target(target)?;
    let target_label = node_target.dist_target;
    if matches!(target, InstallTarget::Host) {
        if let Some(bundled) = bundled_assets::bundled_node_runtime() {
            if bundled.version == NODE_VERSION {
                if let Some(npm_cli_js) = bundled.npm_cli.clone() {
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        "node",
                        format!("Using bundled Node runtime v{NODE_VERSION} ({target_label})"),
                        None,
                        None,
                        None,
                    )
                    .await;
                    return Ok(NodeRuntime {
                        node_root: bundled.root,
                        node_bin: bundled.bin,
                        npm_cli_js,
                    });
                }
            } else {
                tracing::warn!(
                    "bundled Node runtime version {} does not match expected {}",
                    bundled.version,
                    NODE_VERSION
                );
            }
        }
    }
    let folder = format!("node-v{NODE_VERSION}-{}", node_target.dist_target);
    let node_root = data_root.join("runtimes").join("node").join(&folder);
    let (node_bin, npm_cli_js) = node_runtime_paths(&node_root, node_target.is_windows);

    if node_bin.exists() && npm_cli_js.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "node",
            format!("Using existing Node runtime v{NODE_VERSION} ({target_label})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(NodeRuntime {
            node_root,
            node_bin,
            npm_cli_js,
        });
    }

    let _lock = node_runtime_install_lock().lock().await;

    if node_bin.exists() && npm_cli_js.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "node",
            format!("Using existing Node runtime v{NODE_VERSION} ({target_label})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(NodeRuntime {
            node_root,
            node_bin,
            npm_cli_js,
        });
    }

    if let Some(parent) = node_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let (archive_ext, archive_label) = if node_target.is_windows {
        ("zip", "zip")
    } else {
        ("tar.gz", "tar_gz")
    };
    let url = format!("https://nodejs.org/dist/v{NODE_VERSION}/{folder}.{archive_ext}");
    let tmp = data_root
        .join("runtimes")
        .join("node")
        .join(format!("{folder}.{archive_ext}"));
    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "node_download",
        format!("Downloading Node runtime from {url}"),
        None,
        None,
        None,
    )
    .await;
    download_to_file(state, install_id, provider_id, "node_download", &url, &tmp).await?;

    let extract_root = data_root
        .join("runtimes")
        .join("node")
        .join(format!("{folder}.extract"));
    if extract_root.exists() {
        tokio::fs::remove_dir_all(&extract_root).await.ok();
    }
    tokio::fs::create_dir_all(&extract_root).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "node_extract",
        "Extracting Node runtime".to_string(),
        None,
        None,
        None,
    )
    .await;

    let tmp2 = tmp.clone();
    let extract_root2 = extract_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if archive_label == "zip" {
            extract_zip_to_dir(&tmp2, &extract_root2)?;
        } else {
            let tar_gz = std::fs::File::open(&tmp2)?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&extract_root2)?;
        }
        Ok(())
    })
    .await??;

    let extracted = extract_root.join(&folder);
    if !extracted.exists() {
        anyhow::bail!(
            "node extraction failed: missing {folder} in {}",
            extract_root.display()
        );
    }

    if node_root.exists() {
        tokio::fs::remove_dir_all(&node_root).await.ok();
    }
    tokio::fs::rename(&extracted, &node_root).await?;
    tokio::fs::remove_dir_all(&extract_root).await.ok();
    tokio::fs::remove_file(&tmp).await.ok();

    if !node_bin.exists() || !npm_cli_js.exists() {
        anyhow::bail!(
            "node runtime incomplete after install (node: {}, npm: {})",
            node_bin.display(),
            npm_cli_js.display()
        );
    }

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Success,
        "node_extract",
        "Node runtime ready".to_string(),
        None,
        None,
        None,
    )
    .await;

    Ok(NodeRuntime {
        node_root,
        node_bin,
        npm_cli_js,
    })
}

fn node_runtime_paths(node_root: &Path, is_windows: bool) -> (PathBuf, PathBuf) {
    if is_windows {
        (
            node_root.join("node.exe"),
            node_root
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js"),
        )
    } else {
        (
            node_root.join("bin").join("node"),
            node_root
                .join("lib")
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js"),
        )
    }
}

pub(crate) fn node_runtime_target_for_install_target(
    target: InstallTarget,
) -> Result<NodeRuntimeTarget> {
    match target {
        InstallTarget::Host => {
            node_runtime_target_for_os_arch(std::env::consts::OS, std::env::consts::ARCH)
        }
        InstallTarget::Container => {
            node_runtime_target_for_os_arch("linux", std::env::consts::ARCH)
        }
        InstallTarget::LinuxAarch64 => node_runtime_target_for_os_arch("linux", "aarch64"),
        InstallTarget::LinuxX8664 => node_runtime_target_for_os_arch("linux", "x86_64"),
    }
}

pub(crate) fn node_runtime_dependency_targets_for_install_target(
    target: InstallTarget,
    host_os: &str,
) -> Vec<InstallTarget> {
    let mut targets = vec![target];
    if matches!(target, InstallTarget::Container) && host_os != "linux" {
        targets.push(InstallTarget::Host);
    }
    targets
}

fn node_runtime_target_for_os_arch(os: &str, arch: &str) -> Result<NodeRuntimeTarget> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "darwin-arm64",
            is_windows: false,
        }),
        ("macos", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "darwin-x64",
            is_windows: false,
        }),
        ("linux", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "linux-arm64",
            is_windows: false,
        }),
        ("linux", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "linux-x64",
            is_windows: false,
        }),
        ("windows", "x86_64") => Ok(NodeRuntimeTarget {
            dist_target: "win-x64",
            is_windows: true,
        }),
        ("windows", "aarch64") => Ok(NodeRuntimeTarget {
            dist_target: "win-arm64",
            is_windows: true,
        }),
        _ => anyhow::bail!(
            "unsupported platform for managed node install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64), windows (aarch64/x86_64)."
        ),
    }
}

pub(crate) fn archive_bin_requires_node_runtime(bin_path: &str, installed_bin_path: &Path) -> bool {
    let ext = Path::new(bin_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("js") | Some("mjs") | Some("cjs")) {
        return true;
    }
    archive_bin_has_node_shebang(installed_bin_path)
}

fn archive_bin_has_node_shebang(path: &Path) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    let bytes = match std::io::BufRead::read_line(&mut reader, &mut first_line) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    if bytes == 0 {
        return false;
    }
    shebang_invokes_node(first_line.trim())
}

fn shebang_invokes_node(line: &str) -> bool {
    let Some(shebang) = line.strip_prefix("#!") else {
        return false;
    };
    let mut tokens = shebang.split_whitespace();
    let Some(program) = tokens.next() else {
        return false;
    };
    if shebang_token_is_node(program) {
        return true;
    }
    if !shebang_token_is_env(program) {
        return false;
    }
    for token in tokens {
        if token.starts_with('-') || token.contains('=') {
            continue;
        }
        return shebang_token_is_node(token);
    }
    false
}

fn shebang_token_is_env(token: &str) -> bool {
    let base = shebang_token_basename(token);
    base.eq_ignore_ascii_case("env")
}

fn shebang_token_is_node(token: &str) -> bool {
    let base = shebang_token_basename(token);
    base.eq_ignore_ascii_case("node") || base.eq_ignore_ascii_case("node.exe")
}

fn shebang_token_basename(token: &str) -> &str {
    token
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
}

pub(crate) fn node_runtime_dependency_id(target: InstallTarget) -> String {
    format!("runtime-node-{}", target.as_str())
}

pub(crate) fn node_runtime_dependency_metadata(
    data_root: &Path,
    node: &NodeRuntime,
    target: InstallTarget,
) -> ManagedInstallMetadata {
    let bin_dir = node
        .node_bin
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| node.node_root.clone());
    ManagedInstallMetadata {
        package: Some("node-runtime".to_string()),
        version: Some(NODE_VERSION.to_string()),
        target: Some(target),
        install_dir_rel: Some(install_dir_rel(data_root, &node.node_root)),
        bin_dir_rel: Some(install_dir_rel(data_root, &bin_dir)),
        last_success_at: Some(Utc::now().to_rfc3339()),
        last_error: None,
    }
}

pub(crate) async fn ensure_python_runtime_versioned(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    data_root: &Path,
    target: InstallTarget,
    python_version: &str,
    python_build_tag: &str,
) -> Result<PythonRuntime> {
    let target_triple = python_target_triple_for_install_target(target)?;
    if python_target_can_use_bundled_runtime(target) {
        if let Some(bundled) = bundled_assets::bundled_python_runtime_version(python_version) {
            if bundled.version == python_version {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Info,
                    "python",
                    format!("Using bundled Python runtime {python_version} ({target_triple})"),
                    None,
                    None,
                    None,
                )
                .await;
                return Ok(PythonRuntime {
                    python_root: bundled.root,
                    python_bin: bundled.bin,
                });
            } else {
                tracing::warn!(
                    "bundled Python runtime version {} does not match expected {}",
                    bundled.version,
                    python_version
                );
            }
        }
    }
    let folder = format!("cpython-{python_version}+{python_build_tag}-{target_triple}");
    let python_root = data_root.join("runtimes").join("python").join(&folder);
    let python_bin = resolve_python_bin(&python_root, target);

    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {python_version} ({target_triple})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(PythonRuntime {
            python_root,
            python_bin,
        });
    }

    let _lock = python_runtime_install_lock().lock().await;
    let python_bin = resolve_python_bin(&python_root, target);
    if python_bin.exists() {
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "python",
            format!("Using existing Python runtime {python_version} ({target_triple})"),
            None,
            None,
            None,
        )
        .await;
        return Ok(PythonRuntime {
            python_root,
            python_bin,
        });
    }

    if let Some(parent) = python_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let asset =
        format!("cpython-{python_version}+{python_build_tag}-{target_triple}-install_only.tar.gz");
    let url = format!(
        "https://github.com/indygreg/python-build-standalone/releases/download/{python_build_tag}/{asset}"
    );
    let tmp = data_root.join("runtimes").join("python").join(&asset);

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "python_download",
        format!("Downloading Python runtime from {url}"),
        None,
        None,
        None,
    )
    .await;
    download_to_file(
        state,
        install_id,
        provider_id,
        "python_download",
        &url,
        &tmp,
    )
    .await?;

    let extract_root = data_root
        .join("runtimes")
        .join("python")
        .join(format!("{folder}.extract"));
    if extract_root.exists() {
        tokio::fs::remove_dir_all(&extract_root).await.ok();
    }
    tokio::fs::create_dir_all(&extract_root).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "python_extract",
        "Extracting Python runtime".to_string(),
        None,
        None,
        None,
    )
    .await;

    let tmp2 = tmp.clone();
    let extract_root2 = extract_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let tar_gz = std::fs::File::open(&tmp2)?;
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(dec);
        archive.unpack(&extract_root2)?;
        Ok(())
    })
    .await??;

    let extracted = extract_root.join("python");
    if !extracted.exists() {
        anyhow::bail!(
            "python extraction failed: missing python/ in {}",
            extract_root.display()
        );
    }

    if python_root.exists() {
        tokio::fs::remove_dir_all(&python_root).await.ok();
    }
    tokio::fs::rename(&extracted, &python_root).await?;
    tokio::fs::remove_dir_all(&extract_root).await.ok();
    tokio::fs::remove_file(&tmp).await.ok();

    let python_bin = resolve_python_bin(&python_root, target);
    if !python_bin.exists() {
        anyhow::bail!(
            "python runtime incomplete after install (python: {})",
            python_bin.display()
        );
    }

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Success,
        "python_extract",
        format!("Installed Python runtime {python_version} ({target_triple})"),
        None,
        None,
        None,
    )
    .await;

    Ok(PythonRuntime {
        python_root,
        python_bin,
    })
}

pub(crate) fn python_target_can_use_bundled_runtime(target: InstallTarget) -> bool {
    matches!(target, InstallTarget::Host)
}

fn python_target_triple_for_install_target(target: InstallTarget) -> Result<&'static str> {
    match target {
        InstallTarget::Host => {
            python_target_triple_for_os_arch(std::env::consts::OS, std::env::consts::ARCH)
        }
        InstallTarget::Container => {
            python_target_triple_for_os_arch("linux", std::env::consts::ARCH)
        }
        InstallTarget::LinuxAarch64 => python_target_triple_for_os_arch("linux", "aarch64"),
        InstallTarget::LinuxX8664 => python_target_triple_for_os_arch("linux", "x86_64"),
    }
}

fn python_target_triple_for_os_arch(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        _ => anyhow::bail!(
            "unsupported platform for managed python install: {os}/{arch}. Supported: macos (aarch64/x86_64), linux (aarch64/x86_64), windows (aarch64/x86_64)."
        ),
    }
}

pub(crate) fn resolve_python_bin(python_root: &Path, target: InstallTarget) -> PathBuf {
    if target_uses_windows_layout(target) {
        python_root.join("python.exe")
    } else {
        let primary = python_root.join("bin").join("python3");
        if primary.exists() {
            primary
        } else {
            python_root.join("bin").join("python")
        }
    }
}

pub(crate) async fn ensure_python_pip(python: &Path) -> Result<()> {
    let mut pip_check = Command::new(python);
    pip_check
        .arg("-m")
        .arg("pip")
        .arg("--version")
        .kill_on_drop(true);
    let out = run_command_with_timeout(pip_check, Duration::from_secs(60))
        .await
        .context("checking pip availability")?;
    if out.status.success() {
        return Ok(());
    }

    let mut ensure = Command::new(python);
    ensure
        .arg("-m")
        .arg("ensurepip")
        .arg("--upgrade")
        .kill_on_drop(true);
    let out = run_command_with_timeout(ensure, Duration::from_secs(5 * 60))
        .await
        .context("running ensurepip")?;
    if !out.status.success() {
        anyhow::bail!(
            "ensurepip failed status={}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

pub(crate) async fn npm_install(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    node: &NodeRuntime,
    install_dir: &Path,
    package_spec: &str,
    target: InstallTarget,
) -> Result<()> {
    let cache_dir = install_dir.join(".npm-cache");
    tokio::fs::create_dir_all(&cache_dir).await.ok();
    let node_bin_dir = node.node_bin.parent().unwrap_or(node.node_root.as_path());
    let path_sep = if cfg!(windows) { ";" } else { ":" };
    let mut combined_path = std::ffi::OsString::new();
    combined_path.push(node_bin_dir);
    combined_path.push(path_sep);
    if let Some(existing) = std::env::var_os("PATH") {
        combined_path.push(existing);
    }
    let pnpm_bin = if matches!(target, InstallTarget::Host) {
        which::which("pnpm").ok()
    } else {
        None
    };
    let package_manager = if pnpm_bin.is_some() { "pnpm" } else { "npm" };

    for attempt in 1..=RETRY_COUNT {
        ensure_install_not_cancelled(state, install_id).await?;
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Info,
            "npm_install",
            format!("{package_manager} install {package_spec} (attempt {attempt}/{RETRY_COUNT})"),
            None,
            None,
            Some(attempt),
        )
        .await;

        let out = if matches!(target, InstallTarget::Container) {
            container_builder::ensure_builder_ready(&state.core.data_root)
                .await
                .context("ensuring container builder readiness")?;
            let mut argv = Vec::with_capacity(16);
            argv.push(node.node_bin.to_string_lossy().to_string());
            argv.push(node.npm_cli_js.to_string_lossy().to_string());
            argv.push("install".to_string());
            argv.push("--prefix".to_string());
            argv.push(install_dir.to_string_lossy().to_string());
            argv.push("--no-audit".to_string());
            argv.push("--no-fund".to_string());
            argv.push("--silent".to_string());
            argv.push("--ignore-scripts".to_string());
            argv.push(package_spec.to_string());
            let env = vec![
                (
                    "PATH".to_string(),
                    combined_path.to_string_lossy().to_string(),
                ),
                (
                    "npm_config_update_notifier".to_string(),
                    "false".to_string(),
                ),
                ("npm_config_fund".to_string(), "false".to_string()),
                ("npm_config_audit".to_string(), "false".to_string()),
                ("npm_config_progress".to_string(), "false".to_string()),
                (
                    "npm_config_cache".to_string(),
                    cache_dir.to_string_lossy().to_string(),
                ),
                ("npm_config_ignore_scripts".to_string(), "true".to_string()),
            ];
            container_builder::run_command(
                &state.core.data_root,
                install_dir,
                &env,
                &argv,
                NPM_INSTALL_TIMEOUT,
            )
            .await
        } else {
            let mut cmd = if let Some(pnpm) = pnpm_bin.as_ref() {
                let mut cmd = Command::new(pnpm);
                cmd.arg("add")
                    .arg("--dir")
                    .arg(install_dir)
                    .arg("--ignore-scripts")
                    .arg("--lockfile=false")
                    .arg("--reporter")
                    .arg("silent")
                    .arg(package_spec);
                cmd
            } else {
                let mut cmd = Command::new(&node.node_bin);
                cmd.arg(&node.npm_cli_js)
                    .arg("install")
                    .arg("--prefix")
                    .arg(install_dir)
                    .arg("--no-audit")
                    .arg("--no-fund")
                    .arg("--silent")
                    .arg("--ignore-scripts")
                    .arg(package_spec)
                    .env("npm_config_update_notifier", "false")
                    .env("npm_config_fund", "false")
                    .env("npm_config_audit", "false")
                    .env("npm_config_progress", "false")
                    .env("npm_config_cache", cache_dir.clone())
                    .env("npm_config_ignore_scripts", "true");
                cmd
            };
            cmd.env("PATH", combined_path.clone()).kill_on_drop(true);
            run_command_with_timeout(cmd, NPM_INSTALL_TIMEOUT).await
        };
        let out = match out {
            Ok(out) => out,
            Err(e) => {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Error,
                    "npm_install",
                    format!("{package_manager} install failed: {e:#}"),
                    None,
                    None,
                    Some(attempt),
                )
                .await;
                if attempt < RETRY_COUNT {
                    tokio::time::sleep(Duration::from_millis(
                        RETRY_BACKOFF_BASE_MS * attempt as u64,
                    ))
                    .await;
                    continue;
                }
                return Err(e.context("running package install"));
            }
        };
        if out.status.success() {
            emit_install(
                state,
                install_id,
                provider_id,
                InstallEventLevel::Success,
                "npm_install",
                format!("{package_manager} install succeeded"),
                None,
                None,
                Some(attempt),
            )
            .await;
            return Ok(());
        }

        let err_txt = format!(
            "{package_manager} install failed ({package_spec}) status={}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        emit_install(
            state,
            install_id,
            provider_id,
            InstallEventLevel::Error,
            "npm_install",
            truncate_for_storage(&err_txt, INSTALL_EVENT_ERROR_MAX_LEN),
            None,
            None,
            Some(attempt),
        )
        .await;
        if attempt < RETRY_COUNT {
            tokio::time::sleep(Duration::from_millis(
                RETRY_BACKOFF_BASE_MS * attempt as u64,
            ))
            .await;
        }
    }

    anyhow::bail!(
        "{package_manager} install failed after {RETRY_COUNT} attempts ({package_spec}). Try again, or check your network / npm registry access."
    );
}

pub(crate) fn sanitize_npm_package_for_path(pkg: &str) -> String {
    pkg.trim()
        .trim_start_matches('@')
        .replace(['/', '\\'], "__")
}

pub(crate) async fn npm_install_one(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    node: &NodeRuntime,
    install_dir: &Path,
    package: &str,
    version: &str,
) -> Result<()> {
    let package_spec = format!("{package}@{version}");
    npm_install(
        state,
        install_id,
        provider_id,
        node,
        install_dir,
        &package_spec,
        InstallTarget::Host,
    )
    .await
}

pub(crate) async fn npm_dependency_matches(
    install_dir: &Path,
    package: &str,
    version: &str,
) -> Result<bool> {
    let pkg_dir = install_dir.join("node_modules").join(package);
    let pkg_json_path = pkg_dir.join("package.json");
    if !pkg_json_path.exists() {
        return Ok(false);
    }
    let txt = tokio::fs::read_to_string(&pkg_json_path)
        .await
        .with_context(|| format!("reading {}", pkg_json_path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&txt).context("parsing package.json")?;
    Ok(v.get("version")
        .and_then(|v| v.as_str())
        .map(|v| v == version)
        .unwrap_or(false))
}

pub(crate) async fn resolve_node_package_bin(
    install_dir: &Path,
    package: &str,
    preferred_bin_name: Option<&str>,
) -> Result<PathBuf> {
    let pkg_dir = install_dir.join("node_modules").join(package);
    let pkg_json_path = pkg_dir.join("package.json");
    let txt = tokio::fs::read_to_string(&pkg_json_path)
        .await
        .with_context(|| format!("reading {}", pkg_json_path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&txt).context("parsing package.json")?;
    let bin = v.get("bin").context("package.json missing bin")?;
    let entry_rel = match bin {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            if let Some(preferred) = preferred_bin_name {
                if let Some(v) = map.get(preferred).and_then(|v| v.as_str()) {
                    v.to_string()
                } else if map.len() == 1 {
                    map.values()
                        .next()
                        .and_then(|v| v.as_str())
                        .context("bin map invalid")?
                        .to_string()
                } else {
                    anyhow::bail!(
                        "bin {preferred} not found in {} (available: {})",
                        package,
                        map.keys().cloned().collect::<Vec<_>>().join(", ")
                    );
                }
            } else if map.len() == 1 {
                map.values()
                    .next()
                    .and_then(|v| v.as_str())
                    .context("bin map invalid")?
                    .to_string()
            } else {
                anyhow::bail!(
                    "multiple bins in {} but no preferred bin specified",
                    package
                );
            }
        }
        _ => anyhow::bail!("package.json bin has unsupported type"),
    };
    Ok(pkg_dir.join(entry_rel))
}
