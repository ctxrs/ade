use super::*;
use tokio::io::AsyncReadExt;

fn image_load_heartbeat_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(5)
    }
}

fn image_load_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(25)
    } else {
        Duration::from_secs(1)
    }
}

fn image_post_load_visibility_timeout() -> Duration {
    if cfg!(test) {
        Duration::from_millis(500)
    } else {
        Duration::from_secs(10)
    }
}

fn format_image_load_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let minutes = secs / 60;
    let seconds = secs % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

pub(crate) fn resolve_container_image(settings: &ContainerExecutionSettings) -> String {
    if let Ok(value) = std::env::var("CTX_HARNESS_CONTAINER_IMAGE") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    settings
        .image
        .clone()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CONTAINER_IMAGE.to_string())
}

pub fn default_container_image() -> &'static str {
    DEFAULT_CONTAINER_IMAGE
}

pub fn is_default_container_image(image: &str) -> bool {
    image.trim() == DEFAULT_CONTAINER_IMAGE
}

pub fn bundled_default_container_image_tar() -> Option<PathBuf> {
    bundled_assets::bundled_ctx_harness_image_tar(DEFAULT_CONTAINER_IMAGE)
}

pub(crate) async fn prefetch_container_startup_artifacts_with_overrides(
    data_root: &Path,
    image: &str,
    overrides: Option<&ManagedContainerBootstrapOverrides>,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let prefetch_default_image_tar =
        image == DEFAULT_CONTAINER_IMAGE && bundled_default_container_image_tar().is_none();
    let image_tar_download = async {
        if prefetch_default_image_tar {
            return ensure_managed_default_container_image_tar_with_override(
                data_root,
                overrides.and_then(|value| value.default_image_source.as_ref()),
                observer,
                Some(ManagedDownloadAggregate::default()),
            )
            .await
            .map(Some);
        }
        Ok(None)
    };
    let prefetched_image_tar = image_tar_download.await?;
    if let Some(prefetched_image_tar) = prefetched_image_tar {
        observe_log(
            observer,
            HarnessSetupPhase::ImageCheck,
            HarnessSetupLogLevel::Info,
            &format!(
                "prefetched default harness image tar at {}",
                prefetched_image_tar.display()
            ),
        );
    }
    Ok(())
}

pub async fn prefetch_container_startup_artifacts_with_observer(
    data_root: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    prefetch_container_startup_artifacts_with_overrides(data_root, image, None, observer).await
}

pub async fn prefetch_container_image_with_observer(
    data_root: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    prefetch_container_image_with_overrides(data_root, image, None, observer).await
}

pub(crate) async fn prefetch_container_image_with_overrides(
    data_root: &Path,
    image: &str,
    overrides: Option<&ManagedContainerBootstrapOverrides>,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    prefetch_container_startup_artifacts_with_overrides(data_root, image, overrides, observer)
        .await?;
    if !sandbox_engine_ready(data_root).await.unwrap_or(false) {
        anyhow::bail!("native sandbox container runtime is not reachable for image prewarm");
    }
    observe_phase(
        observer,
        HarnessSetupPhase::ImageCheck,
        "checking harness image availability",
    );
    if container_image_present(data_root, image).await? {
        observe_log(
            observer,
            HarnessSetupPhase::ImageCheck,
            HarnessSetupLogLevel::Info,
            "harness image already present",
        );
        return Ok(());
    }
    observe_phase(
        observer,
        HarnessSetupPhase::ImageLoad,
        "loading harness image into local sandbox runtime",
    );
    ensure_container_image_available(data_root, image, observer).await
}

pub async fn prefetch_container_image(data_root: &Path, image: &str) -> Result<()> {
    prefetch_container_image_with_observer(data_root, image, None).await
}

