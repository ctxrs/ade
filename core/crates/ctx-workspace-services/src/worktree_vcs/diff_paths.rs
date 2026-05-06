use std::collections::HashSet;

use anyhow::Result;

pub fn count_diff_paths(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<i64> {
    let mut seen = HashSet::new();
    for (status, path, _) in entries {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.to_string()) {
            continue;
        }
        if status.chars().next().is_none() {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        }
    }
    for path in untracked {
        let path = path.trim();
        if !path.is_empty() {
            seen.insert(path.to_string());
        }
    }
    Ok(seen.len() as i64)
}

pub fn build_diff_path_states(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<Vec<(String, Option<String>, String)>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (status, path, orig_path) in entries {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(status_kind) = status.chars().next() else {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        };
        out.push((path, orig_path, status_kind.to_string()));
    }
    for path in untracked {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        out.push((path, None, "?".to_string()));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_diff_path_states_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
        let out = build_diff_path_states(
            vec![("D".to_string(), "src/example.rs".to_string(), None)],
            vec!["src/example.rs".to_string()],
        )?;

        assert_eq!(
            out,
            vec![("src/example.rs".to_string(), None, "D".to_string())]
        );
        Ok(())
    }

    #[test]
    fn count_diff_paths_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
        let count = count_diff_paths(
            vec![("D".to_string(), "src/example.rs".to_string(), None)],
            vec!["src/example.rs".to_string()],
        )?;

        assert_eq!(count, 1);
        Ok(())
    }
}
