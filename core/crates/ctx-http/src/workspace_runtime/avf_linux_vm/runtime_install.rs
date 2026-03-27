use super::*;

impl AvfLinuxGuestRuntime {
    pub(super) fn from_source(
        data_root: &Path,
        source: &bundled_assets::ManagedRuntimeSource,
    ) -> Result<Self> {
        let runtime_root = managed_avf_linux_runtime_root(data_root, source);
        let rootfs_image = runtime_root.join(source.bin.trim());
        let kernel_path = managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_KERNEL_HELPER)
            .ok_or_else(|| {
                anyhow::anyhow!("managed AVF Linux runtime is missing a kernel helper")
            })?;
        let initrd_path = managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_INITRD_HELPER)
            .ok_or_else(|| {
                anyhow::anyhow!("managed AVF Linux runtime is missing an initrd helper")
            })?;
        let guest_agent_path =
            managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_GUEST_AGENT_HELPER);
        let egress_proxy_path =
            managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_EGRESS_PROXY_HELPER);
        let container_stack_path =
            managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_CONTAINER_STACK_HELPER)
                .ok_or_else(|| {
                    anyhow::anyhow!("managed AVF Linux runtime is missing a guest container stack")
                })?;
        Ok(Self {
            runtime_root,
            rootfs_image,
            kernel_path,
            initrd_path,
            guest_agent_path,
            egress_proxy_path,
            container_stack_path,
            version: source.version.trim().to_string(),
            managed: true,
        })
    }

    fn from_runtime_root(runtime_root: PathBuf, version: String, managed: bool) -> Self {
        Self {
            kernel_path: runtime_root.join("helpers").join(AVF_LINUX_KERNEL_HELPER),
            initrd_path: runtime_root.join("helpers").join(AVF_LINUX_INITRD_HELPER),
            guest_agent_path: {
                let path = runtime_root
                    .join("helpers")
                    .join(AVF_LINUX_GUEST_AGENT_HELPER);
                path.exists().then_some(path)
            },
            egress_proxy_path: {
                let path = runtime_root
                    .join("helpers")
                    .join(AVF_LINUX_EGRESS_PROXY_HELPER);
                path.exists().then_some(path)
            },
            container_stack_path: runtime_root
                .join("helpers")
                .join(AVF_LINUX_CONTAINER_STACK_FILE),
            rootfs_image: runtime_root.join("rootfs.raw"),
            runtime_root,
            version,
            managed,
        }
    }

    fn from_bundled(paths: bundled_assets::BundledRuntimePaths) -> Self {
        let mut runtime = Self::from_runtime_root(paths.root, paths.version, false);
        runtime.rootfs_image = paths.bin;
        runtime
    }
}

pub(crate) fn runtime_target_label() -> String {
    if explicit_staged_avf_linux_guest_runtime_dir().is_some() {
        return format!("{AVF_LINUX_GUEST_RUNTIME_ID}:staged");
    }
    if let Some(runtime) = bundled_avf_linux_guest_runtime() {
        return format!("{AVF_LINUX_GUEST_RUNTIME_ID}:bundled:{}", runtime.version);
    }
    managed_avf_linux_guest_source()
        .map(|source| format!("{AVF_LINUX_GUEST_RUNTIME_ID}:{}", source.version.trim()))
        .unwrap_or_else(|| AVF_LINUX_GUEST_RUNTIME_ID.to_string())
}

