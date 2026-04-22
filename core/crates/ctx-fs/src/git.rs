use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::vcs::{VcsStatusBranchInfo, VcsStatusEntry, VcsStructuredStatus};

#[derive(Debug, Clone, Copy)]
pub enum ApplyPatchTarget {
    Worktree,
    Index,
}

pub async fn assert_git_repo(root_path: impl AsRef<Path>) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse --is-inside-work-tree")?;
    if !output.status.success() {
        bail!(
            "not a git repository: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if String::from_utf8_lossy(&output.stdout).trim() != "true" {
        bail!("not a git repository");
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

pub async fn git_ref_exists(root_path: impl AsRef<Path>, reference: &str) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .args(["show-ref", "--verify", "--quiet", reference])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref")?;
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

pub async fn git_default_branch(root_path: impl AsRef<Path>) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .args(["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git symbolic-ref refs/remotes/origin/HEAD")?;
    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(stripped) = raw.strip_prefix("refs/remotes/") {
            if !stripped.trim().is_empty() {
                return Ok(Some(stripped.to_string()));
            }
        }
        if !raw.trim().is_empty() {
            return Ok(Some(raw));
        }
    }
    let head = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git symbolic-ref --short HEAD")?;
    if head.status.success() {
        let raw = String::from_utf8_lossy(&head.stdout).trim().to_string();
        if !raw.trim().is_empty() {
            return Ok(Some(raw));
        }
    }
    let candidates = [
        ("refs/remotes/origin/main", "origin/main"),
        ("refs/remotes/origin/master", "origin/master"),
        ("refs/heads/main", "main"),
        ("refs/heads/master", "master"),
    ];
    for (reference, branch) in candidates {
        if git_ref_exists(root_path.as_ref(), reference)
            .await
            .unwrap_or(false)
        {
            return Ok(Some(branch.to_string()));
        }
    }
    Ok(None)
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
    for entry in bytes.split(|b| *b == 0) {
        if entry.is_empty() {
            continue;
        }
        let mut fields = entry.splitn(3, |b| *b == b'\t');
        let add = fields.next().unwrap_or_default();
        let del = fields.next();
        let path = fields.next();
        let (Some(del), Some(path)) = (del, path) else {
            continue;
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

#[derive(Debug, Clone)]
pub struct GitNameStatusEntry {
    pub status: String,
    pub path: String,
    pub orig_path: Option<String>,
}

pub async fn git_diff_name_status(
    root_path: impl AsRef<Path>,
    base_commit_sha: &str,
) -> Result<Vec<GitNameStatusEntry>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg("--name-status")
        .arg("-z")
        .arg(base_commit_sha)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff --name-status")?;
    if !output.status.success() {
        bail!(
            "git diff --name-status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(parse_git_diff_name_status_bytes(&output.stdout)
        .into_iter()
        .map(|(status, path, orig_path)| GitNameStatusEntry {
            status,
            path,
            orig_path,
        })
        .collect())
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

pub async fn git_status_structured(
    root_path: impl AsRef<Path>,
    include_untracked_files: bool,
    include_entries: bool,
) -> Result<VcsStructuredStatus> {
    let untracked_mode = if include_untracked_files {
        "--untracked-files=all"
    } else {
        "--untracked-files=normal"
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path.as_ref())
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .arg("--branch")
        .arg(untracked_mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git status --porcelain -z --branch")?;
    if !output.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(git_status_structured_from_bytes_with_entries(
        &output.stdout,
        include_entries,
    ))
}

pub fn git_status_structured_from_bytes(bytes: &[u8]) -> VcsStructuredStatus {
    git_status_structured_from_bytes_with_entries(bytes, true)
}

pub fn git_status_structured_from_bytes_with_entries(
    bytes: &[u8],
    include_entries: bool,
) -> VcsStructuredStatus {
    parse_git_status_structured_bytes(bytes, include_entries)
}

pub async fn git_diff_name_status_paths(
    root_path: impl AsRef<Path>,
    base_revision: &str,
    paths: &[String],
) -> Result<Vec<(String, String, Option<String>)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root_path.as_ref())
        .arg("diff")
        .arg("--name-status")
        .arg("-z")
        .arg(base_revision)
        .arg("--");
    for path in paths {
        cmd.arg(path);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git diff --name-status -z -- <paths>")?;
    if !output.status.success() {
        bail!(
            "git diff --name-status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(parse_git_diff_name_status_bytes(&output.stdout))
}

fn parse_git_status_structured_bytes(bytes: &[u8], include_entries: bool) -> VcsStructuredStatus {
    let mut entries = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .peekable();
    let mut branch = VcsStatusBranchInfo::default();
    let mut parsed_entries = Vec::new();
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    let mut total_count = 0;
    if let Some(first) = entries.peek() {
        let first = String::from_utf8_lossy(first);
        if first.starts_with("## ") {
            branch = parse_git_status_branch_info(&first);
            entries.next();
        }
    }
    while let Some(raw_bytes) = entries.next() {
        let raw = String::from_utf8_lossy(raw_bytes);
        let raw = raw.trim_end();
        if raw.len() < 3 {
            continue;
        }
        let bytes = raw.as_bytes();
        if bytes[2] != b' ' {
            continue;
        }
        let index_status = raw.chars().next().unwrap_or(' ');
        let worktree_status = raw.chars().nth(1).unwrap_or(' ');
        let path = raw[3..].trim();
        if path.is_empty() {
            continue;
        }
        let mut orig_path = None;
        if let Some(next_bytes) = entries
            .peek()
            .copied()
            .filter(|_| index_status == 'R' || index_status == 'C')
        {
            let next = String::from_utf8_lossy(next_bytes);
            let next = next.trim_end();
            if !looks_like_porcelain_status(next) && !next.is_empty() {
                if include_entries {
                    orig_path = Some(next.to_string());
                }
                entries.next();
            }
        }
        total_count += 1;
        if index_status == '?' && worktree_status == '?' {
            untracked += 1;
        } else {
            if index_status != ' ' {
                staged += 1;
            }
            if worktree_status != ' ' {
                unstaged += 1;
            }
        }
        if include_entries {
            parsed_entries.push(VcsStatusEntry {
                path: path.to_string(),
                orig_path,
                index_status: index_status.to_string(),
                worktree_status: worktree_status.to_string(),
            });
        }
    }
    VcsStructuredStatus {
        raw: if include_entries {
            String::from_utf8_lossy(bytes).to_string()
        } else {
            String::new()
        },
        branch,
        entries: parsed_entries,
        staged,
        unstaged,
        untracked,
        total_count,
        truncated: false,
    }
}

fn parse_git_status_branch_info(line: &str) -> VcsStatusBranchInfo {
    let mut info = VcsStatusBranchInfo {
        summary_line: line.trim().to_string(),
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        detached: false,
    };
    let Some(mut line) = line.trim().strip_prefix("## ") else {
        return info;
    };
    let mut counts_part = None;
    if let Some(idx) = line.find(" [") {
        counts_part = Some(line[idx + 2..].trim());
        line = line[..idx].trim();
    }
    if line.starts_with("HEAD") {
        info.detached = true;
    }
    if let Some((local, upstream)) = line.split_once("...") {
        if !local.trim().is_empty() {
            info.branch = Some(local.trim().to_string());
        }
        if !upstream.trim().is_empty() {
            info.upstream = Some(upstream.trim().to_string());
        }
    } else if !line.trim().is_empty() && !info.detached {
        info.branch = Some(line.trim().to_string());
    }
    if let Some(mut counts) = counts_part {
        if counts.ends_with(']') {
            counts = &counts[..counts.len() - 1];
        }
        for part in counts.split(',') {
            let mut iter = part.split_whitespace();
            let Some(kind) = iter.next() else {
                continue;
            };
            let Some(value) = iter.next() else {
                continue;
            };
            let count = value.parse::<i64>().unwrap_or(0);
            match kind {
                "ahead" => info.ahead = count,
                "behind" => info.behind = count,
                _ => {}
            }
        }
    }
    info
}

fn looks_like_porcelain_status(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[2] == b' '
}

fn parse_git_diff_name_status_bytes(bytes: &[u8]) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    let mut parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .peekable();
    while let Some(part) = parts.next() {
        let status = part.trim().to_string();
        if status.is_empty() {
            continue;
        }
        let Some(path) = parts.next() else {
            continue;
        };
        if status.is_empty() || path.trim().is_empty() {
            continue;
        }
        let status_char = status.chars().next().unwrap_or('M');
        if status_char == 'R' || status_char == 'C' {
            let Some(next_path) = parts.next() else {
                continue;
            };
            let new_path = next_path;
            if new_path.trim().is_empty() {
                continue;
            }
            out.push((status, new_path, Some(path)));
        } else {
            out.push((status, path, None));
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::{
        git_status_structured_from_bytes, git_status_structured_from_bytes_with_entries,
        parse_git_diff_name_status_bytes,
    };

    #[test]
    fn parse_git_diff_name_status_handles_regular_entries() {
        let parsed = parse_git_diff_name_status_bytes(b"M\0file.txt\0A\0new.txt\0");
        assert_eq!(
            parsed,
            vec![
                ("M".to_string(), "file.txt".to_string(), None),
                ("A".to_string(), "new.txt".to_string(), None),
            ]
        );
    }

    #[test]
    fn parse_git_diff_name_status_handles_rename_entries() {
        let parsed = parse_git_diff_name_status_bytes(b"R100\0old.txt\0new.txt\0");
        assert_eq!(
            parsed,
            vec![(
                "R100".to_string(),
                "new.txt".to_string(),
                Some("old.txt".to_string()),
            )]
        );
    }

    #[test]
    fn parse_git_status_structured_handles_rename_entries() {
        let parsed = git_status_structured_from_bytes(b"## main\0R  new.txt\0old.txt\0");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "new.txt");
        assert_eq!(parsed.entries[0].orig_path.as_deref(), Some("old.txt"));
        assert_eq!(parsed.staged, 1);
        assert_eq!(parsed.unstaged, 0);
    }

    #[test]
    fn parse_git_status_structured_can_skip_entries() {
        let parsed = git_status_structured_from_bytes_with_entries(
            b"## main\0M  file.txt\0?? new.txt\0",
            false,
        );
        assert!(parsed.entries.is_empty());
        assert_eq!(parsed.total_count, 2);
        assert_eq!(parsed.staged, 1);
        assert_eq!(parsed.untracked, 1);
        assert!(parsed.raw.is_empty());
    }
}
