use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ARTIFACT_IDENTITY_FILENAME: &str = "artifact_identity.json";
const BUILD_IDENTITY_PATH_ENV: &str = "CTX_BUILD_IDENTITY_PATH";
const BUNDLE_DIR_ENV: &str = "CTX_BUNDLE_DIR";

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
    let version = env!("CARGO_PKG_VERSION").to_string();
    BuildIdentity {
        schema_version: 1,
        exact_version: version.clone(),
        build_id: option_env!("CTX_BUILD_ID")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string(),
        compatibility_token: option_env!("CTX_DEV_INSTANCE_ID")
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
    let bundle_dir = std::env::var(BUNDLE_DIR_ENV).ok();
    let Some(raw) = bundle_dir else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{BUNDLE_DIR_ENV} must not be empty when set");
    }
    Ok(Some(PathBuf::from(trimmed).join(ARTIFACT_IDENTITY_FILENAME)))
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
        anyhow::bail!("build identity missing compatibilityToken in {}", path.display());
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
    use super::parse_build_identity;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
  "exactVersion": "0.58.11-canary.deadbeefcafe",
  "buildId": "deadbeefcafe",
  "compatibilityToken": "artifact-deadbeefcafebabefeedface1234567890abcdef",
  "channel": "canary",
  "sourceCommit": "deadbeefcafebabefeedface1234567890abcdef",
  "mode": "release",
  "checkedInVersion": "0.58.11"
}
"#,
        )
        .expect("write identity");
        let identity = parse_build_identity(&path).expect("parse identity");
        assert_eq!(identity.exact_version, "0.58.11-canary.deadbeefcafe");
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
  "exactVersion": "0.58.11",
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
}