fn explicit_staged_avf_linux_guest_runtime_dir() -> Option<PathBuf> {
    let raw = std::env::var(AVF_LINUX_GUEST_RUNTIME_DIR_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn staged_runtime_version(runtime_root: &Path) -> String {
    let version_path = runtime_root.join("version.txt");
    std::fs::read_to_string(&version_path)
        .ok()
        .and_then(|contents| {
            let lines = contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            if lines.is_empty() {
                return None;
            }
            let version = lines
                .iter()
                .find_map(|line| line.strip_prefix("version=").map(str::trim))
                .filter(|line| !line.is_empty())
                .unwrap_or(lines[0]);
            Some(version.to_string())
        })
        .unwrap_or_else(|| "staged".to_string())
}

pub(super) fn staged_avf_linux_guest_runtime() -> Result<Option<AvfLinuxGuestRuntime>> {
    let Some(runtime_root) = explicit_staged_avf_linux_guest_runtime_dir() else {
        return Ok(None);
    };
    if !runtime_root.exists() {
        bail!(
            "explicit staged AVF Linux guest runtime dir does not exist: {}",
            runtime_root.display()
        );
    }
    let version = staged_runtime_version(&runtime_root);
    let runtime = AvfLinuxGuestRuntime::from_runtime_root(runtime_root, version, false);
    if avf_linux_runtime_is_ready(&runtime) {
        return Ok(Some(runtime));
    }
    bail!(
        "explicit staged AVF Linux guest runtime dir is incomplete or not ready: {}",
        runtime.runtime_root.display()
    );
}

pub(super) fn managed_avf_linux_guest_source() -> Option<bundled_assets::ManagedRuntimeSource> {
    #[cfg(test)]
    if let Some(source) = test_runtime_source_override()
        .lock()
        .expect("AVF Linux runtime override mutex poisoned")
        .clone()
    {
        return Some(source);
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    bundled_assets::managed_runtime_source(AVF_LINUX_GUEST_RUNTIME_ID, os, arch)
}

fn managed_artifact_extension(uri: &str) -> &'static str {
    let path = url::Url::parse(uri)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| uri.to_string());
    let path_lc = path.to_ascii_lowercase();
    if path_lc.ends_with(".tar.gz") {
        "tar.gz"
    } else if path_lc.ends_with(".zst") {
        "zst"
    } else if path_lc.ends_with(".tgz") {
        "tgz"
    } else if path_lc.ends_with(".tar") {
        "tar"
    } else {
        "zip"
    }
}

pub(super) fn managed_avf_linux_archive_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let ext = managed_artifact_extension(&source.uri);
    data_root
        .join("managed")
        .join("downloads")
        .join(AVF_LINUX_GUEST_RUNTIME_ID)
        .join(os)
        .join(arch)
        .join(format!(
            "sha256-{}.{}",
            source.sha256.trim().to_ascii_lowercase(),
            ext
        ))
}

pub(super) fn managed_avf_linux_runtime_root(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join(AVF_LINUX_GUEST_RUNTIME_ID)
        .join(os)
        .join(arch)
        .join(format!(
            "{AVF_LINUX_GUEST_RUNTIME_ID}-{}",
            source.version.trim()
        ))
}

pub(super) fn managed_avf_linux_helper_path(
    runtime_root: &Path,
    helper_name: &str,
) -> Option<PathBuf> {
    let helper = helper_name.trim();
    if helper.is_empty() {
        return None;
    }
    let file_name = if helper == AVF_LINUX_CONTAINER_STACK_HELPER {
        AVF_LINUX_CONTAINER_STACK_FILE
    } else {
        helper
    };
    Some(runtime_root.join("helpers").join(file_name))
}

pub(super) fn managed_avf_linux_runtime_ready_marker_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join(AVF_LINUX_RUNTIME_READY_MARKER)
}

pub(super) fn avf_linux_runtime_is_ready(runtime: &AvfLinuxGuestRuntime) -> bool {
    let guest_agent_ready = runtime
        .guest_agent_path
        .as_ref()
        .is_some_and(|path| path.exists());
    let egress_proxy_ready = runtime
        .egress_proxy_path
        .as_ref()
        .is_some_and(|path| path.exists());
    let container_stack_ready = runtime.container_stack_path.exists();
    runtime.rootfs_image.exists()
        && runtime.kernel_path.exists()
        && runtime.initrd_path.exists()
        && guest_agent_ready
        && egress_proxy_ready
        && container_stack_ready
        && (!runtime.managed
            || managed_avf_linux_runtime_ready_marker_path(&runtime.runtime_root).exists())
}

fn managed_avf_linux_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn mark_managed_avf_linux_runtime_ready(runtime_root: &Path) -> Result<()> {
    let marker = managed_avf_linux_runtime_ready_marker_path(runtime_root);
    fs::write(&marker, b"ready")
        .await
        .with_context(|| format!("writing {}", marker.display()))
}

pub(super) fn bundled_avf_linux_guest_runtime() -> Option<AvfLinuxGuestRuntime> {
    let runtime = bundled_assets::bundled_avf_linux_guest_runtime()?;
    let runtime = AvfLinuxGuestRuntime::from_bundled(runtime);
    if avf_linux_runtime_is_ready(&runtime) {
        Some(runtime)
    } else {
        None
    }
}

