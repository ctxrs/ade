use super::*;
use std::ffi::OsString;
use std::io::Read;
use std::path::Component;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

fn normalize_archive_entry_path(raw_path: &Path, label: &str) -> Result<PathBuf> {
    let raw_display = raw_path.display().to_string();
    if raw_display.contains('\\') {
        anyhow::bail!("{label} must not contain backslashes: {raw_display}");
    }

    let mut normalized = PathBuf::new();
    for component in raw_path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!("{label} must not contain parent directory segments: {raw_display}");
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("{label} must be relative: {raw_display}");
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        anyhow::bail!("{label} is empty");
    }
    Ok(normalized)
}

fn safe_archive_dest(out_dir: &Path, raw_path: &Path, label: &str) -> Result<PathBuf> {
    Ok(out_dir.join(normalize_archive_entry_path(raw_path, label)?))
}

fn reject_existing_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!(
                "archive extraction refused to write through symlink: {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

fn ensure_no_symlink_ancestors(out_dir: &Path, dest: &Path) -> Result<()> {
    let rel = dest
        .strip_prefix(out_dir)
        .with_context(|| format!("archive destination escaped root: {}", dest.display()))?;
    let Some(parent) = rel.parent() else {
        return Ok(());
    };

    let mut current = out_dir.to_path_buf();
    for component in parent.components() {
        match component {
            Component::Normal(segment) => {
                current.push(segment);
                reject_existing_symlink(&current)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "archive destination has unsafe ancestor: {}",
                    dest.display()
                );
            }
        }
    }
    Ok(())
}

fn prepare_archive_entry_parent(out_dir: &Path, dest: &Path) -> Result<()> {
    ensure_no_symlink_ancestors(out_dir, dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    ensure_no_symlink_ancestors(out_dir, dest)?;
    Ok(())
}

fn create_archive_dir(out_dir: &Path, dest: &Path) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
    reject_existing_symlink(dest)?;
    std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    reject_existing_symlink(dest)?;
    Ok(())
}

fn validate_symlink_target(out_dir: &Path, dest: &Path, target: &Path) -> Result<()> {
    let target_display = target.display().to_string();
    if target_display.is_empty() {
        anyhow::bail!("archive symlink target is empty for {}", dest.display());
    }
    if target_display.contains('\\') {
        anyhow::bail!("archive symlink target must not contain backslashes: {target_display}");
    }

    let dest_rel = dest.strip_prefix(out_dir).with_context(|| {
        format!(
            "archive symlink destination escaped root: {}",
            dest.display()
        )
    })?;
    let mut stack: Vec<OsString> = dest_rel
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| match component {
                    Component::Normal(segment) => Some(segment.to_os_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for component in target.components() {
        match component {
            Component::Normal(segment) => stack.push(segment.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.pop().is_none() {
                    anyhow::bail!(
                        "archive symlink target escapes extraction root: {} -> {}",
                        dest.display(),
                        target.display()
                    );
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "archive symlink target must be relative: {} -> {}",
                    dest.display(),
                    target.display()
                );
            }
        }
    }
    Ok(())
}

fn create_archive_symlink(out_dir: &Path, dest: &Path, target: &Path) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
    reject_existing_symlink(dest)?;
    if std::fs::symlink_metadata(dest).is_ok() {
        anyhow::bail!(
            "archive extraction refused to replace existing path with symlink: {}",
            dest.display()
        );
    }
    validate_symlink_target(out_dir, dest, target)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dest).with_context(|| {
            format!("create symlink {} -> {}", dest.display(), target.display())
        })?;
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        let _ = (dest, target);
        anyhow::bail!("archive symlink entries are not supported on this platform");
    }
}

