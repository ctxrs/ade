use super::archive::managed_sandbox_cli_archive_path;
use super::*;

#[cfg(test)]
fn managed_sandbox_machine_cache_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(super) async fn ensure_managed_sandbox_machine_cache(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let source = bundled_assets::managed_sandbox_machine_cache_source().ok_or_else(|| {
        anyhow::anyhow!(
            "managed sandbox machine cache source is not available for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let final_path = managed_sandbox_machine_cache_path(data_root, &source);
    let partial_path = managed_artifact_partial_path(&final_path);
    let _install_guard = managed_sandbox_machine_cache_install_lock().lock().await;
    let _cross_process_guard = acquire_managed_artifact_file_lock(
        &managed_artifact_lock_path(&final_path),
        "managed sandbox machine cache",
        observer,
        HarnessSetupPhase::ArtifactDownload,
    )
    .await?;
    if final_path.exists() {
        let digest = updates::sha256_hex_file(&final_path)
            .await
            .with_context(|| format!("computing sha256 for {}", final_path.display()))?;
        if digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&partial_path).await;
            return Ok(final_path);
        }
        observe_log(
            observer,
            HarnessSetupPhase::ArtifactDownload,
            HarnessSetupLogLevel::Warn,
            &format!(
                "managed sandbox machine cache checksum mismatch for {}; re-downloading",
                final_path.display()
            ),
        );
        let _ = fs::remove_file(&final_path).await;
    }

    let Some(parent) = final_path.parent() else {
        anyhow::bail!(
            "managed sandbox machine cache path has no parent: {}",
            final_path.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "downloading managed sandbox machine cache into {}",
            final_path.display()
        ),
    );
    download_managed_artifact(
        &source.uri,
        &partial_path,
        Some(ManagedArtifactDownloadReporter::new(
            observer,
            download_aggregate,
            HarnessSetupPhase::ArtifactDownload,
            "Sandbox machine cache",
        )),
    )
    .await?;
    finalize_managed_artifact_download(
        &partial_path,
        &final_path,
        &source.sha256,
        "managed sandbox machine cache",
    )
    .await?;
    Ok(final_path)
}

#[cfg(test)]
pub(super) fn managed_sandbox_cli_runtime_source() -> Option<bundled_assets::ManagedRuntimeSource> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    bundled_assets::managed_runtime_source("sandbox-cli", os, arch)
}

#[cfg(test)]
pub(super) fn managed_sandbox_cli_runtime_bin_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join(os)
        .join(arch)
        .join(format!("sandbox-cli-{}", source.version.trim()))
        .join(source.bin.trim())
}

#[cfg(test)]
fn managed_sandbox_cli_runtime_root(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join(os)
        .join(arch)
        .join(format!("sandbox-cli-{}", source.version.trim()))
}

#[cfg(test)]
fn managed_sandbox_cli_helper_path(runtime_root: &Path, helper_name: &str) -> Option<PathBuf> {
    let helper = helper_name.trim();
    if helper.is_empty() {
        return None;
    }
    Some(
        runtime_root
            .join("usr")
            .join("libexec")
            .join("sandbox-cli")
            .join(helper),
    )
}

#[cfg(test)]
fn managed_sandbox_cli_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn managed_sandbox_cli_runtime_ready_marker_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join(".ctx-managed-ready")
}

