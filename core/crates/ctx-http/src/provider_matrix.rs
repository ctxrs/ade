#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
use semver::Version;

mod status;

pub use ctx_provider_matrix::{
    builtin_matrix, extract_version, get_entry, invalidate_matrix_cache, is_user_facing_harness_id,
    latest_release, load_matrix, load_matrix_cached, matrix_cache_path, normalize_version,
    parse_version_loose, recommended_release, release_for_version, release_matches_context,
    select_latest_release, version_matches, DependencyInstall, ProviderArchiveKind,
    ProviderArchiveTarget, ProviderCommand, ProviderDependency, ProviderInstall,
    ProviderInstallDependency, ProviderInstallDependencyRole, ProviderInstallDependencyTarget,
    ProviderMatrix, ProviderMatrixCache, ProviderMatrixEntry, ProviderMatrixEntryKind,
    ProviderRelease, ProviderReleaseProvenance, ProviderReleaseStatus, VersionProbe,
};
pub use status::apply_matrix_to_status;

#[cfg(test)]
use ctx_provider_matrix::save_cached_matrix;
#[cfg(test)]
use status::{managed_dependency_update_available, probe_node_package_version};
#[cfg(test)]
mod tests;

#[cfg(test)]
const MATRIX_SCHEMA_VERSION: u32 = 2;

pub fn is_managed_supported(matrix: &ProviderMatrix, provider_id: &str) -> bool {
    let context_version = crate::updates::normalize_version_str(env!("CARGO_PKG_VERSION"));
    ctx_provider_matrix::is_managed_supported_for_context(
        matrix,
        provider_id,
        context_version.as_ref(),
    )
}
