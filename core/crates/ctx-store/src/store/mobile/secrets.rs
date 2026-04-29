use super::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    },
};

const MOBILE_ACCESS_SECRET_VERSION: u32 = 1;
#[cfg(windows)]
const WINDOWS_SECRET_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileAccessSecretEnvelope {
    version: u32,
    tunnel_secret: String,
    daemon_private_key: String,
}

fn ensure_safe_mobile_access_secret_ref(secret_ref: &str) -> Result<()> {
    if secret_ref.trim().is_empty() {
        anyhow::bail!("mobile access secret_ref is required");
    }
    let mut components = std::path::Path::new(secret_ref).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(()),
        _ => anyhow::bail!("mobile access secret_ref must be a single path segment"),
    }
}

fn mobile_access_secret_root(db_path: &Path) -> PathBuf {
    let db_namespace = db_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("store.sqlite"));
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("mobile_access_secrets")
        .join(db_namespace)
}

fn mobile_access_secret_path(db_path: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_mobile_access_secret_ref(secret_ref)?;
    Ok(mobile_access_secret_root(db_path).join(format!("{secret_ref}.json")))
}

fn ensure_safe_runtime_settings_secret_ref(secret_ref: &str) -> Result<()> {
    if secret_ref.trim().is_empty() {
        anyhow::bail!("runtime settings secret_ref is required");
    }
    let mut components = std::path::Path::new(secret_ref).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(()),
        _ => anyhow::bail!("runtime settings secret_ref must be a single path segment"),
    }
}

fn runtime_settings_secret_root(db_path: &Path) -> PathBuf {
    let db_namespace = db_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("store.sqlite"));
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("runtime_settings_secrets")
        .join(db_namespace)
}

fn runtime_settings_secret_path(db_path: &Path, secret_ref: &str) -> Result<PathBuf> {
    ensure_safe_runtime_settings_secret_ref(secret_ref)?;
    Ok(runtime_settings_secret_root(db_path).join(format!("{secret_ref}.json")))
}

async fn ensure_private_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    #[cfg(windows)]
    apply_windows_secret_acl(path)?;
    Ok(())
}

async fn write_secure_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent for {}", path.display()))?;
    ensure_private_dir(parent).await?;
    let path = path.to_path_buf();
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        let result = write_secure_file_atomic_sync(&path, &tmp, &bytes);
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    })
    .await
    .context("joining mobile secret write task")??;
    Ok(())
}

