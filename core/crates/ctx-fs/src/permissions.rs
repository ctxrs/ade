use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    },
    Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
};

pub const PRIVATE_DIR_MODE: u32 = 0o700;
pub const PRIVATE_FILE_MODE: u32 = 0o600;
#[cfg(windows)]
const WINDOWS_PRIVATE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn ensure_private_dir_sync(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("creating private directory {}", path.display()))?;
    harden_private_dir_sync(path)
}

pub async fn ensure_private_dir(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || ensure_private_dir_sync(&path))
        .await
        .context("joining private directory creation task")?
}

pub fn harden_private_dir_sync(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIR_MODE))
            .with_context(|| format!("chmod 0700 {}", path.display()))?;
    }
    #[cfg(windows)]
    apply_windows_private_acl(path)?;
    Ok(())
}

pub fn harden_private_file_sync(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }
    #[cfg(windows)]
    apply_windows_private_acl(path)?;
    Ok(())
}

pub fn reject_symlink_sync(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!("private path must not be a symlink: {}", path.display());
            }
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("reading private path {}", path.display())),
    }
}

pub fn read_private_file_to_string_sync(path: &Path) -> Result<Option<String>> {
    if !reject_symlink_sync(path)? {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("opening private file {}", path.display()));
        }
    };
    let metadata = file
        .metadata()
        .with_context(|| format!("reading private file metadata {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("private path must be a regular file: {}", path.display());
    }
    harden_private_open_file_sync(&file, path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("reading private file {}", path.display()))?;
    Ok(Some(contents))
}

pub async fn harden_private_file_if_exists(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if path.exists() {
            harden_private_file_sync(&path)?;
        }
        Ok(())
    })
    .await
    .context("joining private file chmod task")?
}

pub fn write_private_file_atomic_sync(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("missing parent directory for {}", path.display()))?;
    ensure_private_dir_sync(parent)?;

    let mut last_error = None;
    for _ in 0..32 {
        let tmp_path = private_temp_path(parent);
        match create_private_new_file(&tmp_path) {
            Ok(mut file) => {
                let result = (|| -> Result<()> {
                    harden_private_file_sync(&tmp_path)?;
                    file.write_all(bytes)
                        .with_context(|| format!("writing {}", tmp_path.display()))?;
                    file.sync_data()
                        .with_context(|| format!("syncing {}", tmp_path.display()))?;
                    drop(file);
                    fs::rename(&tmp_path, path).with_context(|| {
                        format!("renaming {} to {}", tmp_path.display(), path.display())
                    })?;
                    harden_private_file_sync(path)?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = fs::remove_file(&tmp_path);
                }
                return result;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(err);
                continue;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("creating private temp file {}", tmp_path.display()));
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "private temp file collision limit exceeded",
            )
        })
        .into())
}

pub async fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || write_private_file_atomic_sync(&path, &bytes))
        .await
        .context("joining private file write task")?
}

pub fn open_private_append_sync(path: &Path) -> Result<File> {
    let parent = path
        .parent()
        .with_context(|| format!("missing parent directory for {}", path.display()))?;
    ensure_private_dir_sync(parent)?;
    reject_symlink_sync(path)?;
    let file = open_private_append_options()
        .open(path)
        .with_context(|| format!("opening private append file {}", path.display()))?;
    harden_private_open_file_sync(&file, path)?;
    Ok(file)
}

pub async fn open_private_append(path: &Path) -> Result<tokio::fs::File> {
    let path = path.to_path_buf();
    let file = tokio::task::spawn_blocking(move || open_private_append_sync(&path))
        .await
        .context("joining private append open task")??;
    Ok(tokio::fs::File::from_std(file))
}

pub async fn harden_sqlite_file_family(path: &Path) -> Result<()> {
    for file in sqlite_file_family(path) {
        harden_private_file_if_exists(&file).await?;
    }
    Ok(())
}

pub fn harden_private_directory_files_sync(
    dir: &Path,
    should_harden: impl Fn(&str) -> bool,
) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
    };

    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry under {}", dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if should_harden(&name) {
            harden_private_file_sync(&entry.path())?;
        }
    }
    Ok(())
}

fn sqlite_file_family(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.to_string_lossy())),
        PathBuf::from(format!("{}-shm", path.to_string_lossy())),
    ]
}

fn private_temp_path(parent: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    parent.join(format!(
        ".ctx-private-{}-{nanos}-{counter}.tmp",
        std::process::id()
    ))
}

#[cfg(unix)]
fn create_private_new_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_new_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create_new(true).write(true).open(path)
}

#[cfg(unix)]
fn open_private_append_options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .create(true)
        .append(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW);
    options
}

#[cfg(not(unix))]
fn open_private_append_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}

fn harden_private_open_file_sync(file: &File, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .with_context(|| format!("chmod 0600 open file {}", path.display()))?;
    }
    #[cfg(windows)]
    apply_windows_private_acl(path)?;
    Ok(())
}

#[cfg(windows)]
fn encode_windows_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn encode_windows_sddl(sddl: &str) -> Vec<u16> {
    std::ffi::OsStr::new(sddl)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn apply_windows_private_acl(path: &Path) -> Result<()> {
    let path_wide = encode_windows_path(path);
    let sddl_wide = encode_windows_sddl(WINDOWS_PRIVATE_SDDL);
    let mut security_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(),
            SDDL_REVISION_1 as u32,
            &mut security_descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        anyhow::bail!("failed to build Windows private ACL for {}", path.display());
    }
    let result = unsafe {
        SetFileSecurityW(
            path_wide.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            security_descriptor,
        )
    };
    unsafe {
        let _ = LocalFree(security_descriptor as isize);
    }
    if result == 0 {
        anyhow::bail!("failed to apply Windows private ACL to {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;

    use super::{
        ensure_private_dir_sync, open_private_append_sync, read_private_file_to_string_sync,
        write_private_file_atomic_sync, PRIVATE_DIR_MODE, PRIVATE_FILE_MODE,
    };

    #[test]
    fn private_dir_sync_sets_0700() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private");

        ensure_private_dir_sync(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_DIR_MODE);
    }

    #[test]
    fn private_atomic_write_sets_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");

        write_private_file_atomic_sync(&path, b"secret").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
    }

    #[test]
    fn private_append_create_sets_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");

        let mut file = open_private_append_sync(&path).unwrap();
        file.write_all(b"line\n").unwrap();
        drop(file);

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
    }

    #[test]
    fn private_append_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target.log");
        let link = dir.path().join("daemon.log");
        std::fs::write(&target, b"outside").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = open_private_append_sync(&link).unwrap_err();

        assert!(format!("{err:#}").contains("must not be a symlink"));
        assert_eq!(std::fs::read(&target).unwrap(), b"outside");
    }

    #[test]
    fn private_read_repairs_permissions_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        std::fs::write(&path, "secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let contents = read_private_file_to_string_sync(&path).unwrap().unwrap();

        assert_eq!(contents, "secret");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
    }

    #[test]
    fn private_read_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret.json");
        let link = dir.path().join("secret.json");
        std::fs::write(&target, "outside").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = read_private_file_to_string_sync(&link).unwrap_err();

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }
}
