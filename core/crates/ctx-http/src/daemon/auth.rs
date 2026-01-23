use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";

pub(super) fn acquire_daemon_lock(data_root: &Path) -> Result<std::fs::File> {
    let path = data_root.join("daemon.lock");
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("opening daemon lockfile {}", path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.set_len(0);
            let _ = writeln!(file, "{}", std::process::id());
            let _ = file.sync_all();
            Ok(file)
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            anyhow::bail!("ctx daemon already running (lockfile {})", path.display())
        }
        Err(e) => Err(e).with_context(|| format!("locking daemon lockfile {}", path.display())),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct DaemonAuthFile {
    pub(super) token: String,
    #[serde(default)]
    pub(super) daemon_url: Option<String>,
}

pub(super) fn daemon_auth_path(data_root: &Path) -> PathBuf {
    data_root.join(DAEMON_AUTH_FILENAME)
}

fn read_daemon_auth_file(path: &Path) -> Result<Option<DaemonAuthFile>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let auth: DaemonAuthFile = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing daemon auth file {}", path.display()))?;
            if auth.token.trim().is_empty() {
                anyhow::bail!("daemon auth file {} contains empty token", path.display());
            }
            Ok(Some(auth))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("reading daemon auth file {}", path.display()))
        }
    }
}

pub(super) fn write_daemon_auth_file(path: &Path, auth: &DaemonAuthFile) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(auth)?;
    std::fs::write(&tmp, bytes)?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    Ok(())
}

pub(super) fn load_or_init_daemon_auth(data_root: &Path) -> Result<DaemonAuthFile> {
    let path = daemon_auth_path(data_root);
    if let Some(auth) = read_daemon_auth_file(&path)? {
        return Ok(auth);
    }
    let auth = DaemonAuthFile {
        token: uuid::Uuid::new_v4().to_string(),
        daemon_url: None,
    };
    write_daemon_auth_file(&path, &auth)?;
    Ok(auth)
}
