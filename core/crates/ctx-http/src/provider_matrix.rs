#[cfg(test)]
use std::collections::HashMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::Context;
#[cfg(test)]
use semver::Version;

#[cfg(test)]
mod status;

pub(crate) use ctx_provider_matrix::{
    get_entry, invalidate_matrix_cache, is_user_facing_harness_id, load_bundled_matrix_from_env,
    load_cached_matrix, load_explicit_matrix_from_env, load_matrix_cached, replace_matrix_cache,
    ProviderMatrix, ProviderMatrixCache, ProviderMatrixEntryKind,
};

#[cfg(test)]
pub(crate) use ctx_provider_matrix::load_matrix;

#[cfg(test)]
pub(crate) use ctx_provider_matrix::{
    builtin_matrix, extract_version, latest_release, normalize_version, parse_version_loose,
    recommended_release, release_for_version, release_matches_context, select_latest_release,
    version_matches, DependencyInstall, ProviderArchiveKind, ProviderArchiveTarget,
    ProviderCommand, ProviderDependency, ProviderInstall, ProviderInstallDependencyRole,
    ProviderInstallDependencyTarget, ProviderMatrixEntry, ProviderRelease, ProviderReleaseStatus,
    VersionProbe,
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

#[derive(Debug, Clone)]
pub(crate) struct MatrixRefreshOutcome {
    pub matrix: ProviderMatrix,
    pub source: MatrixRefreshSource,
    pub degraded: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatrixRefreshSource {
    Remote,
    Cached,
    Bundled,
    Builtin,
    Explicit,
}

impl MatrixRefreshSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Cached => "cached",
            Self::Bundled => "bundled",
            Self::Builtin => "builtin",
            Self::Explicit => "explicit",
        }
    }
}

fn provider_matrix_channel() -> String {
    std::env::var("CTX_PROVIDER_MATRIX_CHANNEL")
        .ok()
        .or_else(|| std::env::var("CTX_DESKTOP_CHANNEL").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "stable".to_string())
}

fn provider_matrix_base_url() -> String {
    std::env::var("CTX_PROVIDER_MATRIX_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://api.ctx.rs/functions/v1".to_string())
}

fn explicit_provider_matrix_override_enabled() -> bool {
    std::env::var("CTX_BUNDLE_MATRIX_JSON")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) async fn refresh_matrix_from_remote_or_fallback(
    data_root: &Path,
    cache: &tokio::sync::Mutex<ProviderMatrixCache>,
) -> MatrixRefreshOutcome {
    if explicit_provider_matrix_override_enabled() {
        return match load_explicit_matrix_from_env() {
            Ok(Some(matrix)) => {
                replace_matrix_cache(cache, matrix.clone()).await;
                MatrixRefreshOutcome {
                    matrix,
                    source: MatrixRefreshSource::Explicit,
                    degraded: false,
                    last_error: None,
                }
            }
            Ok(None) => {
                fallback_matrix_outcome(
                    data_root,
                    cache,
                    "explicit provider matrix override is empty",
                )
                .await
            }
            Err(err) => fallback_matrix_outcome(data_root, cache, err.to_string()).await,
        };
    }

    let channel = provider_matrix_channel();
    let base_url = provider_matrix_base_url();
    let url = format!(
        "{}/provider-matrix/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    match fetch_provider_matrix(&url).await {
        Ok(matrix) => {
            if let Err(err) = ctx_provider_matrix::save_cached_matrix(data_root, &matrix).await {
                eprintln!("warn: failed to cache remote provider matrix: {err:#}");
            }
            replace_matrix_cache(cache, matrix.clone()).await;
            MatrixRefreshOutcome {
                matrix,
                source: MatrixRefreshSource::Remote,
                degraded: false,
                last_error: None,
            }
        }
        Err(err) => fallback_matrix_outcome(data_root, cache, err.to_string()).await,
    }
}

async fn fallback_matrix_outcome(
    data_root: &Path,
    cache: &tokio::sync::Mutex<ProviderMatrixCache>,
    last_error: impl Into<String>,
) -> MatrixRefreshOutcome {
    if let Some(matrix) = load_cached_matrix(data_root) {
        replace_matrix_cache(cache, matrix.clone()).await;
        return MatrixRefreshOutcome {
            matrix,
            source: MatrixRefreshSource::Cached,
            degraded: true,
            last_error: Some(last_error.into()),
        };
    }
    if let Some(matrix) = load_bundled_matrix_from_env() {
        replace_matrix_cache(cache, matrix.clone()).await;
        return MatrixRefreshOutcome {
            matrix,
            source: MatrixRefreshSource::Bundled,
            degraded: true,
            last_error: Some(last_error.into()),
        };
    }
    let matrix = ctx_provider_matrix::builtin_matrix();
    replace_matrix_cache(cache, matrix.clone()).await;
    MatrixRefreshOutcome {
        matrix,
        source: MatrixRefreshSource::Builtin,
        degraded: true,
        last_error: Some(last_error.into()),
    }
}

async fn fetch_provider_matrix(url: &str) -> anyhow::Result<ProviderMatrix> {
    let txt = reqwest::get(url)
        .await
        .with_context(|| format!("fetching provider matrix: {url}"))?
        .error_for_status()
        .with_context(|| format!("provider matrix http error: {url}"))?
        .text()
        .await
        .context("reading provider matrix body")?;
    let matrix: ProviderMatrix =
        serde_json::from_str(&txt).context("parsing provider matrix JSON")?;
    let expected_schema = ctx_provider_matrix::builtin_matrix().version;
    if matrix.version != expected_schema {
        anyhow::bail!(
            "provider matrix schema mismatch: expected {}, got {}",
            expected_schema,
            matrix.version
        );
    }
    Ok(matrix)
}
