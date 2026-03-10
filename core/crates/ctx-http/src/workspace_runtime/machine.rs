use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;

const PODMAN_MACHINE_CACHE_ID: &str = "podman-machine";

pub(super) fn ctx_podman_machine_name(data_root: &Path) -> String {
    let hash = podman_data_root_hash(data_root);
    format!("{CTX_PODMAN_MACHINE_PREFIX}-{hash}")
}

fn podman_data_root_hash(data_root: &Path) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(data_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub(super) fn podman_runtime_root(data_root: &Path) -> PathBuf {
    let hash = podman_data_root_hash(data_root);
    #[cfg(unix)]
    {
        PathBuf::from("/tmp").join("ctxp").join(hash)
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("ctxp").join(hash)
    }
}

pub(super) fn podman_home_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("home")
}

pub(super) fn podman_temp_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("tmp")
}

#[cfg(test)]
pub(super) fn podman_machine_cache_root(data_root: &Path) -> PathBuf {
    data_root
        .join("podman")
        .join("xdg")
        .join("data")
        .join("containers")
        .join("podman")
        .join("machine")
}

fn shared_podman_machine_cache_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_MACHINE_CACHE_DIR_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    directories::BaseDirs::new().map(|base| {
        base.cache_dir()
            .join("ctx")
            .join("podman-machine")
            .join(std::env::consts::OS)
            .join(std::env::consts::ARCH)
    })
}

fn managed_podman_machine_cache_root(data_root: &Path) -> PathBuf {
    shared_podman_machine_cache_root()
        .unwrap_or_else(|| data_root.join("managed").join("machine-cache"))
}

fn managed_artifact_file_name(url: &str, sha256: &str, fallback_prefix: &str) -> String {
    let basename = Url::parse(url)
        .ok()
        .and_then(|parsed| {
            Path::new(parsed.path())
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{fallback_prefix}-{sha256}.bin"));
    format!("sha256-{}-{}", sha256.trim().to_ascii_lowercase(), basename)
}

fn managed_podman_machine_cache_path(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
) -> PathBuf {
    managed_podman_machine_cache_root(data_root)
        .join("managed")
        .join(managed_artifact_file_name(
            &source.uri,
            &source.sha256,
            PODMAN_MACHINE_CACHE_ID,
        ))
}

fn collect_podman_machine_cache_file_relpaths(root: &Path) -> Result<Vec<PathBuf>> {
    fn collect_recursive(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("read_dir entry {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("stat {}", path.display()))?;
            if file_type.is_dir() {
                collect_recursive(root, &path, out)?;
                continue;
            }
            if file_type.is_file() || file_type.is_symlink() {
                let relpath = path.strip_prefix(root).with_context(|| {
                    format!("computing relative cache path for {}", path.display())
                })?;
                out.push(relpath.to_path_buf());
            }
        }
        Ok(())
    }

    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut relpaths = Vec::new();
    for provider_dir in
        std::fs::read_dir(root).with_context(|| format!("read_dir {}", root.display()))?
    {
        let provider_dir =
            provider_dir.with_context(|| format!("read_dir entry {}", root.display()))?;
        let provider_path = provider_dir.path();
        if !provider_path.is_dir() {
            continue;
        }
        let cache_dir = provider_path.join("cache");
        if !cache_dir.is_dir() {
            continue;
        }
        collect_recursive(root, &cache_dir, &mut relpaths)?;
    }
    relpaths.sort();
    Ok(relpaths)
}

#[cfg(unix)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dest)
}

#[cfg(not(any(unix, windows)))]
fn symlink_cache_file(_src: &Path, _dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file symlinks are not supported on this platform",
    ))
}

