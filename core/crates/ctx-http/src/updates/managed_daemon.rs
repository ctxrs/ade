use super::fs_ops::{now_ms, write_json_atomically};
use super::*;

#[derive(Debug, Clone)]
pub(super) struct ManagedDaemonAutoUpdateSource {
    pub(super) channel: String,
    pub(super) base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedDaemonAutoUpdateStatus {
    pub schema_version: u32,
    pub enabled: bool,
    pub channel: String,
    pub base_url: String,
    pub state: String,
    pub checked_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl ManagedDaemonAutoUpdateStatus {
    const SCHEMA_VERSION: u32 = 1;
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

pub(super) fn managed_daemon_auto_update_source_from_env() -> Option<ManagedDaemonAutoUpdateSource>
{
    if !env_truthy("CTX_MANAGED_DAEMON_AUTO_UPDATE") {
        return None;
    }
    let channel = std::env::var("CTX_DAEMON_UPDATE_CHANNEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let base_url = std::env::var("CTX_DAEMON_UPDATE_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(ManagedDaemonAutoUpdateSource { channel, base_url })
}

fn managed_daemon_auto_update_interval() -> std::time::Duration {
    let secs = std::env::var("CTX_MANAGED_DAEMON_AUTO_UPDATE_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs >= 60)
        .unwrap_or(60 * 60);
    std::time::Duration::from_secs(secs)
}

fn managed_daemon_bundle_dir_from_env() -> Result<PathBuf> {
    let value = std::env::var("CTX_BUNDLE_DIR")
        .context("managed daemon auto-update requires CTX_BUNDLE_DIR for bundle parity")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("managed daemon auto-update requires non-empty CTX_BUNDLE_DIR");
    }
    Ok(PathBuf::from(trimmed))
}

fn managed_daemon_bundle_update_root(data_root: &Path) -> PathBuf {
    updates_dir(data_root).join("managed-daemon-auto-update")
}

fn managed_daemon_auto_update_status_path(data_root: &Path) -> PathBuf {
    managed_daemon_bundle_update_root(data_root).join("status.json")
}

pub async fn managed_daemon_auto_update_status_snapshot(
    data_root: &Path,
) -> Option<ManagedDaemonAutoUpdateStatus> {
    let path = managed_daemon_auto_update_status_path(data_root);
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) async fn write_managed_daemon_auto_update_status(
    data_root: &Path,
    source: &ManagedDaemonAutoUpdateSource,
    state: &str,
    last_error: Option<String>,
) {
    let status = ManagedDaemonAutoUpdateStatus {
        schema_version: ManagedDaemonAutoUpdateStatus::SCHEMA_VERSION,
        enabled: true,
        channel: source.channel.clone(),
        base_url: source.base_url.clone(),
        state: state.to_string(),
        checked_at_ms: now_ms(),
        last_error,
    };
    let path = managed_daemon_auto_update_status_path(data_root);
    if let Err(err) = write_json_atomically(&path, &status).await {
        tracing::warn!(err = %err, path = %path.display(), "failed to write managed daemon auto-update status");
    }
}

async fn download_verified_artifact_to_cache(
    base_url: &str,
    artifact: &ReleaseArtifact,
    final_path: &Path,
) -> Result<()> {
    if final_path.exists() {
        let got = sha256_hex_file(final_path).await?;
        if got.eq_ignore_ascii_case(artifact.sha256.trim()) {
            return Ok(());
        }
        let _ = tokio::fs::remove_file(final_path).await;
    }
    let Some(parent) = final_path.parent() else {
        anyhow::bail!(
            "artifact cache path has no parent: {}",
            final_path.display()
        );
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let partial = final_path.with_extension(format!("partial-{}", std::process::id()));
    let url = join_url(base_url, &artifact.url_path);
    download_to_path(&url, &partial).await?;
    let got = sha256_hex_file(&partial).await?;
    if !got.eq_ignore_ascii_case(artifact.sha256.trim()) {
        let _ = tokio::fs::remove_file(&partial).await;
        anyhow::bail!(
            "checksum mismatch for downloaded artifact {}: expected {}, got {}",
            artifact.url_path,
            artifact.sha256,
            got
        );
    }
    tokio::fs::rename(&partial, final_path)
        .await
        .with_context(|| {
            format!(
                "promoting verified artifact {} -> {}",
                partial.display(),
                final_path.display()
            )
        })?;
    Ok(())
}

async fn download_daemon_update_candidate(
    data_root: &Path,
    manifest: &ReleaseManifest,
    platform: &str,
    source: &ManagedDaemonAutoUpdateSource,
) -> Result<PathBuf> {
    let artifact = manifest
        .platforms
        .get(platform)
        .with_context(|| format!("manifest missing platform entry: {platform}"))?
        .daemon
        .as_ref()
        .with_context(|| format!("manifest missing daemon artifact for {platform}"))?;
    let final_path = managed_daemon_bundle_update_root(data_root)
        .join("daemon")
        .join(&source.channel)
        .join(platform)
        .join(artifact.sha256.trim().to_ascii_lowercase())
        .join("ctx.new");
    download_verified_artifact_to_cache(&source.base_url, artifact, &final_path).await?;
    Ok(final_path)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry.with_context(|| format!("reading entry in {}", src.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", entry.path().display()))?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())
                .with_context(|| format!("reading symlink {}", entry.path().display()))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &target).with_context(|| {
                format!(
                    "creating symlink {} -> {}",
                    target.display(),
                    link_target.display()
                )
            })?;
            #[cfg(not(unix))]
            anyhow::bail!(
                "cannot copy symlink {} on this platform",
                entry.path().display()
            );
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).with_context(|| {
                format!("copying {} -> {}", entry.path().display(), target.display())
            })?;
        } else {
            anyhow::bail!("unsupported bundle entry type: {}", entry.path().display());
        }
    }
    Ok(())
}

fn find_bundles_dir(root: &Path) -> Result<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            if !entry
                .file_type()
                .with_context(|| format!("reading file type for {}", path.display()))?
                .is_dir()
            {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("bundles") {
                return Ok(path);
            }
            stack.push(path);
        }
    }
    anyhow::bail!("managed daemon desktop artifact missing bundles directory");
}

async fn stage_managed_daemon_bundle_update(
    data_root: &Path,
    manifest: &ReleaseManifest,
    platform: &str,
    source: &ManagedDaemonAutoUpdateSource,
) -> Result<PathBuf> {
    let platform_entry = manifest
        .platforms
        .get(platform)
        .with_context(|| format!("manifest missing platform entry: {platform}"))?;
    let artifact = platform_entry
        .preferred_desktop_artifact(platform)
        .with_context(|| format!("manifest missing desktop bundle artifact for {platform}"))?;
    let update_root = managed_daemon_bundle_update_root(data_root)
        .join("bundles")
        .join(&source.channel)
        .join(platform)
        .join(artifact.sha256.trim().to_ascii_lowercase());
    let appimage_path = update_root.join("ctx.AppImage");
    download_verified_artifact_to_cache(&source.base_url, artifact, &appimage_path).await?;

    let staged_bundle_dir = update_root.join(format!("bundles.staged.{}", std::process::id()));
    let extract_root = update_root.join(format!("extract.{}", std::process::id()));
    let appimage_for_extract = appimage_path.clone();
    let staged_for_extract = staged_bundle_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _ = fs::remove_dir_all(&extract_root);
        let _ = fs::remove_dir_all(&staged_for_extract);
        fs::create_dir_all(&extract_root)
            .with_context(|| format!("creating {}", extract_root.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&appimage_for_extract)
                .with_context(|| format!("reading {}", appimage_for_extract.display()))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&appimage_for_extract, perms)
                .with_context(|| format!("chmod {}", appimage_for_extract.display()))?;
        }
        let output = Command::new(&appimage_for_extract)
            .arg("--appimage-extract")
            .current_dir(&extract_root)
            .output()
            .with_context(|| format!("extracting {}", appimage_for_extract.display()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            anyhow::bail!("managed daemon bundle extraction failed: {detail}");
        }
        let bundle_src = find_bundles_dir(&extract_root)?;
        copy_dir_recursive(&bundle_src, &staged_for_extract)?;
        let _ = fs::remove_dir_all(&extract_root);
        Ok(())
    })
    .await
    .context("joining managed daemon bundle extraction task")??;
    Ok(staged_bundle_dir)
}

