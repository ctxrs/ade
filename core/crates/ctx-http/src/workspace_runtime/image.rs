use super::*;

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
    observe_phase(
        observer,
        HarnessSetupPhase::MachineCheck,
        "checking container runtime",
    );
    super::machine::ensure_managed_podman_runtime_with_override(
        data_root,
        overrides.and_then(|value| value.podman_runtime_source.as_ref()),
        observer,
    )
    .await?;
    if image == DEFAULT_CONTAINER_IMAGE && bundled_default_container_image_tar().is_none() {
        observe_phase(
            observer,
            HarnessSetupPhase::ImageCheck,
            "checking harness image artifact availability",
        );
        let _ = ensure_managed_default_container_image_tar_with_override(
            data_root,
            overrides.and_then(|value| value.default_image_source.as_ref()),
            observer,
        )
        .await?;
    }
    Ok(())
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
    ensure_podman_machine_running_with_observer(data_root, observer).await?;
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
        "loading harness image into podman",
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
    let mut cmd = podman_command(data_root)?;
    cmd.arg("image").arg("exists").arg("--").arg(image);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    match output.status.code() {
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "podman image exists failed: {}",
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
        load_container_image_tar(data_root, &image_tar, image).await?;
        return Ok(());
    }

    anyhow::bail!(
        "container image '{}' is not present; registry pulls are disabled, so the image must already exist in podman",
        image
    );
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
    ensure_managed_default_container_image_tar_with_override(data_root, None, observer).await
}

pub(super) async fn ensure_managed_default_container_image_tar_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedArtifactSource>,
    observer: Option<&dyn HarnessSetupObserver>,
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
    ensure_managed_default_container_image_tar_with_source(data_root, &source, observer).await
}

pub(super) async fn ensure_managed_default_container_image_tar_with_source(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
    observer: Option<&dyn HarnessSetupObserver>,
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
        HarnessSetupPhase::ImageLoad,
        HarnessSetupLogLevel::Info,
        &format!("downloading default harness image from {}", source.uri),
    );
    download_managed_artifact(&source.uri, &tmp_tar).await?;

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

async fn load_container_image_tar(data_root: &Path, tar: &Path, image: &str) -> Result<()> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("load").arg("-i").arg(tar);
    let output = command_output_with_timeout(cmd, PODMAN_LOAD_TIMEOUT)
        .await
        .with_context(|| format!("podman load failed for {}", tar.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("podman load failed (status: {})", output.status);
        }
        anyhow::bail!("podman load failed: {stderr}");
    }
    if container_image_present(data_root, image).await? {
        return Ok(());
    }
    anyhow::bail!(
        "podman load reported success but image '{}' is still missing",
        image
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
    let output = match podman_command(data_root) {
        Ok(mut cmd) => {
            cmd.arg("image").arg("exists").arg("--").arg(image);
            command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await
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