#[cfg(test)]
fn managed_sandbox_cli_runtime_is_ready(
    runtime_root: &Path,
    runtime_bin: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> bool {
    if !runtime_bin.exists()
        || !managed_sandbox_cli_runtime_ready_marker_path(runtime_root).exists()
    {
        return false;
    }
    source.helpers.keys().all(|name| {
        managed_sandbox_cli_helper_path(runtime_root, name)
            .map(|path| path.exists())
            .unwrap_or(true)
    })
}

#[cfg(test)]
async fn mark_managed_sandbox_cli_runtime_ready(runtime_root: &Path) -> Result<()> {
    let marker = managed_sandbox_cli_runtime_ready_marker_path(runtime_root);
    fs::write(&marker, b"ready")
        .await
        .with_context(|| format!("writing {}", marker.display()))
}

#[cfg(test)]
pub(super) async fn ensure_managed_sandbox_cli_runtime(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    ensure_managed_sandbox_cli_runtime_with_override(data_root, None, observer, download_aggregate)
        .await
}

#[cfg(test)]
pub(super) async fn ensure_managed_sandbox_cli_runtime_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedRuntimeSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    if source_override.is_none() {
        if let Ok(raw) = std::env::var(CTX_HARNESS_SANDBOX_CLI_PATH_ENV) {
            let path = PathBuf::from(raw.trim());
            if path.exists() {
                return Ok(path);
            }
        }
        if let Some(bundled) = bundled_assets::bundled_sandbox_cli_runtime() {
            return Ok(bundled.bin);
        }
    }
    let source = match source_override.cloned() {
        Some(source) => source,
        None => managed_sandbox_cli_runtime_source().ok_or_else(|| {
            anyhow::anyhow!(
                "managed sandbox CLI runtime source is not available for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?,
    };
    let runtime_root = managed_sandbox_cli_runtime_root(data_root, &source);
    let runtime_bin = managed_sandbox_cli_runtime_bin_path(data_root, &source);
    if managed_sandbox_cli_runtime_is_ready(&runtime_root, &runtime_bin, &source) {
        return Ok(runtime_bin);
    }
    let _install_guard = managed_sandbox_cli_install_lock().lock().await;
    if managed_sandbox_cli_runtime_is_ready(&runtime_root, &runtime_bin, &source) {
        return Ok(runtime_bin);
    }

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!("installing managed sandbox CLI runtime {}", source.version),
    );

    let final_archive = managed_sandbox_cli_archive_path(data_root, &source);
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
            anyhow::bail!(
                "managed sandbox CLI archive path has no parent: {}",
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
                "Sandbox CLI runtime",
            )),
        )
        .await?;
        let digest = updates::sha256_hex_file(&partial_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", partial_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&partial_archive).await;
            anyhow::bail!(
                "managed sandbox CLI runtime checksum mismatch: expected {}, got {}",
                source.sha256.trim(),
                digest
            );
        }
        finalize_managed_artifact_download(
            &partial_archive,
            &final_archive,
            &source.sha256,
            "managed sandbox CLI runtime archive",
        )
        .await?;
    }

    let Some(parent) = runtime_root.parent() else {
        anyhow::bail!(
            "managed runtime root has no parent: {}",
            runtime_root.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let staging_dir = parent.join(format!(
        ".sandbox-cli-staging-{}",
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
    .context("joining managed sandbox CLI extract task")??;
    let extracted_root = tokio::task::spawn_blocking({
        let extract_dir = extract_dir.clone();
        move || resolve_single_extracted_root(&extract_dir)
    })
    .await
    .context("joining managed sandbox CLI extraction root task")??;

    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root).await;
    }
    fs::rename(&extracted_root, &runtime_root)
        .await
        .with_context(|| {
            format!(
                "moving extracted sandbox CLI runtime into place: {} -> {}",
                extracted_root.display(),
                runtime_root.display()
            )
        })?;
    let _ = fs::remove_dir_all(&staging_dir).await;

    let mut helper_downloads = Vec::new();
    for (name, helper) in &source.helpers {
        let Some(path) = managed_sandbox_cli_helper_path(&runtime_root, name) else {
            continue;
        };
        let helper_name = name.to_string();
        let helper_source = helper.clone();
        let helper_path = path.clone();
        let aggregate = download_aggregate.clone();
        helper_downloads.push(async move {
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
                    format!("Sandbox helper ({helper_name})"),
                )),
            )
            .await?;
            let digest = updates::sha256_hex_file(&tmp)
                .await
                .with_context(|| format!("computing sha256 for {}", tmp.display()))?;
            if !digest.eq_ignore_ascii_case(helper_source.sha256.trim()) {
                let _ = fs::remove_file(&tmp).await;
                anyhow::bail!(
                    "managed sandbox helper checksum mismatch ({}): expected {}, got {}",
                    helper_name,
                    helper_source.sha256.trim(),
                    digest
                );
            }
            fs::rename(&tmp, &helper_path).await.with_context(|| {
                format!(
                    "moving managed sandbox helper into place: {} -> {}",
                    tmp.display(),
                    helper_path.display()
                )
            })?;
            Ok(())
        });
    }
    for result in futures::future::join_all(helper_downloads).await {
        result?;
    }

    if !runtime_bin.exists() {
        anyhow::bail!(
            "managed sandbox CLI runtime installed but binary is missing at {}",
            runtime_bin.display()
        );
    }
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&runtime_bin)
            .await
            .with_context(|| format!("metadata {}", runtime_bin.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&runtime_bin, perms)
            .await
            .with_context(|| format!("chmod {}", runtime_bin.display()))?;
        for name in source.helpers.keys() {
            if let Some(helper_path) = managed_sandbox_cli_helper_path(&runtime_root, name) {
                if helper_path.exists() {
                    let mut helper_perms = fs::metadata(&helper_path)
                        .await
                        .with_context(|| format!("metadata {}", helper_path.display()))?
                        .permissions();
                    helper_perms.set_mode(0o755);
                    fs::set_permissions(&helper_path, helper_perms)
                        .await
                        .with_context(|| format!("chmod {}", helper_path.display()))?;
                }
            }
        }
    }
    mark_managed_sandbox_cli_runtime_ready(&runtime_root).await?;
    Ok(runtime_bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn partial_managed_sandbox_cli_runtime_triggers_repair_instead_of_reuse() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = bundled_assets::ManagedRuntimeSource {
            uri: "http://127.0.0.1:9/sandbox-cli-runtime.tar.gz".to_string(),
            sha256: "deadbeef".to_string(),
            version: "test-version".to_string(),
            bin: "bin/sandbox-cli".to_string(),
            helpers: HashMap::from([(
                "gvproxy".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: "http://127.0.0.1:9/gvproxy".to_string(),
                    sha256: "deadbeef".to_string(),
                },
            )]),
        };

        let runtime_root = managed_sandbox_cli_runtime_root(temp.path(), &source);
        let runtime_bin = managed_sandbox_cli_runtime_bin_path(temp.path(), &source);
        fs::create_dir_all(runtime_bin.parent().expect("runtime bin parent"))
            .await
            .expect("create runtime bin parent");
        fs::write(&runtime_bin, b"partial-sandbox-cli")
            .await
            .expect("write partial runtime binary");

        let err = ensure_managed_sandbox_cli_runtime_with_override(
            temp.path(),
            Some(&source),
            None,
            None,
        )
        .await
        .expect_err("partial runtime should trigger repair attempt");

        assert!(
            !managed_sandbox_cli_runtime_is_ready(&runtime_root, &runtime_bin, &source),
            "missing ready marker/helper payload must not count as a ready runtime"
        );
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("downloading managed artifact")
                || rendered.contains("managed artifact download http error"),
            "repair should attempt managed runtime download, got: {rendered}"
        );
    }
}