pub async fn container_image_present(data_root: &Path, image: &str) -> Result<bool> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("image").arg("inspect").arg(image);
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    match output.status.code() {
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "container image inspect failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

pub(super) async fn ensure_container_image_available(
    data_root: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }

    if container_image_present(data_root, image).await? {
        return Ok(());
    }

    if image == DEFAULT_CONTAINER_IMAGE {
        let image_tar = if let Some(tar) = bundled_assets::bundled_ctx_harness_image_tar(image) {
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from bundled tar {}",
                    tar.display()
                ),
            );
            tar
        } else {
            let managed_tar =
                ensure_managed_default_container_image_tar(data_root, observer).await?;
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from managed cache {}",
                    managed_tar.display()
                ),
            );
            managed_tar
        };
        load_container_image_tar(data_root, &image_tar, image, observer).await?;
        return Ok(());
    }

    anyhow::bail!(
        "container image '{}' is not present; registry pulls are disabled, so the image must already exist in the local sandbox runtime",
        image
    );
}

pub(super) async fn force_reload_default_container_image(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image_tar = if let Some(tar) =
        bundled_assets::bundled_ctx_harness_image_tar(DEFAULT_CONTAINER_IMAGE)
    {
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "reloading default harness image from bundled tar {}",
                tar.display()
            ),
        );
        tar
    } else {
        let managed_tar = ensure_managed_default_container_image_tar(data_root, observer).await?;
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "reloading default harness image from managed cache {}",
                managed_tar.display()
            ),
        );
        managed_tar
    };
    load_container_image_tar(data_root, &image_tar, DEFAULT_CONTAINER_IMAGE, observer).await
}

fn managed_default_container_image_tar_path(data_root: &Path, sha256: &str) -> PathBuf {
    data_root
        .join("managed")
        .join("images")
        .join("ctx-harness")
        .join("linux")
        .join(std::env::consts::ARCH)
        .join(format!("sha256-{}.tar", sha256.trim().to_ascii_lowercase()))
}

pub(super) fn managed_default_image_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn ensure_managed_default_container_image_tar(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    ensure_managed_default_container_image_tar_with_override(data_root, None, observer, None).await
}

pub(super) async fn ensure_managed_default_container_image_tar_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedArtifactSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let source = match source_override.cloned() {
        Some(source) => source,
        None => bundled_assets::managed_ctx_harness_image_source(DEFAULT_CONTAINER_IMAGE)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "default harness image is missing from bundle and runtime lock managed sources"
                )
            })?,
    };
    ensure_managed_default_container_image_tar_with_source(
        data_root,
        &source,
        observer,
        download_aggregate,
    )
    .await
}

pub(super) async fn ensure_managed_default_container_image_tar_with_source(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let _install_guard = managed_default_image_install_lock().lock().await;

    let final_tar = managed_default_container_image_tar_path(data_root, &source.sha256);
    if final_tar.exists() {
        let digest = updates::sha256_hex_file(&final_tar)
            .await
            .with_context(|| format!("computing sha256 for {}", final_tar.display()))?;
        if digest.eq_ignore_ascii_case(source.sha256.trim()) {
            return Ok(final_tar);
        }
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Warn,
            &format!(
                "managed image cache checksum mismatch for {}; re-downloading",
                final_tar.display()
            ),
        );
        let _ = fs::remove_file(&final_tar).await;
    }

    let Some(parent) = final_tar.parent() else {
        anyhow::bail!(
            "managed image cache path has no parent: {}",
            final_tar.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let tmp_tar = final_tar.with_extension("download");

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!("downloading default harness image from {}", source.uri),
    );
    download_managed_artifact(
        &source.uri,
        &tmp_tar,
        Some(ManagedArtifactDownloadReporter::new(
            observer,
            download_aggregate,
            HarnessSetupPhase::ArtifactDownload,
            "Harness image",
        )),
    )
    .await?;

    let digest = updates::sha256_hex_file(&tmp_tar)
        .await
        .with_context(|| format!("computing sha256 for {}", tmp_tar.display()))?;
    if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
        let _ = fs::remove_file(&tmp_tar).await;
        anyhow::bail!(
            "managed harness image checksum mismatch: expected {}, got {}",
            source.sha256.trim(),
            digest
        );
    }
    fs::rename(&tmp_tar, &final_tar).await.with_context(|| {
        format!(
            "moving managed image tar into place: {} -> {}",
            tmp_tar.display(),
            final_tar.display()
        )
    })?;
    Ok(final_tar)
}

