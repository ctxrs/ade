use super::*;

pub(crate) fn resolve_command_path(command: &str) -> (bool, Option<PathBuf>) {
    if command.contains(std::path::MAIN_SEPARATOR)
        || command.contains('/')
        || command.contains('\\')
    {
        let p = PathBuf::from(command);
        if p.exists() {
            return (true, Some(p));
        }
        return (false, None);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(command);
        if candidate.exists() {
            return (true, Some(candidate));
        }
    }
    (false, None)
}

pub(crate) fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}

pub(crate) fn find_unique_path_ending_with(root: &Path, suffix: &str) -> Result<PathBuf> {
    let suffix = suffix.replace('\\', "/");
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path);
            if rel.to_string_lossy().replace('\\', "/").ends_with(&suffix) {
                matches.push(path);
            }
        }
    }
    if matches.len() == 1 {
        return Ok(matches.remove(0));
    }
    if matches.is_empty() {
        anyhow::bail!("could not find extracted binary ending with {suffix}");
    }
    anyhow::bail!("multiple extracted binaries match {suffix}");
}

pub(crate) fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip")?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).context("zip entry")?;
        let name = f.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let dest = out_dir.join(name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut f, &mut out).context("extract zip entry")?;
    }
    Ok(())
}

pub(crate) fn extract_tar_bz2_to_dir(tar_bz2_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_bz2 = std::fs::File::open(tar_bz2_path)
        .with_context(|| format!("open {}", tar_bz2_path.display()))?;
    let dec = bzip2::read::BzDecoder::new(tar_bz2);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(out_dir).context("extract tar.bz2")?;
    Ok(())
}

pub(crate) async fn prepare_atomic_install_dir(install_dir: &Path) -> Result<PathBuf> {
    let parent = install_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("install dir has no parent: {}", install_dir.display()))?;
    tokio::fs::create_dir_all(parent).await.ok();
    let install_name = install_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("install");
    let staging_dir = parent.join(format!(
        ".{install_name}.staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if staging_dir.exists() {
        tokio::fs::remove_dir_all(&staging_dir).await.ok();
    }
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .with_context(|| format!("creating staging dir: {}", staging_dir.display()))?;
    Ok(staging_dir)
}

pub(crate) async fn commit_atomic_install_dir(
    staging_dir: &Path,
    install_dir: &Path,
) -> Result<()> {
    if install_dir.exists() {
        tokio::fs::remove_dir_all(install_dir).await.ok();
    }
    tokio::fs::rename(staging_dir, install_dir)
        .await
        .with_context(|| {
            format!(
                "committing staging dir {} -> {}",
                staging_dir.display(),
                install_dir.display()
            )
        })?;
    Ok(())
}