pub(super) struct ManagedDaemonBundleActivation {
    pub(super) bundle_dir: PathBuf,
    pub(super) backup_dir: PathBuf,
}

pub(super) fn activate_managed_daemon_bundle(
    bundle_dir: PathBuf,
    staged_bundle_dir: PathBuf,
) -> Result<ManagedDaemonBundleActivation> {
    let backup_dir = bundle_dir.with_extension("pre-update-backup");
    let no_previous_marker = backup_dir.join(".ctx-no-previous-bundle");
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir)
            .with_context(|| format!("removing {}", backup_dir.display()))?;
    }
    if bundle_dir.exists() {
        fs::rename(&bundle_dir, &backup_dir).with_context(|| {
            format!(
                "moving current bundle metadata {} -> {}",
                bundle_dir.display(),
                backup_dir.display()
            )
        })?;
    } else {
        fs::create_dir_all(&backup_dir)
            .with_context(|| format!("creating {}", backup_dir.display()))?;
        fs::write(&no_previous_marker, b"")
            .with_context(|| format!("writing {}", no_previous_marker.display()))?;
    }
    if let Err(err) = fs::rename(&staged_bundle_dir, &bundle_dir) {
        if no_previous_marker.exists() {
            let _ = fs::remove_dir_all(&bundle_dir);
            let _ = fs::remove_dir_all(&backup_dir);
        } else if backup_dir.exists() {
            let _ = fs::remove_dir_all(&bundle_dir);
            let _ = fs::rename(&backup_dir, &bundle_dir);
        }
        return Err(err).with_context(|| {
            format!(
                "moving staged bundle metadata {} -> {}",
                staged_bundle_dir.display(),
                bundle_dir.display()
            )
        });
    }
    Ok(ManagedDaemonBundleActivation {
        bundle_dir,
        backup_dir,
    })
}

