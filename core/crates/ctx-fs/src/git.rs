use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum ApplyPatchTarget {
    Worktree,
    Index,
}

pub async fn assert_git_repo(root_path: impl AsRef<Path>) -> Result<()> {
    let root = root_path.as_ref();
    if !root.join(".git").exists() {
        bail!("workspace at {} is not a git repo", root.display());
    }
    Ok(())
}

pub async fn rev_parse_head(root_path: impl AsRef<Path>) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("rev-parse")
        .arg("HEAD")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse HEAD")?;
    if !output.status.success() {
        bail!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn rev_parse_ref(root_path: impl AsRef<Path>, reference: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("rev-parse")
        .arg(reference)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("running git rev-parse {reference}"))?;
    if !output.status.success() {
        bail!(
            "git rev-parse {reference} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn git_merge_base(root_path: impl AsRef<Path>, a: &str, b: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("merge-base")
        .arg(a)
        .arg(b)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git merge-base")?;
    if !output.status.success() {
        bail!(
            "git merge-base failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn git_is_ancestor(
    root_path: impl AsRef<Path>,
    ancestor: &str,
    descendant: &str,
) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(ancestor)
        .arg(descendant)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git merge-base --is-ancestor")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "git merge-base --is-ancestor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

pub async fn delete_branch(root_path: impl AsRef<Path>, branch: &str) -> Result<()> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Ok(());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref")?;
    if output.status.success() {
        let output = Command::new("git")
            .arg("-C")
            .arg(root_path.as_ref())
            .args(["branch", "-D", branch])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("running git branch -D")?;
        if !output.status.success() {
            bail!(
                "git branch -D failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(());
    }
    if output.status.code() == Some(1) {
        return Ok(());
    }
    bail!(
        "git show-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

pub async fn git_diff(root_path: impl AsRef<Path>, base_commit_sha: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg(base_commit_sha)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff")?;
    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn git_diff_numstat(
    root_path: impl AsRef<Path>,
    base_commit_sha: &str,
) -> Result<Vec<(i64, i64, String)>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg("--numstat")
        .arg("-z")
        .arg(base_commit_sha)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff --numstat")?;
    if !output.status.success() {
        bail!(
            "git diff --numstat failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = output.stdout;
    let mut out = Vec::new();
    let mut parts = bytes.split(|b| *b == 0);
    loop {
        let Some(add) = parts.next() else {
            break;
        };
        if add.is_empty() {
            continue;
        }
        let Some(del) = parts.next() else {
            break;
        };
        let Some(path) = parts.next() else {
            break;
        };
        let add = String::from_utf8_lossy(add);
        let del = String::from_utf8_lossy(del);
        let path = String::from_utf8_lossy(path).to_string();
        if path.trim().is_empty() {
            continue;
        }
        let add_count = add.parse::<i64>().unwrap_or(0);
        let del_count = del.parse::<i64>().unwrap_or(0);
        out.push((add_count, del_count, path));
    }
    Ok(out)
}

pub async fn git_diff_unstaged(root_path: impl AsRef<Path>) -> Result<String> {
    // Shows worktree changes relative to the index (used for edit review; staging acts as "accept").
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff (unstaged)")?;
    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn git_diff_numstat_unstaged(
    root_path: impl AsRef<Path>,
) -> Result<Vec<(i64, i64, String)>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg("--numstat")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff --numstat")?;
    if !output.status.success() {
        bail!(
            "git diff --numstat failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let add = parts.next().unwrap_or("0");
        let del = parts.next().unwrap_or("0");
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let add_count = add.parse::<i64>().unwrap_or(0);
        let del_count = del.parse::<i64>().unwrap_or(0);
        out.push((add_count, del_count, path.to_string()));
    }
    Ok(out)
}

pub async fn git_status_short(root_path: impl AsRef<Path>) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("status")
        .arg("-sb")
        .arg("--untracked-files=all")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git status -sb")?;
    if !output.status.success() {
        bail!(
            "git status -sb failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn list_untracked_files(root_path: impl AsRef<Path>) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("ls-files")
        .arg("--others")
        .arg("--exclude-standard")
        .arg("-z")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git ls-files --others")?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = output.stdout;
    let mut out = Vec::new();
    for part in bytes.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(part).to_string());
    }
    Ok(out)
}

pub async fn git_status_porcelain(root_path: impl AsRef<Path>) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git status --porcelain")?;
    if !output.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = output.stdout;
    let mut out = Vec::new();
    for entry in bytes.split(|b| *b == 0) {
        if entry.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(entry).to_string());
    }
    Ok(out)
}

pub(crate) async fn branch_exists(
    workspace_root: impl AsRef<Path>,
    branch_name: &str,
) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("show-ref")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("refs/heads/{branch_name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref --verify")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "git show-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(crate) async fn is_git_worktree(worktree_path: impl AsRef<Path>) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path.as_ref())
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse --is-inside-work-tree")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

pub async fn list_tracked_files(root_path: impl AsRef<Path>) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("ls-files")
        .arg("-z")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git ls-files")?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = output.stdout;
    let mut out = Vec::new();
    for part in bytes.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(part).to_string());
    }
    Ok(out)
}

pub async fn git_diff_untracked_file(
    root_path: impl AsRef<Path>,
    rel_path: &str,
) -> Result<String> {
    // `git diff --no-index` uses exit code 1 to indicate differences; treat 0/1 as success.
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg("/dev/null")
        .arg(rel_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff --no-index for untracked file")?;

    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "git diff --no-index failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn git_apply_patch(
    root_path: impl AsRef<Path>,
    patch: &str,
    target: ApplyPatchTarget,
    reverse: bool,
) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root_path.as_ref()).arg("apply");
    if matches!(target, ApplyPatchTarget::Index) {
        cmd.arg("--cached");
    }
    if reverse {
        cmd.arg("--reverse");
    }
    cmd.arg("--whitespace=nowarn").arg("-");

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning git apply")?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(patch.as_bytes())
            .await
            .context("writing patch to git apply stdin")?;
    }

    let output = child
        .wait_with_output()
        .await
        .context("waiting for git apply")?;
    if !output.status.success() {
        bail!(
            "git apply failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub async fn git_apply_patch_allow_noop(
    root_path: impl AsRef<Path>,
    patch: &str,
    target: ApplyPatchTarget,
    reverse: bool,
) -> Result<()> {
    match git_apply_patch(root_path, patch, target, reverse).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort for index updates: it's fine if nothing was staged.
            let msg = e.to_string().to_lowercase();
            if msg.contains("patch does not apply") || msg.contains("did not match any files") {
                return Ok(());
            }
            Err(e)
        }
    }
}