pub(crate) async fn ensure_managed_avf_linux_guest_runtime(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<AvfLinuxGuestRuntime> {
    ensure_managed_avf_linux_guest_runtime_with_override(
        data_root,
        None,
        observer,
        download_aggregate,
    )
    .await
}

pub(crate) async fn ensure_managed_avf_linux_guest_runtime_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedRuntimeSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<AvfLinuxGuestRuntime> {
    if source_override.is_none() {
        if let Some(runtime) = staged_avf_linux_guest_runtime()? {
            return Ok(runtime);
        }
        if let Some(runtime) = bundled_avf_linux_guest_runtime() {
            return Ok(runtime);
        }
    }

    let source = match source_override.cloned() {
        Some(source) => source,
        None => managed_avf_linux_guest_source().ok_or_else(|| {
            anyhow::anyhow!(
                "managed AVF Linux guest runtime source is not available for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?,
    };
    let runtime = AvfLinuxGuestRuntime::from_source(data_root, &source)?;
    if avf_linux_runtime_is_ready(&runtime) {
        return Ok(runtime);
    }

    let _install_guard = managed_avf_linux_install_lock().lock().await;
    let runtime = AvfLinuxGuestRuntime::from_source(data_root, &source)?;
    if avf_linux_runtime_is_ready(&runtime) {
        return Ok(runtime);
    }

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "installing managed AVF Linux guest runtime {}",
            runtime.version
        ),
    );

    let final_archive = managed_avf_linux_archive_path(data_root, &source);
    let archive_lock = managed_artifact_lock_path(&final_archive);
    let _archive_guard = acquire_managed_artifact_file_lock(
        &archive_lock,
        "AVF Linux guest runtime archive",
        observer,
        HarnessSetupPhase::ArtifactDownload,
    )
    .await?;
    let partial_archive = managed_artifact_partial_path(&final_archive);
    if final_archive.exists() {
        let digest = updates::sha256_hex_file(&final_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", final_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&final_archive).await;
        } else {
            let _ = fs::remove_file(&partial_archive).await;
        }
    }
    if !final_archive.exists() {
        let Some(parent) = final_archive.parent() else {
            bail!(
                "managed AVF Linux archive path has no parent: {}",
                final_archive.display()
            );
        };
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
        download_managed_artifact(
            &source.uri,
            &partial_archive,
            Some(ManagedArtifactDownloadReporter::new(
                observer,
                download_aggregate.clone(),
                HarnessSetupPhase::ArtifactDownload,
                AVF_LINUX_ROOTFS_LABEL,
            )),
        )
        .await?;
        finalize_managed_artifact_download(
            &partial_archive,
            &final_archive,
            &source.sha256,
            "managed AVF Linux guest runtime archive",
        )
        .await?;
    }

    let Some(parent) = runtime.runtime_root.parent() else {
        bail!(
            "managed AVF Linux runtime root has no parent: {}",
            runtime.runtime_root.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let staging_dir = parent.join(format!(
        ".avf-linux-staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir).await;
    }
    fs::create_dir_all(&staging_dir)
        .await
        .with_context(|| format!("creating {}", staging_dir.display()))?;
    let extract_dir = staging_dir.join("extract");
    fs::create_dir_all(&extract_dir)
        .await
        .with_context(|| format!("creating {}", extract_dir.display()))?;
    let archive_for_extract = final_archive.clone();
    let uri_for_extract = source.uri.clone();
    let extract_dir_for_extract = extract_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_archive_to_dir(
            &archive_for_extract,
            &uri_for_extract,
            &extract_dir_for_extract,
        )
    })
    .await
    .context("joining managed AVF Linux extract task")??;
    let extracted_root = tokio::task::spawn_blocking({
        let extract_dir = extract_dir.clone();
        move || resolve_single_extracted_root(&extract_dir)
    })
    .await
    .context("joining managed AVF Linux extraction root task")??;

    if runtime.runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime.runtime_root).await;
    }
    fs::rename(&extracted_root, &runtime.runtime_root)
        .await
        .with_context(|| {
            format!(
                "moving extracted AVF Linux runtime into place: {} -> {}",
                extracted_root.display(),
                runtime.runtime_root.display()
            )
        })?;
    let _ = fs::remove_dir_all(&staging_dir).await;

    let mut helper_downloads = Vec::new();
    for (helper_name, label) in [
        (AVF_LINUX_KERNEL_HELPER, "Linux kernel"),
        (AVF_LINUX_INITRD_HELPER, "Linux initrd"),
        (AVF_LINUX_GUEST_AGENT_HELPER, "Guest agent"),
        (AVF_LINUX_EGRESS_PROXY_HELPER, "Egress proxy"),
        (AVF_LINUX_CONTAINER_STACK_HELPER, "Guest container stack"),
    ] {
        let helper_source = source.helpers.get(helper_name).cloned().ok_or_else(|| {
            anyhow::anyhow!("managed AVF Linux runtime is missing helper '{helper_name}'")
        })?;
        let helper_root = runtime.runtime_root.clone();
        let aggregate = download_aggregate.clone();
        helper_downloads.push(async move {
            let helper_path =
                managed_avf_linux_helper_path(&helper_root, helper_name).ok_or_else(|| {
                    anyhow::anyhow!("invalid AVF Linux helper path for '{helper_name}'")
                })?;
            let helper_lock = managed_artifact_lock_path(&helper_path);
            let _guard = acquire_managed_artifact_file_lock(
                &helper_lock,
                label,
                observer,
                HarnessSetupPhase::ArtifactDownload,
            )
            .await?;
            if let Some(parent) = helper_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            if helper_path.exists() {
                let digest = updates::sha256_hex_file(&helper_path)
                    .await
                    .with_context(|| format!("computing sha256 for {}", helper_path.display()))?;
                if digest.eq_ignore_ascii_case(helper_source.sha256.trim()) {
                    let _ = fs::remove_file(managed_artifact_partial_path(&helper_path)).await;
                    return Ok(()) as Result<()>;
                }
                let _ = fs::remove_file(&helper_path).await;
            }
            let tmp = managed_artifact_partial_path(&helper_path);
            download_managed_artifact(
                &helper_source.uri,
                &tmp,
                Some(ManagedArtifactDownloadReporter::new(
                    observer,
                    aggregate,
                    HarnessSetupPhase::ArtifactDownload,
                    label,
                )),
            )
            .await?;
            finalize_managed_artifact_download(&tmp, &helper_path, &helper_source.sha256, label)
                .await?;
            Ok(())
        });
    }
    for result in futures::future::join_all(helper_downloads).await {
        result?;
    }

    if !runtime.rootfs_image.exists() {
        bail!(
            "managed AVF Linux runtime installed but rootfs image is missing at {}",
            runtime.rootfs_image.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for path in [&runtime.kernel_path, &runtime.initrd_path] {
            if path.exists() {
                let mut perms = fs::metadata(path)
                    .await
                    .with_context(|| format!("metadata {}", path.display()))?
                    .permissions();
                perms.set_mode(0o644);
                fs::set_permissions(path, perms)
                    .await
                    .with_context(|| format!("chmod {}", path.display()))?;
            }
        }
        for path in [
            runtime.guest_agent_path.as_ref(),
            runtime.egress_proxy_path.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if path.exists() {
                let mut perms = fs::metadata(path)
                    .await
                    .with_context(|| format!("metadata {}", path.display()))?
                    .permissions();
                perms.set_mode(0o755);
                fs::set_permissions(path, perms)
                    .await
                    .with_context(|| format!("chmod {}", path.display()))?;
            }
        }
        if runtime.container_stack_path.exists() {
            let mut perms = fs::metadata(&runtime.container_stack_path)
                .await
                .with_context(|| format!("metadata {}", runtime.container_stack_path.display()))?
                .permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&runtime.container_stack_path, perms)
                .await
                .with_context(|| format!("chmod {}", runtime.container_stack_path.display()))?;
        }
    }
    mark_managed_avf_linux_runtime_ready(&runtime.runtime_root).await?;
    Ok(runtime)
}

#[cfg(test)]
fn test_runtime_source_override() -> &'static StdMutex<Option<bundled_assets::ManagedRuntimeSource>>
{
    static OVERRIDE: OnceLock<StdMutex<Option<bundled_assets::ManagedRuntimeSource>>> =
        OnceLock::new();
    OVERRIDE.get_or_init(|| StdMutex::new(None))
}

#[cfg(test)]
pub(crate) struct TestManagedAvfLinuxRuntimeSourceGuard {
    previous: Option<bundled_assets::ManagedRuntimeSource>,
}

#[cfg(test)]
impl Drop for TestManagedAvfLinuxRuntimeSourceGuard {
    fn drop(&mut self) {
        let mut guard = test_runtime_source_override()
            .lock()
            .expect("AVF Linux runtime override mutex poisoned");
        *guard = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) fn override_managed_avf_linux_runtime_source_for_test(
    source: bundled_assets::ManagedRuntimeSource,
) -> TestManagedAvfLinuxRuntimeSourceGuard {
    let mut guard = test_runtime_source_override()
        .lock()
        .expect("AVF Linux runtime override mutex poisoned");
    let previous = guard.replace(source);
    TestManagedAvfLinuxRuntimeSourceGuard { previous }
}

#[cfg(test)]
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

#[cfg(test)]
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

#[cfg(test)]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(test)]
fn write_staged_runtime(runtime_root: &Path, version: &str) {
    std::fs::create_dir_all(runtime_root.join("helpers")).expect("create staged runtime helpers");
    std::fs::write(runtime_root.join("rootfs.raw"), b"rootfs").expect("write staged rootfs");
    std::fs::write(runtime_root.join("version.txt"), version).expect("write staged version");
    for helper in [
        AVF_LINUX_KERNEL_HELPER,
        AVF_LINUX_INITRD_HELPER,
        AVF_LINUX_GUEST_AGENT_HELPER,
        AVF_LINUX_EGRESS_PROXY_HELPER,
        AVF_LINUX_CONTAINER_STACK_HELPER,
    ] {
        let helper_path =
            managed_avf_linux_helper_path(runtime_root, helper).expect("staged helper path");
        std::fs::write(helper_path, helper.as_bytes())
            .unwrap_or_else(|err| panic!("write staged helper {helper}: {err}"));
    }
}

#[cfg(test)]
#[tokio::test]
async fn ensure_managed_avf_linux_guest_runtime_prefers_explicit_staged_dir() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    write_staged_runtime(&runtime_root, "staged-override");
    let _runtime_dir = EnvGuard::set(
        AVF_LINUX_GUEST_RUNTIME_DIR_ENV,
        runtime_root.to_str().expect("runtime root utf8"),
    );
    let _source =
        override_managed_avf_linux_runtime_source_for_test(bundled_assets::ManagedRuntimeSource {
            uri: "locked://runtimes/avf-linux-guest/macos/aarch64/rootfs.raw.zst".to_string(),
            sha256: "0".repeat(64),
            version: "lock-version".to_string(),
            bin: "rootfs.raw".to_string(),
            helpers: HashMap::from([
                (
                    AVF_LINUX_KERNEL_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: "locked://kernel".to_string(),
                        sha256: "1".repeat(64),
                    },
                ),
                (
                    AVF_LINUX_INITRD_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: "locked://initrd".to_string(),
                        sha256: "2".repeat(64),
                    },
                ),
                (
                    AVF_LINUX_GUEST_AGENT_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: "locked://guest-agent".to_string(),
                        sha256: "3".repeat(64),
                    },
                ),
                (
                    AVF_LINUX_EGRESS_PROXY_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: "locked://egress-proxy".to_string(),
                        sha256: "4".repeat(64),
                    },
                ),
                (
                    AVF_LINUX_CONTAINER_STACK_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: "locked://container-stack".to_string(),
                        sha256: "5".repeat(64),
                    },
                ),
            ]),
        });

    let runtime = ensure_managed_avf_linux_guest_runtime(temp.path(), None, None)
        .await
        .expect("staged runtime should be used before lock download");

    assert_eq!(runtime.runtime_root, runtime_root);
    assert_eq!(runtime.version, "staged-override");
    assert!(!runtime.managed);
}

