use super::*;

pub fn matrix_cache_path(data_root: &Path) -> PathBuf {
    data_root.join("providers").join(MATRIX_CACHE_FILENAME)
}

pub fn builtin_matrix() -> ProviderMatrix {
    ProviderMatrix::default()
}

pub async fn load_matrix(data_root: &Path) -> ProviderMatrix {
    if let Some(matrix) = load_cached_matrix(data_root) {
        return matrix;
    }
    builtin_matrix()
}

pub async fn load_matrix_cached(
    data_root: &Path,
    cache: &tokio::sync::Mutex<ProviderMatrixCache>,
) -> ProviderMatrix {
    let cached = {
        let guard = cache.lock().await;
        if let Some(at) = guard.cached_at {
            if at.elapsed() < MATRIX_CACHE_TTL {
                guard.matrix.clone()
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(matrix) = cached {
        return matrix;
    }

    let matrix = load_matrix(data_root).await;
    let mut guard = cache.lock().await;
    guard.cached_at = Some(Instant::now());
    guard.matrix = Some(matrix.clone());
    matrix
}

pub async fn invalidate_matrix_cache(cache: &tokio::sync::Mutex<ProviderMatrixCache>) {
    let mut guard = cache.lock().await;
    guard.cached_at = None;
    guard.matrix = None;
}

pub(crate) fn load_cached_matrix(data_root: &Path) -> Option<ProviderMatrix> {
    let path = matrix_cache_path(data_root);
    let txt = std::fs::read_to_string(&path).ok()?;
    let parsed: ProviderMatrix = serde_json::from_str(&txt).ok()?;
    if parsed.version != MATRIX_SCHEMA_VERSION {
        return None;
    }
    Some(parsed)
}

#[cfg(test)]
pub async fn save_cached_matrix(
    data_root: &Path,
    matrix: &ProviderMatrix,
) -> anyhow::Result<()> {
    use anyhow::Context;

    let path = matrix_cache_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let txt = serde_json::to_string_pretty(matrix).context("serializing provider matrix")?;
    tokio::fs::write(&path, txt)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
