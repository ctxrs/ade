use super::*;
use serde::{Deserialize, Serialize};
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
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await?;
    }
    #[cfg(windows)]
    apply_windows_secret_acl(&tmp)?;
    tokio::fs::rename(&tmp, path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    #[cfg(windows)]
    apply_windows_secret_acl(path)?;
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

pub struct MobileDeviceUpsert {
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub push_token: Option<String>,
    pub push_provider: Option<String>,
    pub public_key: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug)]
pub struct MobileAccessConfig {
    pub id: String,
    pub profile_id: ConnectionProfileId,
    pub tunnel_id: String,
    pub public_base_url: String,
    pub relay_base_url: String,
    pub tunnel_secret: String,
    pub daemon_public_key: String,
    pub daemon_private_key: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct RuntimeSettingsDocument {
    pub id: String,
    pub schema_version: i64,
    pub settings_json: String,
    pub secret_ref: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub enum MobileDeviceSeqAdvance {
    Advanced,
    Stale { current: i64 },
    Missing,
}

impl Store {
    fn next_mobile_access_secret_ref() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn next_runtime_settings_secret_ref() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn mobile_access_secret_db_path(&self) -> Result<&Path> {
        self.sqlite_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "mobile access secret storage requires a filesystem-backed sqlite store"
            )
        })
    }

    fn runtime_settings_secret_db_path(&self) -> Result<&Path> {
        self.sqlite_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "runtime settings secret storage requires a filesystem-backed sqlite store"
            )
        })
    }

    async fn write_mobile_access_secrets(
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

    async fn read_mobile_access_secrets_if_present(
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

    async fn remove_mobile_access_secrets_if_present(&self, secret_ref: &str) -> Result<()> {
        let path = mobile_access_secret_path(self.mobile_access_secret_db_path()?, secret_ref)?;
        match tokio::fs::remove_file(&path).await {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err)
                .with_context(|| format!("removing mobile access secrets at {}", path.display())),
        }
    }

    async fn write_runtime_settings_secrets(
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

    async fn remove_runtime_settings_secrets_if_present(&self, secret_ref: &str) -> Result<()> {
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

    async fn migrate_legacy_mobile_access_secrets(
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

    async fn migrate_legacy_mobile_access_sidecar(
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

    async fn lookup_mobile_access_secret_ref(&self, id: &str) -> Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT secret_ref FROM mobile_access_config WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.flatten())
        .map_err(Into::into)
    }

    async fn lookup_runtime_settings_secret_ref(&self) -> Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT secret_ref FROM runtime_settings WHERE id = ?"#,
        )
        .bind("default")
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.flatten())
        .map_err(Into::into)
    }

    async fn finalize_legacy_mobile_access_secret_migration(
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

    async fn clear_legacy_mobile_access_secrets(&self, id: &str) -> Result<()> {
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

impl Store {
    // Mobile connection profiles + devices
    pub async fn create_mobile_connection_profile(
        &self,
        label: String,
        base_url: String,
        token_hash: String,
        token_prefix: String,
        scopes: Vec<String>,
    ) -> Result<MobileConnectionProfile> {
        let now = Utc::now();
        let profile = MobileConnectionProfile {
            id: ConnectionProfileId::new(),
            label,
            base_url,
            token_prefix,
            scopes,
            created_at: now,
            last_used_at: None,
        };
        let scopes_json = serde_json::to_string(&profile.scopes)?;
        self.query(
            r#"INSERT INTO mobile_connection_profiles
               (id, label, base_url, token_hash, token_prefix, scopes_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(profile.id.0.to_string())
        .bind(&profile.label)
        .bind(&profile.base_url)
        .bind(&token_hash)
        .bind(&profile.token_prefix)
        .bind(scopes_json)
        .bind(profile.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(profile)
    }

    pub async fn list_mobile_connection_profiles(&self) -> Result<Vec<MobileConnectionProfile>> {
        let rows = self
            .query(
                r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles
               ORDER BY created_at DESC"#,
            )
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(build_mobile_connection_profile_from_row(row)?);
        }
        Ok(out)
    }

    pub async fn get_mobile_connection_profile(
        &self,
        id: ConnectionProfileId,
    ) -> Result<Option<MobileConnectionProfile>> {
        let row = self
            .query(
                r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles WHERE id = ?"#,
            )
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(build_mobile_connection_profile_from_row)
            .transpose()
    }

    pub async fn get_mobile_connection_profile_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<MobileConnectionProfile>> {
        let row = self
            .query(
                r#"SELECT id, label, base_url, token_prefix, scopes_json, created_at, last_used_at
               FROM mobile_connection_profiles WHERE token_hash = ?"#,
            )
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?;
        row.map(build_mobile_connection_profile_from_row)
            .transpose()
    }

    pub async fn mark_mobile_connection_profile_used(&self, id: ConnectionProfileId) -> Result<()> {
        self.query(r#"UPDATE mobile_connection_profiles SET last_used_at = ? WHERE id = ?"#)
            .bind(Utc::now().to_rfc3339())
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_mobile_connection_profile_scopes(
        &self,
        id: ConnectionProfileId,
        scopes: Vec<String>,
    ) -> Result<()> {
        let scopes_json = serde_json::to_string(&scopes)?;
        self.query(r#"UPDATE mobile_connection_profiles SET scopes_json = ? WHERE id = ?"#)
            .bind(scopes_json)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_mobile_connection_profile(&self, id: ConnectionProfileId) -> Result<()> {
        self.query(r#"DELETE FROM mobile_connection_profiles WHERE id = ?"#)
            .bind(id.0.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_runtime_settings_document(&self) -> Result<Option<RuntimeSettingsDocument>> {
        let row = self
            .query(
                r#"SELECT id, schema_version, settings_json, secret_ref, updated_at
               FROM runtime_settings
               WHERE id = ?"#,
            )
            .bind("default")
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(RuntimeSettingsDocument {
            id: row.try_get("id")?,
            schema_version: row.try_get("schema_version")?,
            settings_json: row.try_get("settings_json")?,
            secret_ref: row.try_get("secret_ref")?,
            updated_at: parse_dt(&row.try_get::<String, _>("updated_at")?)?,
        }))
    }

    pub async fn upsert_runtime_settings_document(
        &self,
        schema_version: i64,
        settings_json: &str,
    ) -> Result<RuntimeSettingsDocument> {
        let old_secret_ref = self.lookup_runtime_settings_secret_ref().await?;
        let updated_at = Utc::now().to_rfc3339();
        self.query(
            r#"INSERT INTO runtime_settings (id, schema_version, settings_json, secret_ref, updated_at)
               VALUES (?, ?, ?, NULL, ?)
               ON CONFLICT(id) DO UPDATE SET
                   schema_version = excluded.schema_version,
                   settings_json = excluded.settings_json,
                   secret_ref = excluded.secret_ref,
                   updated_at = excluded.updated_at"#,
        )
        .bind("default")
        .bind(schema_version)
        .bind(settings_json)
        .bind(&updated_at)
        .execute(&self.pool)
        .await?;
        if let Some(old_secret_ref) = old_secret_ref {
            self.remove_runtime_settings_secrets_if_present(&old_secret_ref)
                .await?;
        }

        self.get_runtime_settings_document()
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back runtime settings"))
    }

    pub async fn upsert_runtime_settings_document_with_secrets(
        &self,
        schema_version: i64,
        settings_json: &str,
        settings_secret_json: &str,
    ) -> Result<RuntimeSettingsDocument> {
        let old_secret_ref = self.lookup_runtime_settings_secret_ref().await?;
        let new_secret_ref = Self::next_runtime_settings_secret_ref();
        self.write_runtime_settings_secrets(&new_secret_ref, settings_secret_json)
            .await?;
        let updated_at = Utc::now().to_rfc3339();
        let upsert_result = self
            .query(
                r#"INSERT INTO runtime_settings (id, schema_version, settings_json, secret_ref, updated_at)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   schema_version = excluded.schema_version,
                   settings_json = excluded.settings_json,
                   secret_ref = excluded.secret_ref,
                   updated_at = excluded.updated_at"#,
            )
            .bind("default")
            .bind(schema_version)
            .bind(settings_json)
            .bind(&new_secret_ref)
            .bind(&updated_at)
            .execute(&self.pool)
            .await;
        if let Err(err) = upsert_result {
            let _ = self
                .remove_runtime_settings_secrets_if_present(&new_secret_ref)
                .await;
            return Err(err.into());
        }
        if let Some(old_secret_ref) = old_secret_ref {
            if old_secret_ref != new_secret_ref {
                self.remove_runtime_settings_secrets_if_present(&old_secret_ref)
                    .await?;
            }
        }

        self.get_runtime_settings_document()
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back runtime settings"))
    }

    pub async fn get_mobile_access_config(&self) -> Result<Option<MobileAccessConfig>> {
        let row = self
            .query(
                r#"SELECT id, profile_id, tunnel_id, public_base_url, relay_base_url, secret_ref, tunnel_secret,
                      daemon_public_key, daemon_private_key, enabled, created_at, updated_at
               FROM mobile_access_config
               WHERE id = ?"#,
            )
            .bind("default")
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let id: String = row.try_get("id")?;
        let secret_ref: Option<String> = row.try_get("secret_ref")?;
        let tunnel_secret: String = row.try_get("tunnel_secret")?;
        let daemon_private_key: String = row.try_get("daemon_private_key")?;
        let (tunnel_secret, daemon_private_key) = match secret_ref {
            Some(secret_ref) => match self
                .read_mobile_access_secrets_if_present(&secret_ref)
                .await?
            {
                Some(secrets) => {
                    if !tunnel_secret.is_empty() || !daemon_private_key.is_empty() {
                        self.clear_legacy_mobile_access_secrets(&id).await?;
                    }
                    if id != secret_ref {
                        self.remove_mobile_access_secrets_if_present(&id).await?;
                    }
                    secrets
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "mobile access secrets are missing for config {} (secret_ref={})",
                        id,
                        secret_ref
                    ));
                }
            },
            None => {
                if let Some(secrets) = self.migrate_legacy_mobile_access_sidecar(&id).await? {
                    secrets
                } else {
                    self.migrate_legacy_mobile_access_secrets(
                        &id,
                        &tunnel_secret,
                        &daemon_private_key,
                    )
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "mobile access secrets are missing for config {} and no legacy secrets remain",
                            id
                        )
                    })?
                }
            }
        };
        Ok(Some(MobileAccessConfig {
            id,
            profile_id: ConnectionProfileId(uuid::Uuid::parse_str(
                &row.try_get::<String, _>("profile_id")?,
            )?),
            tunnel_id: row.try_get("tunnel_id")?,
            public_base_url: row.try_get("public_base_url")?,
            relay_base_url: row.try_get("relay_base_url")?,
            tunnel_secret,
            daemon_public_key: row.try_get("daemon_public_key")?,
            daemon_private_key,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
            created_at: parse_dt(&row.try_get::<String, _>("created_at")?)?,
            updated_at: parse_dt(&row.try_get::<String, _>("updated_at")?)?,
        }))
    }

    pub async fn upsert_mobile_access_config(
        &self,
        config: MobileAccessConfig,
    ) -> Result<MobileAccessConfig> {
        let old_secret_ref = self.lookup_mobile_access_secret_ref(&config.id).await?;
        let new_secret_ref = Self::next_mobile_access_secret_ref();
        self.write_mobile_access_secrets(
            &new_secret_ref,
            &config.tunnel_secret,
            &config.daemon_private_key,
        )
        .await?;
        let created_at = config.created_at.to_rfc3339();
        let updated_at = Utc::now().to_rfc3339();
        let upsert_result = self
            .query(
                r#"INSERT INTO mobile_access_config
                    (id, profile_id, tunnel_id, public_base_url, relay_base_url, secret_ref, tunnel_secret, daemon_public_key, daemon_private_key, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(id) DO UPDATE SET
                        profile_id=excluded.profile_id,
                        tunnel_id=excluded.tunnel_id,
                        public_base_url=excluded.public_base_url,
                        relay_base_url=excluded.relay_base_url,
                        secret_ref=excluded.secret_ref,
                        tunnel_secret=excluded.tunnel_secret,
                        daemon_public_key=excluded.daemon_public_key,
                        daemon_private_key=excluded.daemon_private_key,
                        enabled=excluded.enabled,
                        updated_at=excluded.updated_at"#,
            )
            .bind(&config.id)
            .bind(config.profile_id.0.to_string())
            .bind(&config.tunnel_id)
            .bind(&config.public_base_url)
            .bind(&config.relay_base_url)
            .bind(&new_secret_ref)
            .bind("")
            .bind(&config.daemon_public_key)
            .bind("")
            .bind(if config.enabled { 1 } else { 0 })
            .bind(created_at)
            .bind(updated_at)
            .execute(&self.pool)
            .await;
        if let Err(err) = upsert_result {
            let _ = self
                .remove_mobile_access_secrets_if_present(&new_secret_ref)
                .await;
            return Err(err.into());
        }
        if let Some(old_secret_ref) = old_secret_ref {
            if old_secret_ref != new_secret_ref {
                self.remove_mobile_access_secrets_if_present(&old_secret_ref)
                    .await?;
            }
        }
        if config.id != new_secret_ref {
            self.remove_mobile_access_secrets_if_present(&config.id)
                .await?;
        }

        self.get_mobile_access_config()
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back mobile access config"))
    }

    pub async fn set_mobile_access_enabled(&self, enabled: bool) -> Result<()> {
        self.query(r#"UPDATE mobile_access_config SET enabled = ?, updated_at = ? WHERE id = ?"#)
            .bind(if enabled { 1 } else { 0 })
            .bind(Utc::now().to_rfc3339())
            .bind("default")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_mobile_access_config(&self) -> Result<()> {
        let secret_ref = self.lookup_mobile_access_secret_ref("default").await?;
        self.query(r#"DELETE FROM mobile_access_config WHERE id = ?"#)
            .bind("default")
            .execute(&self.pool)
            .await?;
        if let Some(secret_ref) = secret_ref {
            self.remove_mobile_access_secrets_if_present(&secret_ref)
                .await?;
        }
        self.remove_mobile_access_secrets_if_present("default")
            .await?;
        Ok(())
    }

    pub async fn insert_mobile_pairing_token(
        &self,
        token_id: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        self.query(
            r#"INSERT INTO mobile_pairing_tokens
                (id, token_hash, created_at, expires_at)
               VALUES (?, ?, ?, ?)"#,
        )
        .bind(token_id)
        .bind(token_hash)
        .bind(Utc::now().to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_mobile_pairing_token(&self, token_hash: &str) -> Result<bool> {
        let row = self
            .query(r#"SELECT id, expires_at FROM mobile_pairing_tokens WHERE token_hash = ?"#)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(false);
        };
        let expires_at: String = row.try_get("expires_at")?;
        let expires_at = parse_dt(&expires_at)?;
        if expires_at < Utc::now() {
            let _ = self
                .query(r#"DELETE FROM mobile_pairing_tokens WHERE token_hash = ?"#)
                .bind(token_hash)
                .execute(&self.pool)
                .await;
            return Ok(false);
        }

        self.query(r#"DELETE FROM mobile_pairing_tokens WHERE token_hash = ?"#)
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(true)
    }

    pub async fn clear_mobile_pairing_tokens(&self) -> Result<()> {
        self.query(r#"DELETE FROM mobile_pairing_tokens"#)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn advance_mobile_device_seq(
        &self,
        id: MobileDeviceId,
        seq: i64,
    ) -> Result<MobileDeviceSeqAdvance> {
        let _write_guard = self.write_gate.lock().await;
        let mut tx = self.pool.begin().await?;
        let row = self
            .query(r#"SELECT last_seen_seq FROM mobile_devices WHERE id = ?"#)
            .bind(id.0.to_string())
            .fetch_optional(&mut *tx)
            .await?;

        let Some(row) = row else {
            return Ok(MobileDeviceSeqAdvance::Missing);
        };

        let last_seen: Option<i64> = row.try_get("last_seen_seq").ok();
        if let Some(current) = last_seen {
            if seq <= current {
                return Ok(MobileDeviceSeqAdvance::Stale { current });
            }
        }
        self.query(
            r#"UPDATE mobile_devices
               SET last_seen_seq = ?, last_seen_at = ?
               WHERE id = ?"#,
        )
        .bind(seq)
        .bind(Utc::now().to_rfc3339())
        .bind(id.0.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MobileDeviceSeqAdvance::Advanced)
    }

    pub async fn upsert_mobile_device(
        &self,
        id: MobileDeviceId,
        profile_id: ConnectionProfileId,
        update: MobileDeviceUpsert,
    ) -> Result<MobileDeviceRegistration> {
        let MobileDeviceUpsert {
            device_label,
            platform,
            push_token,
            push_provider,
            public_key,
            app_version,
        } = update;
        let now = Utc::now();
        self.query(
            r#"INSERT INTO mobile_devices
                (id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    device_label=excluded.device_label,
                    platform=excluded.platform,
                    push_token=excluded.push_token,
                    push_provider=excluded.push_provider,
                    public_key=excluded.public_key,
                    app_version=excluded.app_version,
                    last_seen_at=excluded.last_seen_at"#,
        )
        .bind(id.0.to_string())
        .bind(profile_id.0.to_string())
        .bind(device_label.clone())
        .bind(platform.clone())
        .bind(push_token.clone())
        .bind(push_provider.clone())
        .bind(public_key.clone())
        .bind(app_version.clone())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.get_mobile_device(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("failed to read back mobile device {}", id.0))
    }

    pub async fn get_mobile_device(
        &self,
        id: MobileDeviceId,
    ) -> Result<Option<MobileDeviceRegistration>> {
        let row = self.query(
            r#"SELECT id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at
               FROM mobile_devices WHERE id = ?"#,
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(build_mobile_device_from_row).transpose()
    }

    pub async fn list_mobile_devices(
        &self,
        profile_id: ConnectionProfileId,
    ) -> Result<Vec<MobileDeviceRegistration>> {
        let rows = self.query(
            r#"SELECT id, profile_id, device_label, platform, push_token, push_provider, public_key, app_version, created_at, last_seen_at
               FROM mobile_devices WHERE profile_id = ? ORDER BY created_at DESC"#,
        )
        .bind(profile_id.0.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(build_mobile_device_from_row(row)?);
        }
        Ok(out)
    }
}
