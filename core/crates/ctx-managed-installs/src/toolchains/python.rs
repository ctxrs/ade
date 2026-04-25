use super::*;

pub async fn ensure_python_runtime_versioned(
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

pub async fn ensure_python_pip(python: &Path) -> Result<()> {
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