async fn read_child_pipe<R>(mut reader: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

async fn load_container_image_tar(
    data_root: &Path,
    tar: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("load").arg("-i").arg(tar);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning container image load for {}", tar.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("container image load stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("container image load stderr was not captured")?;
    let stdout_task = tokio::spawn(read_child_pipe(stdout));
    let stderr_task = tokio::spawn(read_child_pipe(stderr));
    let deadline = tokio::time::Instant::now() + SANDBOX_IMAGE_LOAD_TIMEOUT;
    let started = tokio::time::Instant::now();
    let mut last_heartbeat = started;

    let output = loop {
        if let Some(status) = child
            .try_wait()
            .context("polling container image load process")?
        {
            let stdout = stdout_task
                .await
                .context("joining container image load stdout capture")??;
            let stderr = stderr_task
                .await
                .context("joining container image load stderr capture")??;
            break std::process::Output {
                status,
                stdout,
                stderr,
            };
        }

        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            let status = child
                .wait()
                .await
                .context("waiting for timed out container image load")?;
            let stdout = stdout_task
                .await
                .context("joining timed out container image load stdout capture")??;
            let stderr = stderr_task
                .await
                .context("joining timed out container image load stderr capture")??;
            let output = std::process::Output {
                status,
                stdout,
                stderr,
            };
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                anyhow::bail!(
                    "container image load timed out after {}s",
                    SANDBOX_IMAGE_LOAD_TIMEOUT.as_secs()
                );
            }
            anyhow::bail!(
                "container image load timed out after {}s: {stderr}",
                SANDBOX_IMAGE_LOAD_TIMEOUT.as_secs()
            );
        }

        let now = tokio::time::Instant::now();
        if now.duration_since(last_heartbeat) >= image_load_heartbeat_interval() {
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "still loading harness image into local sandbox runtime ({} elapsed)",
                    format_image_load_elapsed(started.elapsed())
                ),
            );
            observe_progress(
                observer,
                HarnessSetupProgressUpdate {
                    phase: HarnessSetupPhase::ImageLoad,
                    active_download: None,
                },
            );
            last_heartbeat = now;
        }

        tokio::time::sleep(image_load_poll_interval()).await;
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("container image load failed (status: {})", output.status);
        }
        anyhow::bail!("container image load failed: {stderr}");
    }
    let load_message = command_output_message(&output);
    let visibility_deadline = tokio::time::Instant::now() + image_post_load_visibility_timeout();
    while tokio::time::Instant::now() < visibility_deadline {
        if container_image_present(data_root, image).await? {
            return Ok(());
        }
        tokio::time::sleep(image_load_poll_interval()).await;
    }
    if load_message.is_empty() {
        anyhow::bail!(
            "container image load reported success but image '{}' is still missing after {}s",
            image,
            image_post_load_visibility_timeout().as_secs()
        );
    }
    anyhow::bail!(
        "container image load reported success but image '{}' is still missing after {}s: {}",
        image,
        image_post_load_visibility_timeout().as_secs(),
        load_message
    );
}

#[derive(Debug, Clone)]
pub struct ContainerImageStatus {
    pub present: bool,
    pub available: bool,
    pub error: Option<String>,
}