fn create_archive_file<R: Read>(
    out_dir: &Path,
    dest: &Path,
    reader: &mut R,
    mode: Option<u32>,
) -> Result<()> {
    prepare_archive_entry_parent(out_dir, dest)?;
    reject_existing_symlink(dest)?;
    let mut out =
        std::fs::File::create(dest).with_context(|| format!("create {}", dest.display()))?;
    std::io::copy(reader, &mut out).context("extract archive entry")?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode & 0o777))
            .with_context(|| format!("chmod {}", dest.display()))?;
    }
    Ok(())
}

pub(crate) fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip")?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).context("zip entry")?;
        let entry_name = f.name().to_string();
        let dest = safe_archive_dest(out_dir, Path::new(&entry_name), "zip entry path")?;
        let mode = f.unix_mode();
        let file_type = mode.unwrap_or(0) & 0o170000;
        if f.is_dir() || file_type == 0o040000 {
            create_archive_dir(out_dir, &dest)?;
            continue;
        }

        if file_type == 0o120000 {
            let mut target = String::new();
            f.read_to_string(&mut target)
                .context("read zip symlink target")?;
            create_archive_symlink(out_dir, &dest, Path::new(&target))?;
            continue;
        }

        if file_type != 0 && file_type != 0o100000 {
            anyhow::bail!("unsupported zip entry type for {}", entry_name);
        }
        create_archive_file(out_dir, &dest, &mut f, mode)?;
    }
    Ok(())
}

pub(crate) fn extract_tar_gz_to_dir(tar_gz_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_gz = std::fs::File::open(tar_gz_path)
        .with_context(|| format!("open {}", tar_gz_path.display()))?;
    let dec = flate2::read::GzDecoder::new(tar_gz);
    extract_tar_stream_to_dir(dec, out_dir, "tar.gz")
}

pub(crate) fn extract_tar_bz2_to_dir(tar_bz2_path: &Path, out_dir: &Path) -> Result<()> {
    let tar_bz2 = std::fs::File::open(tar_bz2_path)
        .with_context(|| format!("open {}", tar_bz2_path.display()))?;
    let dec = bzip2::read::BzDecoder::new(tar_bz2);
    extract_tar_stream_to_dir(dec, out_dir, "tar.bz2")
}