pub(crate) fn agent_server_download_tmp_name(
    provider_id: &str,
    version: &str,
    target: InstallTarget,
    url: &str,
    expected_sha256: Option<&str>,
) -> String {
    let identity = expected_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("sha256-{}", value.to_ascii_lowercase()))
        .unwrap_or_else(|| {
            let mut hasher = sha2::Sha256::new();
            hasher.update(url.as_bytes());
            format!("url-{:x}", hasher.finalize())
        });
    format!(
        "{provider_id}-{version}-{target}-{identity}.download",
        target = target.as_str()
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn install_agent_server_url_binary(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    event_provider_id: &str,
    version: &str,
    url: &str,
    expected_sha256: Option<&str>,
    archive: AgentServerArchive,
    bin_path: &str,
    target: InstallTarget,
    stage: &mut &'static str,
) -> Result<PathBuf> {
    let data_root = state.data_root();
    let install_dir = install_dir_for_provider(data_root, provider_id, version, target);
    let expected_sha256 = expected_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    let tmp_dir = data_root.join("providers").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(agent_server_download_tmp_name(
        provider_id,
        version,
        target,
        url,
        expected_sha256.as_deref(),
    ));
    let staging_dir = prepare_atomic_install_dir(&install_dir).await?;

    *stage = "download";
    download_to_file(state, install_id, event_provider_id, "download", url, &tmp).await?;

    if let Some(expected_sha256) = expected_sha256.as_deref() {
        *stage = "verify";
        emit_install(
            state,
            install_id,
            event_provider_id,
            InstallEventLevel::Info,
            "verify",
            "Verifying archive checksum".to_string(),
            None,
            None,
            None,
        )
        .await;

        let digest = sha256_file(&tmp).await?;
        if let Err(error) = validate_sha256_digest(expected_sha256, &digest) {
            tokio::fs::remove_file(&tmp).await.ok();
            return Err(error);
        }
    } else {
        emit_install(
            state,
            install_id,
            event_provider_id,
            InstallEventLevel::Warning,
            "verify",
            "Archive checksum missing in provider matrix; proceeding without verification"
                .to_string(),
            None,
            None,
            None,
        )
        .await;
    }

    *stage = "extract";
    emit_install(
        state,
        install_id,
        event_provider_id,
        InstallEventLevel::Info,
        "extract",
        "Extracting…".to_string(),
        None,
        None,
        None,
    )
    .await;

    let resolved_in_staging = match archive {
        AgentServerArchive::None => {
            let dest = staging_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::rename(&tmp, &dest).with_context(|| {
                format!("move downloaded binary into staging: {}", dest.display())
            })?;
            ensure_executable(&dest)?;
            dest
        }
        AgentServerArchive::TarGz => {
            let tar_gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&staging_dir).context("extract tar.gz")?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::TarBz2 => {
            extract_tar_bz2_to_dir(&tmp, &staging_dir)?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::Zip => {
            extract_zip_to_dir(&tmp, &staging_dir)?;
            let direct = staging_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&staging_dir, bin_path)?
            }
        }
        AgentServerArchive::Dmg => {
            tokio::fs::remove_dir_all(&staging_dir).await.ok();
            anyhow::bail!("dmg archive extraction is not supported in managed installs")
        }
    };
    ensure_executable(&resolved_in_staging)?;

    let should_remove_tmp = !matches!(archive, AgentServerArchive::None);
    let relative_bin = resolved_in_staging
        .strip_prefix(&staging_dir)
        .ok()
        .map(|path| path.to_path_buf());

    if let Err(error) = commit_atomic_install_dir(&staging_dir, &install_dir).await {
        tokio::fs::remove_dir_all(&staging_dir).await.ok();
        return Err(error);
    }

    let resolved = if let Some(relative_bin) = relative_bin {
        let candidate = install_dir.join(relative_bin);
        if candidate.exists() {
            candidate
        } else {
            let direct = install_dir.join(bin_path);
            if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            }
        }
    } else {
        let direct = install_dir.join(bin_path);
        if direct.exists() {
            direct
        } else {
            find_unique_path_ending_with(&install_dir, bin_path)?
        }
    };
    if should_remove_tmp {
        tokio::fs::remove_file(&tmp).await.ok();
    }
    ensure_executable(&resolved)?;
    Ok(resolved)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn install_url_binary(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    server_id: &str,
    version: &str,
    url: &str,
    archive: LspCatalogArchive,
    bin_path: &str,
) -> Result<PathBuf> {
    let data_root = state.data_root();
    let install_dir = data_root
        .join("lsp")
        .join("binaries")
        .join(server_id)
        .join(version);
    tokio::fs::create_dir_all(&install_dir).await.ok();

    let tmp_dir = data_root.join("lsp").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(format!("{server_id}-{version}.download"));

    download_to_file(state, install_id, provider_id, "download", url, &tmp).await?;

    emit_install(
        state,
        install_id,
        provider_id,
        InstallEventLevel::Info,
        "extract",
        "Extracting…".to_string(),
        None,
        None,
        None,
    )
    .await;

    match archive {
        LspCatalogArchive::None => {
            let dest = install_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::rename(&tmp, &dest).await.ok();
            ensure_executable(&dest)?;
            Ok(dest)
        }
        LspCatalogArchive::Gz => {
            let gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let mut dec = flate2::read::GzDecoder::new(gz);
            let dest = install_dir.join(bin_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out = std::fs::File::create(&dest)
                .with_context(|| format!("create {}", dest.display()))?;
            std::io::copy(&mut dec, &mut out).context("decompress gz")?;
            ensure_executable(&dest)?;
            Ok(dest)
        }
        LspCatalogArchive::TarGz => {
            let tar_gz =
                std::fs::File::open(&tmp).with_context(|| format!("open {}", tmp.display()))?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            archive.unpack(&install_dir).context("extract tar.gz")?;
            let direct = install_dir.join(bin_path);
            let resolved = if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            };
            ensure_executable(&resolved)?;
            Ok(resolved)
        }
        LspCatalogArchive::Zip => {
            extract_zip_to_dir(&tmp, &install_dir)?;
            let direct = install_dir.join(bin_path);
            let resolved = if direct.exists() {
                direct
            } else {
                find_unique_path_ending_with(&install_dir, bin_path)?
            };
            ensure_executable(&resolved)?;
            Ok(resolved)
        }
    }
}

