use std::path::{Path, PathBuf};

pub(in crate::api::repo) fn expand_tilde(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| "could not resolve home directory to expand '~'".to_string())?;
        let home = base.home_dir();
        if raw == "~" {
            return Ok(home.to_path_buf());
        }
        return Ok(home.join(raw.trim_start_matches("~/")));
    }
    Ok(PathBuf::from(raw))
}

pub(in crate::api::repo) fn validate_absolute_path(path: &Path, field: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{field} must be an absolute path"));
    }
    Ok(())
}

pub(in crate::api::repo) fn derive_repo_name(repo_url: &str) -> Option<String> {
    let url = repo_url.trim().trim_end_matches('/');
    if url.is_empty() {
        return None;
    }
    // Handle `git@github.com:org/repo.git` style URLs by treating `:` as a path separator.
    let normalized = url.replace(':', "/");
    let last = normalized.split('/').next_back()?.trim();
    if last.is_empty() {
        return None;
    }
    let name = last.strip_suffix(".git").unwrap_or(last);
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub(in crate::api::repo) fn validate_dest_name(name: &str) -> Result<(), String> {
    let s = name.trim();
    if s.is_empty() {
        return Err("dest_name must be non-empty".to_string());
    }
    let p = Path::new(s);
    if p.is_absolute() {
        return Err("dest_name must be a single path segment".to_string());
    }
    let mut components = p.components();
    let first = components.next();
    if components.next().is_some() {
        return Err("dest_name must be a single path segment".to_string());
    }
    match first {
        Some(std::path::Component::Normal(_)) => Ok(()),
        _ => Err("dest_name must be a single path segment".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_repo_name_strips_git_and_handles_scp_style() {
        assert_eq!(
            derive_repo_name("git@github.com:org/repo.git"),
            Some("repo".to_string())
        );
        assert_eq!(
            derive_repo_name("https://github.com/org/repo"),
            Some("repo".to_string())
        );
        assert_eq!(derive_repo_name(""), None);
    }

    #[test]
    fn validate_absolute_path_rejects_relative() {
        let p = PathBuf::from("relative/path");
        assert!(validate_absolute_path(&p, "path").is_err());
    }
}