fn write_secure_file_atomic_sync(path: &Path, tmp: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = create_private_secret_file(tmp)
        .with_context(|| format!("creating private temp file {}", tmp.display()))?;
    #[cfg(windows)]
    apply_windows_secret_acl(tmp)?;
    file.write_all(bytes)
        .with_context(|| format!("writing private temp file {}", tmp.display()))?;
    file.sync_data()
        .with_context(|| format!("syncing private temp file {}", tmp.display()))?;
    drop(file);
    std::fs::rename(tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    apply_windows_secret_acl(path)?;
    Ok(())
}

#[cfg(unix)]
fn create_private_secret_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_secret_file(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().create_new(true).write(true).open(path)
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
fn apply_windows_secret_acl(path: &Path) -> Result<()> {
    let path_wide = encode_windows_path(path);
    let sddl_wide = encode_windows_sddl(WINDOWS_SECRET_SDDL);
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
        anyhow::bail!("failed to build Windows secret ACL for {}", path.display());
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
        anyhow::bail!("failed to apply Windows secret ACL to {}", path.display());
    }
    Ok(())
}

impl Store {
    pub(super) fn next_mobile_access_secret_ref() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub(super) fn next_runtime_settings_secret_ref() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub(super) fn mobile_access_secret_db_path(&self) -> Result<&Path> {
        self.sqlite_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "mobile access secret storage requires a filesystem-backed sqlite store"
            )
        })
    }

    pub(super) fn runtime_settings_secret_db_path(&self) -> Result<&Path> {
        self.sqlite_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "runtime settings secret storage requires a filesystem-backed sqlite store"
            )
        })
    }

    pub(super) async fn write_mobile_access_secrets(
        &self,
        secret_ref: &str,
        tunnel_secret: &str,
        daemon_private_key: &str,
    ) -> Result<()> {
        let path = mobile_access_secret_path(self.mobile_access_secret_db_path()?, secret_ref)?;
        let payload = serde_json::to_vec_pretty(&MobileAccessSecretEnvelope {
            version: MOBILE_ACCESS_SECRET_VERSION,
            tunnel_secret: tunnel_secret.to_string(),
            daemon_private_key: daemon_private_key.to_string(),
        })?;
        write_secure_file_atomic(&path, &payload).await
    }

    pub(super) async fn read_mobile_access_secrets_if_present(
        &self,
        secret_ref: &str,
    ) -> Result<Option<(String, String)>> {
        let path = mobile_access_secret_path(self.mobile_access_secret_db_path()?, secret_ref)?;
        let payload = match tokio::fs::read_to_string(&path).await {
            Ok(payload) => payload,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("reading mobile access secrets from {}", path.display())
                });
            }
        };
        let envelope: MobileAccessSecretEnvelope = serde_json::from_str(&payload)
            .with_context(|| format!("parsing mobile access secrets from {}", path.display()))?;
        if envelope.version != MOBILE_ACCESS_SECRET_VERSION {
            anyhow::bail!(
                "unsupported mobile access secret version {} at {}",
                envelope.version,
                path.display()
            );
        }
        if envelope.tunnel_secret.trim().is_empty() || envelope.daemon_private_key.trim().is_empty()
        {
            anyhow::bail!(
                "mobile access secrets at {} must include tunnel_secret and daemon_private_key",
                path.display()
            );
        }
        Ok(Some((envelope.tunnel_secret, envelope.daemon_private_key)))
    }

    pub(super) async fn remove_mobile_access_secrets_if_present(
        &self,
        secret_ref: &str,
    ) -> Result<()> {
        let path = mobile_access_secret_path(self.mobile_access_secret_db_path()?, secret_ref)?;
        match tokio::fs::remove_file(&path).await {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err)
                .with_context(|| format!("removing mobile access secrets at {}", path.display())),
        }
    }

    pub(super) async fn write_runtime_settings_secrets(
        &self,
        secret_ref: &str,
        settings_secret_json: &str,
    ) -> Result<()> {
        let path =
            runtime_settings_secret_path(self.runtime_settings_secret_db_path()?, secret_ref)?;
        write_secure_file_atomic(&path, settings_secret_json.as_bytes()).await
    }

    pub async fn read_runtime_settings_secrets_if_present(
        &self,
        secret_ref: &str,
    ) -> Result<Option<String>> {
        let path =
            runtime_settings_secret_path(self.runtime_settings_secret_db_path()?, secret_ref)?;
        match tokio::fs::read_to_string(&path).await {
            Ok(payload) => Ok(Some(payload)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err)
                .with_context(|| format!("reading runtime settings secrets at {}", path.display())),
        }
    }

    pub(super) async fn remove_runtime_settings_secrets_if_present(
        &self,
        secret_ref: &str,
    ) -> Result<()> {
        let path =
            runtime_settings_secret_path(self.runtime_settings_secret_db_path()?, secret_ref)?;
        match tokio::fs::remove_file(&path).await {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).with_context(|| {
                format!("removing runtime settings secrets at {}", path.display())
            }),
        }
    }

    pub(super) async fn migrate_legacy_mobile_access_secrets(
        &self,
        id: &str,
        legacy_tunnel_secret: &str,
        legacy_daemon_private_key: &str,
    ) -> Result<Option<(String, String)>> {
        if legacy_tunnel_secret.trim().is_empty() || legacy_daemon_private_key.trim().is_empty() {
            return Ok(None);
        }
        let secret_ref = Self::next_mobile_access_secret_ref();
        self.write_mobile_access_secrets(
            &secret_ref,
            legacy_tunnel_secret,
            legacy_daemon_private_key,
        )
        .await?;
        if let Err(err) = self
            .finalize_legacy_mobile_access_secret_migration(id, &secret_ref)
            .await
        {
            let _ = self
                .remove_mobile_access_secrets_if_present(&secret_ref)
                .await;
            return Err(err);
        }
        self.checkpoint_wal_truncate().await?;
        Ok(Some((
            legacy_tunnel_secret.to_string(),
            legacy_daemon_private_key.to_string(),
        )))
    }

    pub(super) async fn migrate_legacy_mobile_access_sidecar(
        &self,
        id: &str,
    ) -> Result<Option<(String, String)>> {
        let Some((tunnel_secret, daemon_private_key)) =
            self.read_mobile_access_secrets_if_present(id).await?
        else {
            return Ok(None);
        };
        let secret_ref = Self::next_mobile_access_secret_ref();
        self.write_mobile_access_secrets(&secret_ref, &tunnel_secret, &daemon_private_key)
            .await?;
        if let Err(err) = self
            .finalize_legacy_mobile_access_secret_migration(id, &secret_ref)
            .await
        {
            let _ = self
                .remove_mobile_access_secrets_if_present(&secret_ref)
                .await;
            return Err(err);
        }
        self.remove_mobile_access_secrets_if_present(id).await?;
        Ok(Some((tunnel_secret, daemon_private_key)))
    }

    pub(super) async fn lookup_mobile_access_secret_ref(&self, id: &str) -> Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT secret_ref FROM mobile_access_config WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.flatten())
        .map_err(Into::into)
    }

    pub(super) async fn lookup_runtime_settings_secret_ref(&self) -> Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT secret_ref FROM runtime_settings WHERE id = ?"#,
        )
        .bind("default")
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.flatten())
        .map_err(Into::into)
    }

    pub(super) async fn finalize_legacy_mobile_access_secret_migration(
        &self,
        id: &str,
        secret_ref: &str,
    ) -> Result<()> {
        self.query(
            r#"UPDATE mobile_access_config
               SET secret_ref = ?, tunnel_secret = '', daemon_private_key = ''
               WHERE id = ?"#,
        )
        .bind(secret_ref)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(super) async fn clear_legacy_mobile_access_secrets(&self, id: &str) -> Result<()> {
        self.query(
            r#"UPDATE mobile_access_config
               SET tunnel_secret = '', daemon_private_key = ''
               WHERE id = ?"#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.checkpoint_wal_truncate().await?;
        Ok(())
    }
}