fn podman_machine_cache_tmp_path(dest: &Path) -> PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("cache-file");
    dest.with_file_name(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn materialize_podman_machine_cache_file(
    src: &Path,
    dest: &Path,
    allow_symlink: bool,
) -> Result<()> {
    if src == dest || dest.exists() {
        return Ok(());
    }
    let Some(parent) = dest.parent() else {
        anyhow::bail!(
            "podman machine cache target has no parent: {}",
            dest.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    match std::fs::hard_link(src, dest) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => {}
    }
    if allow_symlink {
        match symlink_cache_file(src, dest) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(_) => {}
        }
    }

    let tmp = podman_machine_cache_tmp_path(dest);
    let _ = fs::remove_file(&tmp).await;
    fs::copy(src, &tmp)
        .await
        .with_context(|| format!("copying {} -> {}", src.display(), tmp.display()))?;
    match fs::rename(&tmp, dest).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp).await;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp).await;
            Err(err).with_context(|| format!("moving {} -> {}", tmp.display(), dest.display()))
        }
    }
}

#[cfg(test)]
pub(super) async fn seed_shared_podman_machine_cache(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_podman_machine_cache_root() else {
        return Ok(());
    };
    let relpaths = collect_podman_machine_cache_file_relpaths(&shared_root)?;
    if relpaths.is_empty() {
        return Ok(());
    }
    let local_root = podman_machine_cache_root(data_root);
    let mut seeded = 0usize;
    for relpath in relpaths {
        let src = shared_root.join(&relpath);
        let dest = local_root.join(&relpath);
        if dest.exists() {
            continue;
        }
        materialize_podman_machine_cache_file(&src, &dest, true).await?;
        seeded += 1;
    }
    if seeded > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "seeded {seeded} podman machine cache file(s) from {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(super) async fn persist_podman_machine_cache_to_shared(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_podman_machine_cache_root() else {
        return Ok(());
    };
    let local_root = podman_machine_cache_root(data_root);
    let relpaths = collect_podman_machine_cache_file_relpaths(&local_root)?;
    if relpaths.is_empty() {
        return Ok(());
    }
    let mut persisted = 0usize;
    for relpath in relpaths {
        let src = local_root.join(&relpath);
        let dest = shared_root.join(&relpath);
        if src == dest || dest.exists() {
            continue;
        }
        materialize_podman_machine_cache_file(&src, &dest, false).await?;
        persisted += 1;
    }
    if persisted > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "persisted {persisted} podman machine cache file(s) into {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

pub(super) async fn seed_shared_podman_machine_cache_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    #[cfg(test)]
    {
        if let Err(err) = seed_shared_podman_machine_cache(data_root, observer).await {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to seed shared podman machine cache: {err:#}"),
            );
            tracing::warn!("failed to seed shared podman machine cache: {err:#}");
        }
    }
    #[cfg(not(test))]
    {
        let Some(shared_root) = shared_podman_machine_cache_root() else {
            return;
        };
        let local_root = data_root
            .join("podman")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("podman")
            .join("machine");
        let result = async {
            let relpaths = collect_podman_machine_cache_file_relpaths(&shared_root)?;
            if relpaths.is_empty() {
                return Ok(());
            }
            let mut seeded = 0usize;
            for relpath in relpaths {
                let src = shared_root.join(&relpath);
                let dest = local_root.join(&relpath);
                if dest.exists() {
                    continue;
                }
                materialize_podman_machine_cache_file(&src, &dest, true).await?;
                seeded += 1;
            }
            if seeded > 0 {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    &format!(
                        "seeded {seeded} podman machine cache file(s) from {}",
                        shared_root.display()
                    ),
                );
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(err) = result {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to seed shared podman machine cache: {err:#}"),
            );
            tracing::warn!("failed to seed shared podman machine cache: {err:#}");
        }
    }
}

