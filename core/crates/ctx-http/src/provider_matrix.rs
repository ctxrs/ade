#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
use semver::Version;

#[cfg(test)]
mod status;

pub(crate) use ctx_provider_matrix::{
    get_entry, invalidate_matrix_cache, is_user_facing_harness_id, load_matrix_cached,
    ProviderMatrixCache, ProviderMatrixEntryKind,
};

#[cfg(test)]
pub(crate) use ctx_provider_matrix::{
    builtin_matrix, extract_version, latest_release, load_matrix, normalize_version,
    parse_version_loose, recommended_release, release_for_version, release_matches_context,
    select_latest_release, version_matches, DependencyInstall, ProviderArchiveKind,
    ProviderArchiveTarget, ProviderCommand, ProviderDependency, ProviderInstall,
    ProviderInstallDependencyRole, ProviderInstallDependencyTarget, ProviderMatrix,
    ProviderMatrixEntry, ProviderRelease, ProviderReleaseStatus, VersionProbe,
};
#[cfg(test)]
pub(crate) use status::apply_matrix_to_status;

#[cfg(test)]
use ctx_provider_matrix::save_cached_matrix;
#[cfg(test)]
use status::{managed_dependency_update_available, probe_node_package_version};
#[cfg(test)]
mod tests;

#[cfg(test)]
const MATRIX_SCHEMA_VERSION: u32 = 3;