#[cfg(test)]
#[tokio::test]
async fn explicit_staged_avf_linux_guest_runtime_dir_must_be_ready() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    std::fs::create_dir_all(runtime_root.join("helpers")).expect("create staged helpers");
    std::fs::write(runtime_root.join("rootfs.raw"), b"rootfs").expect("write staged rootfs");
    let _runtime_dir = EnvGuard::set(
        AVF_LINUX_GUEST_RUNTIME_DIR_ENV,
        runtime_root.to_str().expect("runtime root utf8"),
    );

    let err = ensure_managed_avf_linux_guest_runtime(temp.path(), None, None)
        .await
        .expect_err("incomplete staged runtime should fail closed");

    assert!(
        err.to_string()
            .contains("explicit staged AVF Linux guest runtime dir is incomplete or not ready"),
        "unexpected error: {err:#}"
    );
}

#[cfg(test)]
#[test]
fn managed_avf_linux_archive_path_preserves_zstd_extension() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = bundled_assets::ManagedRuntimeSource {
        uri: "https://example.test/runtimes/avf-linux-guest/rootfs.raw.zst".to_string(),
        sha256: "a".repeat(64),
        version: "ubuntu-noble-arm64-test".to_string(),
        bin: "rootfs.raw".to_string(),
        helpers: HashMap::new(),
    };

    let archive_path = managed_avf_linux_archive_path(temp.path(), &source);
    assert_eq!(
        archive_path
            .extension()
            .and_then(|ext| ext.to_str())
            .expect("zst extension"),
        "zst"
    );
}
