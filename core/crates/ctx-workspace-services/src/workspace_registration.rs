use std::path::Path;

use anyhow::Context;
use ctx_core::models::VcsKind;
use ctx_fs::git::git_default_branch;
use ctx_fs::vcs;

pub async fn detect_workspace_primary_branch(
    vcs_kind: VcsKind,
    root_path: &Path,
    driver: &dyn vcs::VcsDriver,
) -> anyhow::Result<String> {
    match vcs_kind {
        VcsKind::Git => {
            let branch = git_default_branch(root_path)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unable to detect default git branch"))?;
            normalize_detected_primary_branch(&branch)
        }
        VcsKind::Jj => {
            driver
                .rev_parse_ref(root_path, "main")
                .await
                .context("resolving jj primary bookmark `main`")?;
            Ok("main".to_string())
        }
        _ => anyhow::bail!("primary branch detection is only supported for git and jj workspaces"),
    }
}

fn normalize_detected_primary_branch(branch: &str) -> anyhow::Result<String> {
    let trimmed = branch.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("detected default git branch is empty");
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::normalize_detected_primary_branch;

    #[test]
    fn normalize_detected_primary_branch_trims_git_output() {
        let branch = normalize_detected_primary_branch(" main \n").expect("branch");
        assert_eq!(branch, "main");
    }

    #[test]
    fn normalize_detected_primary_branch_rejects_empty_output() {
        let error = normalize_detected_primary_branch(" \n").expect_err("empty branch");
        assert_eq!(error.to_string(), "detected default git branch is empty");
    }
}
