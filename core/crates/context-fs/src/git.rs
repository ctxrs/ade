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
