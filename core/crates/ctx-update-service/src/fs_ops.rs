use super::*;
use sha2::Digest;
use std::sync::atomic::{AtomicU64, Ordering};

static UPDATE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub async fn download_to_path(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let resp = reqwest::get(url)
        .await
        .with_context(|| format!("downloading: {url}"))?
        .error_for_status()
        .with_context(|| format!("download http error: {url}"))?;
    let bytes = resp.bytes().await.context("reading download body")?;
    tokio::fs::write(dest, &bytes)
        .await
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

pub async fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

pub async fn atomic_replace_exe_with_backup(
    current_exe: &Path,
    new_file: &Path,
) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(new_file).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(new_file, perms).await.ok();
    }

    let backup = current_exe.with_extension("bak");
    if backup.exists() {
        tokio::fs::remove_file(&backup).await.ok();
    }

    tokio::fs::rename(current_exe, &backup)
        .await
        .with_context(|| format!("moving current exe to backup: {}", backup.display()))?;
    if let Err(e) = tokio::fs::rename(new_file, current_exe).await {
        let _ = tokio::fs::rename(&backup, current_exe).await;
        return Err(e)
            .with_context(|| format!("moving new exe into place: {}", current_exe.display()));
    }

    Ok(backup)
}

pub async fn atomic_replace_exe(current_exe: &Path, new_file: &Path) -> Result<()> {
    let _ = atomic_replace_exe_with_backup(current_exe, new_file).await?;
    Ok(())
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) async fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let sequence = UPDATE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value).context("serializing JSON")?;
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("writing {}", tmp.display()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .with_context(|| format!("committing {} -> {}", tmp.display(), path.display()))
}

pub(super) fn next_update_temp_sequence() -> u64 {
    UPDATE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
}