pub(crate) async fn download_to_file(
    state: &AppState,
    install_id: Option<InstallId>,
    provider_id: &str,
    stage: &str,
    url: &str,
    path: &Path,
) -> Result<()> {
    for attempt in 1..=RETRY_COUNT {
        ensure_install_not_cancelled(state, install_id).await?;
        let attempt_res: Result<()> = async {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }

            if url.starts_with("file://") {
                let u = url::Url::parse(url).context("parsing file:// url")?;
                let src = u
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file url: {url}"))?;
                tokio::fs::copy(&src, path)
                    .await
                    .with_context(|| format!("copying {} -> {}", src.display(), path.display()))?;
            } else {
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(DOWNLOAD_TIMEOUT)
                    .build()
                    .context("building http client")?;
                let existing_len = tokio::fs::metadata(path)
                    .await
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                let mut request = client.get(url);
                if existing_len > 0 {
                    use reqwest::header::RANGE;
                    request = request.header(RANGE, format!("bytes={existing_len}-"));
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        stage,
                        format!("resuming download from byte {existing_len}"),
                        Some(existing_len),
                        None,
                        Some(attempt),
                    )
                    .await;
                }

                let resp = request.send().await.context("sending request")?;
                let status = resp.status();
                if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                    tokio::fs::remove_file(path).await.ok();
                    anyhow::bail!("server rejected ranged resume request");
                }
                let resp = resp.error_for_status().context("http error")?;

                let (resumed, total) =
                    resolve_download_resume(existing_len, status, resp.content_length());
                let mut stream = resp.bytes_stream();
                let mut file = if resumed {
                    tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .await
                        .with_context(|| {
                            format!("opening download target for append: {}", path.display())
                        })?
                } else {
                    if existing_len > 0 {
                        emit_install(
                            state,
                            install_id,
                            provider_id,
                            InstallEventLevel::Warning,
                            stage,
                            "server does not support resume; restarting download from byte 0"
                                .to_string(),
                            None,
                            total,
                            Some(attempt),
                        )
                        .await;
                    }
                    tokio::fs::File::create(path)
                        .await
                        .with_context(|| format!("creating download target: {}", path.display()))?
                };
                use futures::StreamExt;
                use tokio::io::AsyncWriteExt;

                let mut downloaded: u64 = if resumed { existing_len } else { 0 };
                while let Some(chunk) = stream.next().await {
                    ensure_install_not_cancelled(state, install_id).await?;
                    let bytes = chunk.context("streaming download")?;
                    downloaded += bytes.len() as u64;
                    file.write_all(&bytes).await.context("writing download")?;
                    emit_install(
                        state,
                        install_id,
                        provider_id,
                        InstallEventLevel::Info,
                        stage,
                        "downloading…".to_string(),
                        Some(downloaded),
                        total,
                        Some(attempt),
                    )
                    .await;
                }
                file.flush().await.context("flushing download")?;
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Success,
                    stage,
                    "download complete".to_string(),
                    Some(downloaded),
                    total,
                    Some(attempt),
                )
                .await;
            }
            Ok(())
        }
        .await;

        match attempt_res {
            Ok(()) => return Ok(()),
            Err(e) => {
                emit_install(
                    state,
                    install_id,
                    provider_id,
                    InstallEventLevel::Error,
                    stage,
                    format!("download failed: {e}"),
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
                return Err(e);
            }
        }
    }

    Ok(())
}

pub(crate) fn resolve_download_resume(
    existing_len: u64,
    status: reqwest::StatusCode,
    content_length: Option<u64>,
) -> (bool, Option<u64>) {
    let resumed = existing_len > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total = match content_length {
        Some(remaining) if resumed => Some(existing_len.saturating_add(remaining)),
        other => other,
    };
    (resumed, total)
}

pub(crate) async fn run_command_with_timeout(
    mut cmd: Command,
    dur: Duration,
) -> Result<std::process::Output> {
    let child = cmd.spawn().context("spawning process")?;
    let wait = async move { child.wait_with_output().await };
    match timeout(dur, wait).await {
        Ok(res) => Ok(res.context("waiting for process")?),
        Err(_) => anyhow::bail!("process timed out after {}s", dur.as_secs()),
    }
}

pub(crate) fn validate_sha256_digest(expected_sha256: &str, digest: &str) -> Result<()> {
    let expected_sha256 = expected_sha256.trim();
    if digest.eq_ignore_ascii_case(expected_sha256) {
        return Ok(());
    }
    anyhow::bail!("archive checksum mismatch: expected {expected_sha256}, got {digest}");
}

pub(crate) async fn sha256_file(path: &Path) -> Result<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
