use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_fs::permissions::{
    read_private_file_to_string_sync, reject_symlink_sync, write_private_file_atomic_sync,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";

pub(super) fn acquire_daemon_lock(data_root: &Path) -> Result<std::fs::File> {
    let path = data_root.join("daemon.lock");
    reject_symlink_if_exists(&path)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("opening daemon lockfile {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    let _ = ctx_fs::permissions::harden_private_file_sync(&path);

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
    let Some(contents) = read_private_file_to_string_sync(path)? else {
        return Ok(None);
    };
    let auth: DaemonAuthFile = serde_json::from_str(&contents)
        .with_context(|| format!("parsing daemon auth file {}", path.display()))?;
    if auth.token.trim().is_empty() {
        anyhow::bail!("daemon auth file {} contains empty token", path.display());
    }
    Ok(Some(auth))
}

fn reject_symlink_if_exists(path: &Path) -> Result<Option<()>> {
    if reject_symlink_sync(path)? {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

pub(super) fn write_daemon_auth_file(path: &Path, auth: &DaemonAuthFile) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(auth)?;
    let _ = std::fs::remove_file(&tmp);
    write_private_file_atomic_sync(path, &bytes)?;
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

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::{
        acquire_daemon_lock, daemon_auth_path, load_or_init_daemon_auth, write_daemon_auth_file,
        DaemonAuthFile,
    };

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn load_or_init_daemon_auth_repairs_existing_file_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let path = daemon_auth_path(temp.path());
        write_daemon_auth_file(
            &path,
            &DaemonAuthFile {
                token: "token".to_string(),
                daemon_url: None,
            },
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let auth = load_or_init_daemon_auth(temp.path()).unwrap();

        assert_eq!(auth.token, "token");
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn load_or_init_daemon_auth_rejects_symlinked_auth_file() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = daemon_auth_path(temp.path());
        let outside_path = outside.path().join("daemon_auth.json");
        std::fs::write(&outside_path, r#"{"token":"outside"}"#).unwrap();
        std::os::unix::fs::symlink(&outside_path, &path).unwrap();

        let err = load_or_init_daemon_auth(temp.path()).unwrap_err();

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }

    #[test]
    fn acquire_daemon_lock_creates_private_lock_file() {
        let temp = tempfile::tempdir().unwrap();

        let lock = acquire_daemon_lock(temp.path()).unwrap();

        assert_eq!(mode(&temp.path().join("daemon.lock")), 0o600);
        drop(lock);
    }

    #[test]
    fn acquire_daemon_lock_rejects_symlinked_lock_file() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().join("daemon.lock");
        std::fs::write(&outside_path, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside_path, temp.path().join("daemon.lock")).unwrap();

        let err = acquire_daemon_lock(temp.path()).unwrap_err();

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }
}
