use super::*;

#[cfg(test)]
fn sandbox_machine_data_root_hash(data_root: &Path) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(data_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
pub(in crate::workspace_runtime) fn sandbox_machine_runtime_root(data_root: &Path) -> PathBuf {
    let hash = sandbox_machine_data_root_hash(data_root);
    #[cfg(unix)]
    {
        PathBuf::from("/tmp").join("ctxp").join(hash)
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("ctxp").join(hash)
    }
}

#[cfg(test)]
pub(in crate::workspace_runtime) fn sandbox_machine_home_root(data_root: &Path) -> PathBuf {
    sandbox_machine_runtime_root(data_root).join("home")
}

#[cfg(test)]
pub(in crate::workspace_runtime) fn sandbox_machine_temp_root(data_root: &Path) -> PathBuf {
    sandbox_machine_runtime_root(data_root).join("tmp")
}

#[cfg(test)]
pub(in crate::workspace_runtime) fn sandbox_machine_cache_root(data_root: &Path) -> PathBuf {
    data_root
        .join("sandbox-cli")
        .join("xdg")
        .join("data")
        .join("containers")
        .join("sandbox-cli")
        .join("machine")
}

#[cfg(test)]
fn shared_sandbox_machine_cache_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(SANDBOX_MACHINE_CACHE_DIR_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    directories::BaseDirs::new().map(|base| {
        base.cache_dir()
            .join("ctx")
            .join("sandbox-machine")
            .join(std::env::consts::OS)
            .join(std::env::consts::ARCH)
    })
}

#[cfg(test)]
fn managed_sandbox_machine_cache_root(data_root: &Path) -> PathBuf {
    shared_sandbox_machine_cache_root()
        .unwrap_or_else(|| data_root.join("managed").join("machine-cache"))
}

#[cfg(test)]
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

#[cfg(test)]
pub(in crate::workspace_runtime) fn managed_sandbox_machine_cache_path(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
) -> PathBuf {
    managed_sandbox_machine_cache_root(data_root)
        .join("managed")
        .join(managed_artifact_file_name(
            &source.uri,
            &source.sha256,
            SANDBOX_MACHINE_CACHE_ID,
        ))
}

#[cfg(test)]
fn collect_sandbox_machine_cache_file_relpaths(root: &Path) -> Result<Vec<PathBuf>> {
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

#[cfg(test)]
#[cfg(unix)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(test)]
#[cfg(windows)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dest)
}

#[cfg(test)]
#[cfg(not(any(unix, windows)))]
fn symlink_cache_file(_src: &Path, _dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file symlinks are not supported on this platform",
    ))
}

#[cfg(test)]
fn sandbox_machine_cache_tmp_path(dest: &Path) -> PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("cache-file");
    dest.with_file_name(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

#[cfg(test)]
async fn materialize_sandbox_machine_cache_file(
    src: &Path,
    dest: &Path,
    allow_symlink: bool,
) -> Result<()> {
    if src == dest || dest.exists() {
        return Ok(());
    }
    let Some(parent) = dest.parent() else {
        anyhow::bail!(
            "sandbox machine cache target has no parent: {}",
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

    let tmp = sandbox_machine_cache_tmp_path(dest);
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
pub(in crate::workspace_runtime) async fn seed_shared_sandbox_machine_cache(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_sandbox_machine_cache_root() else {
        return Ok(());
    };
    let relpaths = collect_sandbox_machine_cache_file_relpaths(&shared_root)?;
    if relpaths.is_empty() {
        return Ok(());
    }
    let local_root = sandbox_machine_cache_root(data_root);
    let mut seeded = 0usize;
    for relpath in relpaths {
        let src = shared_root.join(&relpath);
        let dest = local_root.join(&relpath);
        if dest.exists() {
            continue;
        }
        materialize_sandbox_machine_cache_file(&src, &dest, true).await?;
        seeded += 1;
    }
    if seeded > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "seeded {seeded} sandbox machine cache file(s) from {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::workspace_runtime) async fn persist_sandbox_machine_cache_to_shared(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_sandbox_machine_cache_root() else {
        return Ok(());
    };
    let local_root = sandbox_machine_cache_root(data_root);
    let relpaths = collect_sandbox_machine_cache_file_relpaths(&local_root)?;
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
        materialize_sandbox_machine_cache_file(&src, &dest, false).await?;
        persisted += 1;
    }
    if persisted > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "persisted {persisted} sandbox machine cache file(s) into {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::workspace_runtime) async fn seed_shared_sandbox_machine_cache_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    #[cfg(test)]
    {
        if let Err(err) = seed_shared_sandbox_machine_cache(data_root, observer).await {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to seed shared sandbox machine cache: {err:#}"),
            );
            tracing::warn!("failed to seed shared sandbox machine cache: {err:#}");
        }
    }
    #[cfg(not(test))]
    {
        let Some(shared_root) = shared_sandbox_machine_cache_root() else {
            return;
        };
        let local_root = data_root
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine");
        let result = async {
            let relpaths = collect_sandbox_machine_cache_file_relpaths(&shared_root)?;
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
                materialize_sandbox_machine_cache_file(&src, &dest, true).await?;
                seeded += 1;
            }
            if seeded > 0 {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    &format!(
                        "seeded {seeded} sandbox machine cache file(s) from {}",
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
                &format!("failed to seed shared sandbox machine cache: {err:#}"),
            );
            tracing::warn!("failed to seed shared sandbox machine cache: {err:#}");
        }
    }
}

#[cfg(test)]
pub(in crate::workspace_runtime) async fn persist_sandbox_machine_cache_to_shared_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    #[cfg(test)]
    {
        if let Err(err) = persist_sandbox_machine_cache_to_shared(data_root, observer).await {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("failed to persist shared sandbox machine cache: {err:#}"),
            );
            tracing::warn!("failed to persist shared sandbox machine cache: {err:#}");
        }
    }
    #[cfg(not(test))]
    {
        let Some(shared_root) = shared_sandbox_machine_cache_root() else {
            return;
        };
        let local_root = data_root
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine");
        let result = async {
            let relpaths = collect_sandbox_machine_cache_file_relpaths(&local_root)?;
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
                materialize_sandbox_machine_cache_file(&src, &dest, false).await?;
                persisted += 1;
            }
            if persisted > 0 {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    &format!(
                        "persisted {persisted} sandbox machine cache file(s) into {}",
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
                &format!("failed to persist shared sandbox machine cache: {err:#}"),
            );
            tracing::warn!("failed to persist shared sandbox machine cache: {err:#}");
        }
    }
}