fn extract_tar_stream_to_dir<R: Read>(reader: R, out_dir: &Path, label: &str) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .with_context(|| format!("read {label} entries"))?
    {
        let mut entry = entry.with_context(|| format!("read {label} entry"))?;
        let raw_path = entry
            .path()
            .with_context(|| format!("read {label} entry path"))?
            .into_owned();
        let dest = safe_archive_dest(out_dir, &raw_path, "tar entry path")?;
        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            create_archive_dir(out_dir, &dest)?;
            continue;
        }
        if entry_type.is_file() {
            let mode = entry.header().mode().ok();
            create_archive_file(out_dir, &dest, &mut entry, mode)?;
            continue;
        }
        if entry_type.is_symlink() {
            let target = entry
                .link_name()
                .with_context(|| format!("read {label} symlink target"))?
                .ok_or_else(|| {
                    anyhow::anyhow!("tar symlink missing target: {}", raw_path.display())
                })?
                .into_owned();
            create_archive_symlink(out_dir, &dest, &target)?;
            continue;
        }
        if entry_type.is_hard_link() {
            anyhow::bail!(
                "archive hardlinks are not supported: {}",
                raw_path.display()
            );
        }

        match entry_type.as_byte() {
            b'g' | b'x' | b'L' | b'K' => {}
            kind => anyhow::bail!(
                "unsupported tar entry type {} for {}",
                kind,
                raw_path.display()
            ),
        }
    }
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
    let Some(expected_sha256) = expected_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
    else {
        anyhow::bail!("provider matrix archive target is missing required sha256");
    };
    validate_expected_sha256(&expected_sha256)?;

    let tmp_dir = data_root.join("providers").join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp = tmp_dir.join(agent_server_download_tmp_name(
        provider_id,
        version,
        target,
        url,
        Some(&expected_sha256),
    ));
    let staging_dir = prepare_atomic_install_dir(&install_dir).await?;

    *stage = "download";
    download_to_file(state, install_id, event_provider_id, "download", url, &tmp).await?;

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
    if let Err(error) = validate_sha256_digest(&expected_sha256, &digest) {
        tokio::fs::remove_file(&tmp).await.ok();
        return Err(error);
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
            extract_tar_gz_to_dir(&tmp, &staging_dir)?;
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

pub(crate) fn validate_expected_sha256(expected_sha256: &str) -> Result<()> {
    let expected_sha256 = expected_sha256.trim();
    if expected_sha256.len() == 64 && expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    anyhow::bail!("provider matrix archive target has invalid sha256");
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

#[cfg(test)]
mod archive_extraction_tests {
    use super::*;
    use std::io::Write;

    fn write_tar_gz(
        path: &Path,
        write_entries: impl FnOnce(
            &mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>,
        ) -> Result<()>,
    ) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        write_entries(&mut builder)?;
        let encoder = builder.into_inner()?;
        encoder.finish()?;
        Ok(())
    }

    #[test]
    fn archive_extraction_zip_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("traversal.zip");
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        zip.start_file("../escape.txt", options)
            .expect("start zip entry");
        zip.write_all(b"escape").expect("write zip entry");
        zip.finish().expect("finish zip");

        let out_dir = temp.path().join("out");
        let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("zip traversal should fail");
        assert!(
            err.to_string().contains("parent directory"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn archive_extraction_tar_gz_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("traversal.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let data = b"escape";
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, "../escape.txt", &data[..])?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        let err =
            extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("tar traversal should fail");
        assert!(
            err.to_string().contains("parent directory"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn archive_extraction_tar_gz_rejects_symlink_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("symlink-escape.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_path("bin/ctx")?;
            header.set_link_name("../../outside")?;
            header.set_cksum();
            builder.append(&header, std::io::empty())?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        let err =
            extract_tar_gz_to_dir(&tar_path, &out_dir).expect_err("symlink escape should fail");
        assert!(
            err.to_string().contains("escapes extraction root"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("outside").exists());
    }

    #[test]
    fn archive_extraction_zip_rejects_symlink_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zip_path = temp.path().join("symlink-escape.zip");
        let file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o120777);
        zip.start_file("bin/ctx", options)
            .expect("start zip symlink");
        zip.write_all(b"../../outside").expect("write zip symlink");
        zip.finish().expect("finish zip");

        let out_dir = temp.path().join("out");
        let err = extract_zip_to_dir(&zip_path, &out_dir).expect_err("symlink escape should fail");
        assert!(
            err.to_string().contains("escapes extraction root"),
            "unexpected error: {err:#}"
        );
        assert!(!temp.path().join("outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn archive_extraction_tar_gz_allows_in_root_symlink() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tar_path = temp.path().join("safe-symlink.tar.gz");
        write_tar_gz(&tar_path, |builder| {
            let mut dir_header = tar::Header::new_gnu();
            dir_header.set_entry_type(tar::EntryType::Directory);
            dir_header.set_mode(0o755);
            dir_header.set_size(0);
            dir_header.set_cksum();
            builder.append_data(&mut dir_header, "bin", std::io::empty())?;

            let mut symlink_header = tar::Header::new_gnu();
            symlink_header.set_entry_type(tar::EntryType::Symlink);
            symlink_header.set_mode(0o777);
            symlink_header.set_size(0);
            symlink_header.set_path("bin/npm")?;
            symlink_header.set_link_name("../lib/node_modules/npm/bin/npm-cli.js")?;
            symlink_header.set_cksum();
            builder.append(&symlink_header, std::io::empty())?;
            Ok(())
        })
        .expect("write tar.gz");

        let out_dir = temp.path().join("out");
        extract_tar_gz_to_dir(&tar_path, &out_dir).expect("extract safe symlink");
        let target = std::fs::read_link(out_dir.join("bin/npm")).expect("read symlink");
        assert_eq!(target, Path::new("../lib/node_modules/npm/bin/npm-cli.js"));
    }
}