pub async fn container_image_status(data_root: &Path, image: &str) -> Result<ContainerImageStatus> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let output = match sandbox_container_command(data_root) {
        Ok(mut cmd) => {
            cmd.arg("image").arg("inspect").arg(image);
            command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await
        }
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    let output = match output {
        Ok(out) => out,
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    if output.status.success() {
        return Ok(ContainerImageStatus {
            present: true,
            available: true,
            error: None,
        });
    }
    match output.status.code() {
        Some(1) => Ok(ContainerImageStatus {
            present: false,
            available: true,
            error: None,
        }),
        _ => Ok(ContainerImageStatus {
            present: false,
            available: false,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex as StdMutex;

    use tempfile::tempdir;

    use super::*;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prev.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::sandbox_cli_env_test_lock()
    }

    #[derive(Default)]
    struct RecordingObserver {
        logs: StdMutex<Vec<(HarnessSetupPhase, HarnessSetupLogLevel, String)>>,
        progress: StdMutex<Vec<HarnessSetupProgressUpdate>>,
    }

    impl HarnessSetupObserver for RecordingObserver {
        fn on_phase(&self, _phase: HarnessSetupPhase, _message: &str) {}

        fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str) {
            self.logs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((phase, level, message.to_string()));
        }

        fn on_progress(&self, progress: HarnessSetupProgressUpdate) {
            self.progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(progress);
        }
    }

    #[tokio::test]
    async fn load_container_image_emits_heartbeat_logs_and_progress_while_waiting() {
        let _serial = env_var_test_lock().lock().await;
        let temp = tempdir().expect("tempdir");
        let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
        let marker_path = temp.path().join("image-present");
        let tar_path = temp.path().join("ctx-harness.tar");
        std::fs::write(&tar_path, b"fake-image-tar").expect("write image tar");
        std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nset -eu\nmarker='{}'\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  sleep 0.25\n  : > \"$marker\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$marker\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nprintf 'unexpected sandbox CLI invocation: %s\\n' \"$*\" >&2\nexit 1\n",
                marker_path.display()
            ),
        )
        .expect("write sandbox CLI shim");
        std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox CLI shim");
        let _guard = EnvGuard::set(
            CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
            &sandbox_cli_path.to_string_lossy(),
        );
        let observer = RecordingObserver::default();

        load_container_image_tar(
            temp.path(),
            &tar_path,
            "ghcr.io/ctxrs/ctx-harness:test",
            Some(&observer),
        )
        .await
        .expect("image load should succeed");

        let logs = observer
            .logs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(logs.iter().any(|(phase, level, message)| {
            *phase == HarnessSetupPhase::ImageLoad
                && *level == HarnessSetupLogLevel::Info
                && message.contains("still loading harness image into local sandbox runtime")
        }));

        let progress = observer
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(progress.iter().any(|update| {
            update.phase == HarnessSetupPhase::ImageLoad && update.active_download.is_none()
        }));
    }

    #[tokio::test]
    async fn load_container_image_waits_for_post_load_visibility() {
        let _serial = env_var_test_lock().lock().await;
        let temp = tempdir().expect("tempdir");
        let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
        let marker_path = temp.path().join("image-present");
        let tar_path = temp.path().join("ctx-harness.tar");
        std::fs::write(&tar_path, b"fake-image-tar").expect("write image tar");
        std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nset -eu\nmarker='{}'\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  (sleep 0.1; : > \"$marker\") &\n  printf 'Loaded image: ghcr.io/ctxrs/ctx-harness:test\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$marker\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nprintf 'unexpected sandbox CLI invocation: %s\\n' \"$*\" >&2\nexit 1\n",
                marker_path.display()
            ),
        )
        .expect("write sandbox CLI shim");
        std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox CLI shim");
        let _guard = EnvGuard::set(
            CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
            &sandbox_cli_path.to_string_lossy(),
        );

        load_container_image_tar(
            temp.path(),
            &tar_path,
            "ghcr.io/ctxrs/ctx-harness:test",
            None,
        )
        .await
        .expect("image visibility should settle after load success");
    }
}
