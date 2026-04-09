use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::timeout;

use crate::installer::AgentServerConfigFile;
use crate::updates;

mod cache;
mod query;
mod status;

pub use cache::{
    builtin_matrix, invalidate_matrix_cache, load_matrix, load_matrix_cached, matrix_cache_path,
};
pub use query::{
    get_entry, is_managed_supported, is_user_facing_harness_id, latest_release, normalize_version,
    recommended_release, release_for_version,
};
pub use status::apply_matrix_to_status;

#[cfg(test)]
use cache::save_cached_matrix;
use query::release_matches_context;
#[cfg(test)]
use query::{parse_version_loose, select_latest_release, version_matches};
#[cfg(test)]
use status::{managed_dependency_update_available, probe_node_package_version};

const MATRIX_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const MATRIX_CACHE_FILENAME: &str = "provider_matrix.json";
const MATRIX_SCHEMA_VERSION: u32 = 2;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMatrix {
    pub version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderMatrixEntry>,
}

impl Default for ProviderMatrix {
    fn default() -> Self {
        serde_json::from_str(ctx_provider_accounts::PROVIDER_MATRIX_JSON).unwrap_or(
            ProviderMatrix {
                version: MATRIX_SCHEMA_VERSION,
                generated_at: None,
                providers: vec![],
            },
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMatrixEntry {
    pub id: String,
    #[serde(default)]
    pub kind: ProviderMatrixEntryKind,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub command: Option<ProviderCommand>,
    #[serde(default, rename = "managed_install")]
    pub managed_install: Option<ProviderInstall>,
    #[serde(default)]
    pub provider_dependencies: Vec<ProviderInstallDependency>,
    #[serde(default)]
    pub dependencies: Vec<ProviderDependency>,
    #[serde(default)]
    pub version_probe: Option<VersionProbe>,
    #[serde(default)]
    pub releases: Vec<ProviderRelease>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMatrixEntryKind {
    #[default]
    Harness,
    Dependency,
}

impl ProviderMatrixEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Dependency => "dependency",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderInstall {
    Npm {
        package: String,
        entrypoint: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Archive {
        version: String,
        #[serde(default)]
        args: Vec<String>,
        targets: HashMap<String, ProviderArchiveTarget>,
    },
    Python {
        package: String,
        version: String,
        entrypoint: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        python_version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        python_build_tag: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInstallDependency {
    pub id: String,
    #[serde(default)]
    pub role: ProviderInstallDependencyRole,
    #[serde(default)]
    pub target: ProviderInstallDependencyTarget,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderInstallDependencyRole {
    Prerequisite,
    #[default]
    Readiness,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderInstallDependencyTarget {
    #[default]
    SameAsProvider,
    Host,
    Container,
    LinuxAarch64,
    LinuxX8664,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDependency {
    pub id: String,
    #[serde(rename = "install")]
    pub install: DependencyInstall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyInstall {
    Npm {
        package: String,
        version: String,
    },
    Archive {
        version: String,
        targets: HashMap<String, ProviderArchiveTarget>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderArchiveTarget {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    pub archive: ProviderArchiveKind,
    pub bin_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderArchiveKind {
    None,
    TarGz,
    TarBz2,
    Zip,
    Dmg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionProbe {
    Command { args: Vec<String> },
    NodePackage { package: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRelease {
    pub version: String,
    #[serde(default)]
    pub status: ProviderReleaseStatus,
    #[serde(default)]
    pub upstream_version: Option<String>,
    #[serde(default)]
    pub provenance: Option<ProviderReleaseProvenance>,
    #[serde(default)]
    pub context_min: Option<String>,
    #[serde(default)]
    pub context_max: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderReleaseProvenance {
    #[serde(default)]
    pub upstream_repo: Option<String>,
    #[serde(default)]
    pub upstream_release_tag: Option<String>,
    #[serde(default)]
    pub upstream_commit_sha: Option<String>,
    #[serde(default)]
    pub ctx_repo: Option<String>,
    #[serde(default)]
    pub ctx_release_tag: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReleaseStatus {
    #[default]
    Supported,
    Blocked,
    Deprecated,
}

#[derive(Debug, Default)]
pub struct ProviderMatrixCache {
    pub cached_at: Option<Instant>,
    pub matrix: Option<ProviderMatrix>,
}

#[cfg(test)]
mod tests;