pub(super) fn restore_managed_daemon_bundle(
    activation: &ManagedDaemonBundleActivation,
) -> Result<()> {
    let no_previous_marker = activation.backup_dir.join(".ctx-no-previous-bundle");
    if no_previous_marker.exists() {
        let _ = fs::remove_dir_all(&activation.bundle_dir);
        fs::remove_dir_all(&activation.backup_dir)
            .with_context(|| format!("removing {}", activation.backup_dir.display()))?;
        return Ok(());
    }
    if !activation.backup_dir.exists() {
        anyhow::bail!(
            "managed daemon bundle backup missing after failed update: {}",
            activation.backup_dir.display()
        );
    }
    let _ = fs::remove_dir_all(&activation.bundle_dir);
    fs::rename(&activation.backup_dir, &activation.bundle_dir).with_context(|| {
        format!(
            "restoring managed daemon bundle {} -> {}",
            activation.backup_dir.display(),
            activation.bundle_dir.display()
        )
    })
}

fn cleanup_managed_daemon_bundle_backup(activation: &ManagedDaemonBundleActivation) -> Result<()> {
    if activation.backup_dir.exists() {
        fs::remove_dir_all(&activation.backup_dir)
            .with_context(|| format!("removing {}", activation.backup_dir.display()))?;
    }
    Ok(())
}

async fn restore_current_exe_backup(current_exe: &Path, backup: &Path) -> Result<()> {
    if !backup.exists() {
        anyhow::bail!(
            "daemon executable backup missing after failed update: {}",
            backup.display()
        );
    }
    let failed_path = current_exe.with_extension("failed-update");
    if failed_path.exists() {
        let _ = tokio::fs::remove_file(&failed_path).await;
    }
    if current_exe.exists() {
        tokio::fs::rename(current_exe, &failed_path)
            .await
            .with_context(|| {
                format!(
                    "moving failed daemon executable {} -> {}",
                    current_exe.display(),
                    failed_path.display()
                )
            })?;
    }
    tokio::fs::rename(backup, current_exe)
        .await
        .with_context(|| {
            format!(
                "restoring daemon executable {} -> {}",
                backup.display(),
                current_exe.display()
            )
        })?;
    Ok(())
}

