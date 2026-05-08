use std::path::Path;

pub(crate) async fn path_resolves_within_root(path: &Path, root: &Path) -> bool {
    let Ok(canonical_path) = tokio::fs::canonicalize(path).await else {
        return false;
    };
    let Ok(canonical_root) = tokio::fs::canonicalize(root).await else {
        return false;
    };
    canonical_path.starts_with(&canonical_root)
}