pub(super) async fn persist_podman_machine_cache_to_shared_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    #[cfg(test)]
    {
        if let Err(err) = persist_podman_machine_cache_to_shared(data_root, observer).await {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to persist shared podman machine cache: {err:#}"),
            );
            tracing::warn!("failed to persist shared podman machine cache: {err:#}");
        }
    }
    #[cfg(not(test))]
    {
        let Some(shared_root) = shared_podman_machine_cache_root() else {
            return;
        };
        let local_root = data_root
            .join("podman")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("podman")
            .join("machine");
        let result = async {
            let relpaths = collect_podman_machine_cache_file_relpaths(&local_root)?;
            if relpaths.is_empty() {
                return Ok(());
            }
            let mut persisted = 0usize;
            for relpath in relpaths {
                let src = local_root.join(&relpath);
                let dest = shared_root.join(&relpath);
                if src == dest || dest.exists() {
                    continue;
                }
                materialize_podman_machine_cache_file(&src, &dest, false).await?;
                persisted += 1;
            }
            if persisted > 0 {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    &format!(
                        "persisted {persisted} podman machine cache file(s) into {}",
                        shared_root.display()
                    ),
                );
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(err) = result {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to persist shared podman machine cache: {err:#}"),
            );
            tracing::warn!("failed to persist shared podman machine cache: {err:#}");
        }
    }
}

fn managed_podman_machine_cache_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn managed_artifact_download_tmp_path(final_path: &Path) -> PathBuf {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("managed-artifact");
    final_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{file_name}.download-{}",
            uuid::Uuid::new_v4().simple()
        ))
}

async fn verify_managed_artifact_checksum(path: &Path, expected_sha256: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let digest = updates::sha256_hex_file(path)
        .await
        .with_context(|| format!("computing sha256 for {}", path.display()))?;
    Ok(digest.eq_ignore_ascii_case(expected_sha256.trim()))
}

async fn finalize_managed_artifact_download(
    tmp_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
    artifact_label: &str,
) -> Result<()> {
    let digest = updates::sha256_hex_file(tmp_path)
        .await
        .with_context(|| format!("computing sha256 for {}", tmp_path.display()))?;
    if !digest.eq_ignore_ascii_case(expected_sha256.trim()) {
        let _ = fs::remove_file(tmp_path).await;
        anyhow::bail!(
            "{artifact_label} checksum mismatch: expected {}, got {}",
            expected_sha256.trim(),
            digest
        );
    }

    match fs::rename(tmp_path, final_path).await {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            if verify_managed_artifact_checksum(final_path, expected_sha256).await? {
                let _ = fs::remove_file(tmp_path).await;
                return Ok(());
            }
            Err(rename_err).with_context(|| {
                format!(
                    "moving {artifact_label} into place: {} -> {}",
                    tmp_path.display(),
                    final_path.display()
                )
            })
        }
    }
}

pub(super) async fn ensure_managed_podman_machine_cache(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let _install_guard = managed_podman_machine_cache_install_lock().lock().await;
    let source = bundled_assets::managed_podman_machine_cache_source().ok_or_else(|| {
        anyhow::anyhow!(
            "managed podman machine cache source is not available for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let final_path = managed_podman_machine_cache_path(data_root, &source);
    if final_path.exists() {
        let digest = updates::sha256_hex_file(&final_path)
            .await
            .with_context(|| format!("computing sha256 for {}", final_path.display()))?;
        if digest.eq_ignore_ascii_case(source.sha256.trim()) {
            return Ok(final_path);
        }
        observe_log(
            observer,
            HarnessSetupPhase::ArtifactDownload,
            HarnessSetupLogLevel::Warn,
            &format!(
                "managed podman machine cache checksum mismatch for {}; re-downloading",
                final_path.display()
            ),
        );
        let _ = fs::remove_file(&final_path).await;
    }

    let Some(parent) = final_path.parent() else {
        anyhow::bail!(
            "managed podman machine cache path has no parent: {}",
            final_path.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let tmp_path = managed_artifact_download_tmp_path(&final_path);
    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "downloading managed podman machine cache into {}",
            final_path.display()
        ),
    );
    download_managed_artifact(
        &source.uri,
        &tmp_path,
        Some(ManagedArtifactDownloadReporter::new(
            observer,
            download_aggregate,
            HarnessSetupPhase::ArtifactDownload,
            "Podman machine cache",
        )),
    )
    .await?;
    finalize_managed_artifact_download(
        &tmp_path,
        &final_path,
        &source.sha256,
        "managed podman machine cache",
    )
    .await?;
    Ok(final_path)
}

pub(super) fn managed_podman_runtime_source() -> Option<bundled_assets::ManagedRuntimeSource> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    bundled_assets::managed_runtime_source("podman", os, arch)
}

pub(super) fn managed_podman_runtime_bin_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!("podman-{}", source.version.trim()))
        .join(source.bin.trim())
}

