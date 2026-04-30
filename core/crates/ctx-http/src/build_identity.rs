use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const BUILD_IDENTITY_PATH_ENV: &str = "CTX_BUILD_IDENTITY_PATH";

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct BuildIdentity {
    #[serde(rename = "schemaVersion")]
    pub(crate) schema_version: u32,
    #[serde(rename = "exactVersion")]
    pub(crate) exact_version: String,
    #[serde(rename = "buildId")]
    pub(crate) build_id: String,
    #[serde(rename = "compatibilityToken")]
    pub(crate) compatibility_token: String,
}

fn compile_time_build_identity() -> BuildIdentity {
    let version = option_env!("CTX_RELEASE_EFFECTIVE_VERSION")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();
    BuildIdentity {
        schema_version: 1,
        exact_version: version.clone(),
        build_id: option_env!("CTX_BUILD_ID")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string(),
        compatibility_token: option_env!("CTX_COMPATIBILITY_TOKEN")
            .or(option_env!("CTX_DEV_INSTANCE_ID"))
            .unwrap_or("unknown")
            .to_string(),
    }
}

fn configured_identity_path() -> Result<Option<PathBuf>> {
    let explicit = std::env::var(BUILD_IDENTITY_PATH_ENV).ok();
    if let Some(raw) = explicit {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{BUILD_IDENTITY_PATH_ENV} must not be empty");
        }
        return Ok(Some(PathBuf::from(trimmed)));
    }
    Ok(None)
}

fn parse_build_identity(path: &Path) -> Result<BuildIdentity> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading build identity {}", path.display()))?;
    let identity: BuildIdentity = serde_json::from_str(&raw)
        .with_context(|| format!("parsing build identity {}", path.display()))?;
    if identity.schema_version != 1 {
        anyhow::bail!(
            "unsupported build identity schema {} in {}",
            identity.schema_version,
            path.display()
        );
    }
    if identity.exact_version.trim().is_empty() {
        anyhow::bail!("build identity missing exactVersion in {}", path.display());
    }
    if identity.build_id.trim().is_empty() {
        anyhow::bail!("build identity missing buildId in {}", path.display());
    }
    if identity.compatibility_token.trim().is_empty() {
        anyhow::bail!(
            "build identity missing compatibilityToken in {}",
            path.display()
        );
    }
    Ok(identity)
}

fn load_build_identity() -> Result<BuildIdentity> {
    let Some(identity_path) = configured_identity_path()? else {
        return Ok(compile_time_build_identity());
    };
    parse_build_identity(&identity_path)
}

pub(crate) fn current_build_identity() -> Result<&'static BuildIdentity> {
    static BUILD_IDENTITY: OnceLock<Result<BuildIdentity, String>> = OnceLock::new();
    let entry = BUILD_IDENTITY.get_or_init(|| load_build_identity().map_err(|err| err.to_string()));
    match entry {
        Ok(identity) => Ok(identity),
        Err(err) => Err(anyhow!(err.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::{configured_identity_path, load_build_identity, parse_build_identity};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        build_identity_path: Option<String>,
        bundle_dir: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_LOCK.lock().expect("build identity env lock poisoned");
            let build_identity_path = std::env::var(super::BUILD_IDENTITY_PATH_ENV).ok();
            let bundle_dir = std::env::var("CTX_BUNDLE_DIR").ok();
            std::env::remove_var(super::BUILD_IDENTITY_PATH_ENV);
            std::env::remove_var("CTX_BUNDLE_DIR");
            Self {
                _lock: lock,
                build_identity_path,
                bundle_dir,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.build_identity_path.take() {
                std::env::set_var(super::BUILD_IDENTITY_PATH_ENV, value);
            } else {
                std::env::remove_var(super::BUILD_IDENTITY_PATH_ENV);
            }
            if let Some(value) = self.bundle_dir.take() {
                std::env::set_var("CTX_BUNDLE_DIR", value);
            } else {
                std::env::remove_var("CTX_BUNDLE_DIR");
            }
        }
    }

    fn temp_identity_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ctx-build-identity-{name}-{unique}.json"))
    }

    #[test]
    fn parse_build_identity_accepts_valid_manifest() {
        let path = temp_identity_path("valid");
        fs::write(
            &path,
            r#"{
  "schemaVersion": 1,
  "exactVersion": "0.59.0-canary.deadbeefcafe",
  "buildId": "deadbeefcafe",
  "compatibilityToken": "artifact-deadbeefcafebabefeedface1234567890abcdef",
  "channel": "canary",
  "sourceCommit": "deadbeefcafebabefeedface1234567890abcdef",
  "mode": "release",
  "checkedInVersion": "0.59.0"
}
"#,
        )
        .expect("write identity");
        let identity = parse_build_identity(&path).expect("parse identity");
        assert_eq!(identity.exact_version, "0.59.0-canary.deadbeefcafe");
        assert_eq!(identity.build_id, "deadbeefcafe");
        assert_eq!(
            identity.compatibility_token,
            "artifact-deadbeefcafebabefeedface1234567890abcdef"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn parse_build_identity_rejects_missing_build_id() {
        let path = temp_identity_path("missing-build-id");
        fs::write(
            &path,
            r#"{
  "schemaVersion": 1,
  "exactVersion": "0.59.0",
  "buildId": "",
  "compatibilityToken": "artifact-localpkg123"
}
"#,
        )
        .expect("write identity");
        let error = parse_build_identity(&path).expect_err("missing buildId should fail");
        assert!(error.to_string().contains("missing buildId"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn build_identity_does_not_implicitly_follow_bundle_metadata() {
        let _env = EnvGuard::new();
        let bundle_dir = tempfile::tempdir().expect("bundle tempdir");
        fs::write(
            bundle_dir.path().join("artifact_identity.json"),
            r#"{
  "schemaVersion": 1,
  "exactVersion": "9.9.9-preview.bundle",
  "buildId": "bundle-build",
  "compatibilityToken": "artifact-bundle"
}
"#,
        )
        .expect("write bundle identity");
        std::env::set_var("CTX_BUNDLE_DIR", bundle_dir.path());

        assert!(
            configured_identity_path()
                .expect("resolve identity path")
                .is_none(),
            "daemon build identity must not be inferred from CTX_BUNDLE_DIR"
        );
        let identity = load_build_identity().expect("load compile-time identity");
        assert_ne!(identity.exact_version, "9.9.9-preview.bundle");
        assert_ne!(identity.build_id, "bundle-build");
        assert_ne!(identity.compatibility_token, "artifact-bundle");
    }

    #[test]
    fn build_identity_allows_explicit_identity_path_override() {
        let _env = EnvGuard::new();
        let path = temp_identity_path("explicit-path");
        fs::write(
            &path,
            r#"{
  "schemaVersion": 1,
  "exactVersion": "1.2.3-explicit",
  "buildId": "explicit-build",
  "compatibilityToken": "artifact-explicit"
}
"#,
        )
        .expect("write identity");
        std::env::set_var(super::BUILD_IDENTITY_PATH_ENV, &path);

        let identity = load_build_identity().expect("load explicit identity");
        assert_eq!(identity.exact_version, "1.2.3-explicit");
        assert_eq!(identity.build_id, "explicit-build");
        assert_eq!(identity.compatibility_token, "artifact-explicit");
        let _ = fs::remove_file(path);
    }
}