pub fn spawn_managed_daemon_auto_update(state: Arc<AppState>, bind: Vec<String>) {
    let Some(source) = managed_daemon_auto_update_source_from_env() else {
        return;
    };
    tokio::spawn(async move {
        let interval = managed_daemon_auto_update_interval();
        write_managed_daemon_auto_update_status(&state.core.data_root, &source, "waiting", None)
            .await;
        loop {
            tokio::time::sleep(interval).await;
            write_managed_daemon_auto_update_status(
                &state.core.data_root,
                &source,
                "checking",
                None,
            )
            .await;
            if let Err(err) =
                try_managed_daemon_auto_update(Arc::clone(&state), bind.clone(), source.clone())
                    .await
            {
                write_managed_daemon_auto_update_status(
                    &state.core.data_root,
                    &source,
                    "failed",
                    Some(err.to_string()),
                )
                .await;
                tracing::warn!(err = %err, "managed daemon auto-update attempt failed");
            } else {
                write_managed_daemon_auto_update_status(
                    &state.core.data_root,
                    &source,
                    "idle",
                    None,
                )
                .await;
            }
        }
    });
}

async fn try_managed_daemon_auto_update(
    state: Arc<AppState>,
    bind: Vec<String>,
    source: ManagedDaemonAutoUpdateSource,
) -> Result<()> {
    let Some(platform) = platform_key() else {
        return Ok(());
    };
    let current_version = crate::build_identity::current_build_identity()
        .context("loading daemon build identity for auto-update")?
        .exact_version
        .clone();
    let manifest = fetch_latest_manifest(&source.base_url, &source.channel).await?;
    if !platform_supported(&manifest, Some(platform))
        || !is_update_available(&current_version, &manifest.latest_version, true)
    {
        return Ok(());
    }
    let bundle_dir = managed_daemon_bundle_dir_from_env()?;
    let bundle_candidate =
        stage_managed_daemon_bundle_update(&state.core.data_root, &manifest, platform, &source)
            .await?;
    let daemon_candidate =
        download_daemon_update_candidate(&state.core.data_root, &manifest, platform, &source)
            .await?;
    let Some(_drain) = state
        .acquire_update_drain("managed_daemon_auto_update", "daemon_worker")
        .await
    else {
        return Ok(());
    };
    let activity = crate::daemon::daemon_turn_activity_summary(&state).await?;
    if !activity.idle {
        let _ = state.release_update_drain().await;
        return Ok(());
    }
    let activation =
        activate_managed_daemon_bundle(bundle_dir, bundle_candidate).with_context(|| {
            format!(
                "activating managed daemon bundle metadata for channel {}",
                source.channel
            )
        })?;
    let current_exe = std::env::current_exe().context("resolving current daemon executable")?;
    let binary_backup = match atomic_replace_exe_with_backup(&current_exe, &daemon_candidate).await
    {
        Ok(backup) => backup,
        Err(err) => {
            let _ = restore_managed_daemon_bundle(&activation);
            let _ = state.release_update_drain().await;
            return Err(err);
        }
    };
    if let Err(err) = spawn_replacement_daemon(&state.core.data_root, &bind, &source)
        .context("spawning replacement daemon after managed auto-update")
    {
        let _ = restore_current_exe_backup(&current_exe, &binary_backup).await;
        let _ = restore_managed_daemon_bundle(&activation);
        let _ = state.release_update_drain().await;
        return Err(err);
    }
    if let Err(err) = cleanup_managed_daemon_bundle_backup(&activation) {
        tracing::warn!(err = %err, "failed to clean up managed daemon bundle backup");
    }
    std::process::exit(0);
}

fn spawn_replacement_daemon(
    data_root: &Path,
    bind: &[String],
    source: &ManagedDaemonAutoUpdateSource,
) -> Result<()> {
    let current_exe = std::env::current_exe().context("resolving current daemon executable")?;
    let mut cmd = Command::new(current_exe);
    cmd.arg("serve");
    for bind in bind {
        cmd.arg("--bind").arg(bind);
    }
    cmd.arg("--data-dir").arg(data_root);
    cmd.env("CTX_MANAGED_DAEMON_AUTO_UPDATE", "1")
        .env("CTX_DAEMON_UPDATE_CHANNEL", &source.channel)
        .env("CTX_DAEMON_UPDATE_BASE_URL", &source.base_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("spawning replacement daemon")?;
    Ok(())
}