fn managed_podman_runtime_root(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!("podman-{}", source.version.trim()))
}

fn managed_podman_helper_path(runtime_root: &Path, helper_name: &str) -> Option<PathBuf> {
    let helper = helper_name.trim();
    if helper.is_empty() {
        return None;
    }
    Some(
        runtime_root
            .join("usr")
            .join("libexec")
            .join("podman")
            .join(helper),
    )
}

fn managed_artifact_extension(uri: &str) -> &'static str {
    let path = Url::parse(uri)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| uri.to_string());
    let path_lc = path.to_ascii_lowercase();
    if path_lc.ends_with(".tar.gz") {
        "tar.gz"
    } else if path_lc.ends_with(".tgz") {
        "tgz"
    } else if path_lc.ends_with(".tar") {
        "tar"
    } else {
        "zip"
    }
}

fn managed_podman_archive_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let ext = managed_artifact_extension(&source.uri);
    data_root
        .join("managed")
        .join("downloads")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!(
            "sha256-{}.{}",
            source.sha256.trim().to_ascii_lowercase(),
            ext
        ))
}

fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip archive")?;
    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).context("zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let dest = out_dir.join(enclosed);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut out).context("extract zip entry")?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

fn extract_archive_to_dir(archive_path: &Path, source_uri: &str, out_dir: &Path) -> Result<()> {
    let kind = managed_artifact_extension(source_uri);
    match kind {
        "zip" => extract_zip_to_dir(archive_path, out_dir),
        "tar.gz" | "tgz" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let decoder = flate2::read::GzDecoder::new(archive_file);
            let mut archive = tar::Archive::new(decoder);
            archive.unpack(out_dir).context("extract tar.gz archive")
        }
        "tar" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let mut archive = tar::Archive::new(archive_file);
            archive.unpack(out_dir).context("extract tar archive")
        }
        _ => anyhow::bail!("unsupported podman archive type for {source_uri}"),
    }
}

fn resolve_single_extracted_root(extract_dir: &Path) -> Result<PathBuf> {
    let mut dirs = Vec::new();
    let mut has_files = false;
    for entry in std::fs::read_dir(extract_dir)
        .with_context(|| format!("read_dir {}", extract_dir.display()))?
    {
        let entry = entry.with_context(|| format!("read_dir entry {}", extract_dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            has_files = true;
        }
    }
    if has_files || dirs.len() != 1 {
        return Ok(extract_dir.to_path_buf());
    }
    Ok(dirs.remove(0))
}

fn managed_podman_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn managed_podman_runtime_ready_marker_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join(".ctx-managed-ready")
}

fn managed_podman_runtime_is_ready(
    runtime_root: &Path,
    runtime_bin: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> bool {
    if !runtime_bin.exists() || !managed_podman_runtime_ready_marker_path(runtime_root).exists() {
        return false;
    }
    source.helpers.keys().all(|name| {
        managed_podman_helper_path(runtime_root, name)
            .map(|path| path.exists())
            .unwrap_or(true)
    })
}

async fn mark_managed_podman_runtime_ready(runtime_root: &Path) -> Result<()> {
    let marker = managed_podman_runtime_ready_marker_path(runtime_root);
    fs::write(&marker, b"ready")
        .await
        .with_context(|| format!("writing {}", marker.display()))
}

pub(super) async fn ensure_managed_podman_runtime(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    ensure_managed_podman_runtime_with_override(data_root, None, observer, download_aggregate).await
}

pub(super) async fn ensure_managed_podman_runtime_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedRuntimeSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    if source_override.is_none() {
        if let Ok(raw) = std::env::var(PODMAN_PATH_ENV) {
            let path = PathBuf::from(raw.trim());
            if path.exists() {
                return Ok(path);
            }
        }
        if let Some(bundled) = bundled_assets::bundled_podman_runtime() {
            return Ok(bundled.bin);
        }
    }
    let source = match source_override.cloned() {
        Some(source) => source,
        None => managed_podman_runtime_source().ok_or_else(|| {
            anyhow::anyhow!(
                "managed podman runtime source is not available for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?,
    };
    let runtime_root = managed_podman_runtime_root(data_root, &source);
    let runtime_bin = managed_podman_runtime_bin_path(data_root, &source);
    if managed_podman_runtime_is_ready(&runtime_root, &runtime_bin, &source) {
        return Ok(runtime_bin);
    }
    let _install_guard = managed_podman_install_lock().lock().await;
    if managed_podman_runtime_is_ready(&runtime_root, &runtime_bin, &source) {
        return Ok(runtime_bin);
    }

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!("installing managed podman runtime {}", source.version),
    );

    let final_archive = managed_podman_archive_path(data_root, &source);
    if final_archive.exists() {
        let digest = updates::sha256_hex_file(&final_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", final_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&final_archive).await;
        }
    }
    if !final_archive.exists() {
        let Some(parent) = final_archive.parent() else {
            anyhow::bail!(
                "managed podman archive path has no parent: {}",
                final_archive.display()
            );
        };
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
        let tmp_archive = final_archive.with_extension("download");
        download_managed_artifact(
            &source.uri,
            &tmp_archive,
            Some(ManagedArtifactDownloadReporter::new(
                observer,
                download_aggregate.clone(),
                HarnessSetupPhase::ArtifactDownload,
                "Podman runtime",
            )),
        )
        .await?;
        let digest = updates::sha256_hex_file(&tmp_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", tmp_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&tmp_archive).await;
            anyhow::bail!(
                "managed podman runtime checksum mismatch: expected {}, got {}",
                source.sha256.trim(),
                digest
            );
        }
        fs::rename(&tmp_archive, &final_archive)
            .await
            .with_context(|| {
                format!(
                    "moving managed podman archive into place: {} -> {}",
                    tmp_archive.display(),
                    final_archive.display()
                )
            })?;
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
    let staging_dir = parent.join(format!(".podman-staging-{}", uuid::Uuid::new_v4().simple()));
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
    .context("joining managed podman extract task")??;
    let extracted_root = tokio::task::spawn_blocking({
        let extract_dir = extract_dir.clone();
        move || resolve_single_extracted_root(&extract_dir)
    })
    .await
    .context("joining managed podman extraction root task")??;

    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root).await;
    }
    fs::rename(&extracted_root, &runtime_root)
        .await
        .with_context(|| {
            format!(
                "moving extracted podman runtime into place: {} -> {}",
                extracted_root.display(),
                runtime_root.display()
            )
        })?;
    let _ = fs::remove_dir_all(&staging_dir).await;

    let mut helper_downloads = Vec::new();
    for (name, helper) in &source.helpers {
        let Some(path) = managed_podman_helper_path(&runtime_root, name) else {
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
                    return Ok(()) as Result<()>;
                }
                let _ = fs::remove_file(&helper_path).await;
            }
            let tmp = helper_path.with_extension("download");
            download_managed_artifact(
                &helper_source.uri,
                &tmp,
                Some(ManagedArtifactDownloadReporter::new(
                    observer,
                    aggregate,
                    HarnessSetupPhase::ArtifactDownload,
                    format!("Podman helper ({helper_name})"),
                )),
            )
            .await?;
            let digest = updates::sha256_hex_file(&tmp)
                .await
                .with_context(|| format!("computing sha256 for {}", tmp.display()))?;
            if !digest.eq_ignore_ascii_case(helper_source.sha256.trim()) {
                let _ = fs::remove_file(&tmp).await;
                anyhow::bail!(
                    "managed podman helper checksum mismatch ({}): expected {}, got {}",
                    helper_name,
                    helper_source.sha256.trim(),
                    digest
                );
            }
            fs::rename(&tmp, &helper_path).await.with_context(|| {
                format!(
                    "moving managed podman helper into place: {} -> {}",
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
            "managed podman runtime installed but binary is missing at {}",
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
            if let Some(helper_path) = managed_podman_helper_path(&runtime_root, name) {
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
    mark_managed_podman_runtime_ready(&runtime_root).await?;
    Ok(runtime_bin)
}

fn format_byte_count(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Debug, Default)]
struct ManagedDownloadAggregateState {
    downloads: BTreeMap<String, ManagedDownloadArtifactState>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ManagedDownloadAggregate {
    inner: Arc<StdMutex<ManagedDownloadAggregateState>>,
}

#[derive(Debug, Clone, Default)]
struct ManagedDownloadArtifactState {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    bytes_per_sec: Option<u64>,
    finished: bool,
}

impl ManagedDownloadAggregate {
    fn update(
        &self,
        artifact: &str,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_sec: Option<u64>,
        finished: bool,
    ) -> Option<HarnessSetupDownloadStatus> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = inner.downloads.entry(artifact.to_string()).or_default();
        entry.downloaded_bytes = downloaded_bytes;
        entry.total_bytes = total_bytes;
        entry.bytes_per_sec = bytes_per_sec;
        entry.finished = finished;

        let all_finished = inner.downloads.values().all(|download| download.finished);
        if all_finished {
            return None;
        }

        let downloaded_total = inner
            .downloads
            .values()
            .map(|download| download.downloaded_bytes)
            .sum();
        let total_bytes = inner.downloads.values().try_fold(0u64, |acc, download| {
            download.total_bytes.map(|value| acc.saturating_add(value))
        });
        let bytes_per_sec = inner
            .downloads
            .values()
            .filter_map(|download| download.bytes_per_sec)
            .fold(None, |acc: Option<u64>, value| {
                Some(acc.unwrap_or(0u64).saturating_add(value))
            });
        Some(HarnessSetupDownloadStatus {
            artifact: "Required artifacts".to_string(),
            downloaded_bytes: downloaded_total,
            total_bytes,
            bytes_per_sec,
        })
    }
}

#[derive(Clone)]
pub(super) struct ManagedArtifactDownloadReporter<'a> {
    observer: Option<&'a dyn HarnessSetupObserver>,
    aggregate: Option<ManagedDownloadAggregate>,
    phase: HarnessSetupPhase,
    artifact: String,
}

impl<'a> ManagedArtifactDownloadReporter<'a> {
    pub(super) fn new(
        observer: Option<&'a dyn HarnessSetupObserver>,
        aggregate: Option<ManagedDownloadAggregate>,
        phase: HarnessSetupPhase,
        artifact: impl Into<String>,
    ) -> Self {
        Self {
            observer,
            aggregate,
            phase,
            artifact: artifact.into(),
        }
    }

    fn emit_progress(
        &self,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_sec: Option<u64>,
        finished: bool,
    ) {
        let active_download = if let Some(aggregate) = &self.aggregate {
            aggregate.update(
                &self.artifact,
                downloaded_bytes,
                total_bytes,
                bytes_per_sec,
                finished,
            )
        } else if finished {
            None
        } else {
            Some(HarnessSetupDownloadStatus {
                artifact: self.artifact.clone(),
                downloaded_bytes,
                total_bytes,
                bytes_per_sec,
            })
        };
        observe_progress(
            self.observer,
            HarnessSetupProgressUpdate {
                phase: self.phase,
                active_download,
            },
        );
    }
}

pub(super) async fn download_managed_artifact(
    url: &str,
    dest: &Path,
    reporter: Option<ManagedArtifactDownloadReporter<'_>>,
) -> Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("download destination missing parent: {}", dest.display());
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let client = reqwest::Client::builder()
        .timeout(PODMAN_LOAD_TIMEOUT)
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("building reqwest client for managed artifact download")?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("downloading managed artifact: {url}"))?
        .error_for_status()
        .with_context(|| format!("managed artifact download http error: {url}"))?;
    let total_bytes = response.content_length();
    if let Some(reporter) = reporter.as_ref() {
        observe_phase(
            reporter.observer,
            reporter.phase,
            "downloading required artifacts",
        );
        let size_suffix = total_bytes
            .map(format_byte_count)
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        observe_log(
            reporter.observer,
            reporter.phase,
            HarnessSetupLogLevel::Info,
            &format!("starting {} download{}", reporter.artifact, size_suffix),
        );
        reporter.emit_progress(0, total_bytes, None, false);
    }
    let mut stream = response.bytes_stream();
    let mut file = fs::File::create(dest)
        .await
        .with_context(|| format!("creating {}", dest.display()))?;
    let started = tokio::time::Instant::now();
    let mut downloaded_bytes = 0u64;
    let mut next_progress_pct = 10u64;
    let mut last_progress_snapshot = tokio::time::Instant::now();
    let mut last_progress_log = tokio::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("reading download stream from {url}"))?;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing {}", dest.display()))?;
        downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);

        let elapsed = started.elapsed();
        let bytes_per_sec = if elapsed.as_secs_f64() > 0.0 {
            Some((downloaded_bytes as f64 / elapsed.as_secs_f64()).round() as u64)
        } else {
            None
        };

        if let Some(reporter) = reporter.as_ref() {
            let now = tokio::time::Instant::now();
            let should_emit_snapshot = now.duration_since(last_progress_snapshot)
                >= Duration::from_secs(1)
                || total_bytes
                    .map(|total| downloaded_bytes >= total)
                    .unwrap_or(false);
            if should_emit_snapshot {
                reporter.emit_progress(downloaded_bytes, total_bytes, bytes_per_sec, false);
                last_progress_snapshot = now;
            }

            let should_emit_log = if let Some(total) = total_bytes {
                let pct = if total == 0 {
                    100
                } else {
                    ((downloaded_bytes as f64 / total as f64) * 100.0).floor() as u64
                };
                if pct >= next_progress_pct {
                    next_progress_pct = ((pct / 10) + 1) * 10;
                    true
                } else {
                    now.duration_since(last_progress_log) >= Duration::from_secs(15)
                }
            } else {
                now.duration_since(last_progress_log) >= Duration::from_secs(15)
            };

            if should_emit_log {
                let message = if let Some(total) = total_bytes {
                    let pct = if total == 0 {
                        100
                    } else {
                        ((downloaded_bytes as f64 / total as f64) * 100.0).floor() as u64
                    };
                    format!(
                        "{} download {}% ({} / {})",
                        reporter.artifact,
                        pct.min(100),
                        format_byte_count(downloaded_bytes),
                        format_byte_count(total),
                    )
                } else {
                    format!(
                        "{} download in progress ({})",
                        reporter.artifact,
                        format_byte_count(downloaded_bytes),
                    )
                };
                observe_log(
                    reporter.observer,
                    reporter.phase,
                    HarnessSetupLogLevel::Info,
                    &message,
                );
                last_progress_log = now;
            }
        }
    }
    file.flush()
        .await
        .with_context(|| format!("flushing {}", dest.display()))?;
    if let Some(reporter) = reporter.as_ref() {
        let elapsed = started.elapsed();
        let bytes_per_sec = if elapsed.as_secs_f64() > 0.0 {
            Some((downloaded_bytes as f64 / elapsed.as_secs_f64()).round() as u64)
        } else {
            None
        };
        reporter.emit_progress(downloaded_bytes, total_bytes, bytes_per_sec, true);
        let complete_suffix = total_bytes
            .map(format_byte_count)
            .unwrap_or_else(|| format_byte_count(downloaded_bytes));
        observe_log(
            reporter.observer,
            reporter.phase,
            HarnessSetupLogLevel::Info,
            &format!(
                "{} download complete ({complete_suffix})",
                reporter.artifact
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_download_aggregate_combines_parallel_artifact_progress() {
        let aggregate = ManagedDownloadAggregate::default();

        let first = aggregate
            .update("Podman runtime", 10, Some(40), Some(3), false)
            .expect("first aggregate snapshot");
        assert_eq!(first.artifact, "Required artifacts");
        assert_eq!(first.downloaded_bytes, 10);
        assert_eq!(first.total_bytes, Some(40));
        assert_eq!(first.bytes_per_sec, Some(3));

        let combined = aggregate
            .update("Harness image", 5, Some(20), Some(2), false)
            .expect("combined aggregate snapshot");
        assert_eq!(combined.downloaded_bytes, 15);
        assert_eq!(combined.total_bytes, Some(60));
        assert_eq!(combined.bytes_per_sec, Some(5));

        let still_running = aggregate
            .update("Podman runtime", 40, Some(40), Some(4), true)
            .expect("remaining artifact should keep aggregate active");
        assert_eq!(still_running.downloaded_bytes, 45);
        assert_eq!(still_running.total_bytes, Some(60));
        assert_eq!(still_running.bytes_per_sec, Some(6));

        let finished = aggregate.update("Harness image", 20, Some(20), Some(2), true);
        assert!(
            finished.is_none(),
            "aggregate should clear once all downloads finish"
        );
    }

    #[tokio::test]
    async fn partial_managed_podman_runtime_triggers_repair_instead_of_reuse() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = bundled_assets::ManagedRuntimeSource {
            uri: "http://127.0.0.1:9/podman-runtime.tar.gz".to_string(),
            sha256: "deadbeef".to_string(),
            version: "test-version".to_string(),
            bin: "bin/podman".to_string(),
            helpers: HashMap::from([(
                "gvproxy".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: "http://127.0.0.1:9/gvproxy".to_string(),
                    sha256: "deadbeef".to_string(),
                },
            )]),
        };

        let runtime_root = managed_podman_runtime_root(temp.path(), &source);
        let runtime_bin = managed_podman_runtime_bin_path(temp.path(), &source);
        fs::create_dir_all(runtime_bin.parent().expect("runtime bin parent"))
            .await
            .expect("create runtime bin parent");
        fs::write(&runtime_bin, b"partial-podman")
            .await
            .expect("write partial runtime binary");

        let err =
            ensure_managed_podman_runtime_with_override(temp.path(), Some(&source), None, None)
                .await
                .expect_err("partial runtime should trigger repair attempt");

        assert!(
            !managed_podman_runtime_is_ready(&runtime_root, &runtime_bin, &source),
            "missing ready marker/helper payload must not count as a ready runtime"
        );
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("downloading managed artifact")
                || rendered.contains("managed artifact download http error"),
            "repair should attempt managed runtime download, got: {rendered}"
        );
    }

    #[tokio::test]
    async fn finalize_managed_artifact_download_tolerates_parallel_committers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let final_path = temp.path().join("podman-machine");
        let first_tmp = temp.path().join("podman-machine.download-first");
        let second_tmp = temp.path().join("podman-machine.download-second");
        let payload = b"shared-machine-cache";

        fs::write(&first_tmp, payload)
            .await
            .expect("write first tmp payload");
        fs::write(&second_tmp, payload)
            .await
            .expect("write second tmp payload");
        let expected_sha256 = updates::sha256_hex_file(&first_tmp)
            .await
            .expect("compute tmp checksum");

        let (first, second) = tokio::join!(
            finalize_managed_artifact_download(
                &first_tmp,
                &final_path,
                &expected_sha256,
                "managed podman machine cache"
            ),
            finalize_managed_artifact_download(
                &second_tmp,
                &final_path,
                &expected_sha256,
                "managed podman machine cache"
            ),
        );

        first.expect("first finalization should succeed");
        second.expect("second finalization should succeed");
        assert!(
            verify_managed_artifact_checksum(&final_path, &expected_sha256)
                .await
                .expect("verify final checksum"),
            "final cache artifact should exist with the expected checksum"
        );
        assert!(
            !first_tmp.exists(),
            "first tmp path should be consumed during finalization"
        );
        assert!(
            !second_tmp.exists(),
            "second tmp path should be cleaned up during finalization"
        );
    }
}
