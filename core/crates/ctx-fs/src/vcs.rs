use std::collections::HashSet;
use std::future::Future;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use tokio::fs;
use tokio::process::Command;

use ctx_core::models::VcsKind;

use crate::git;
use crate::patch::{self, WorktreePatch};

pub use crate::git::ApplyPatchTarget;

pub type VcsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait VcsDriver: Send + Sync {
    fn kind(&self) -> VcsKind;
    fn assert_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, ()>;
    fn is_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, bool>;
    fn is_worktree<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, bool>;
    fn rev_parse_head<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, String>;
    fn rev_parse_ref<'a>(&'a self, root: &'a Path, reference: &'a str) -> VcsFuture<'a, String>;
    fn merge_base<'a>(&'a self, root: &'a Path, a: &'a str, b: &'a str) -> VcsFuture<'a, String>;
    fn is_ancestor<'a>(
        &'a self,
        root: &'a Path,
        ancestor: &'a str,
        descendant: &'a str,
    ) -> VcsFuture<'a, bool>;
    fn create_worktree<'a>(
        &'a self,
        workspace_root: &'a Path,
        worktree_path: &'a Path,
        base_revision: &'a str,
        branch_name: &'a str,
    ) -> VcsFuture<'a, ()>;
    fn remove_worktree<'a>(
        &'a self,
        workspace_root: &'a Path,
        worktree_path: &'a Path,
    ) -> VcsFuture<'a, ()>;
    fn prune_worktrees<'a>(&'a self, workspace_root: &'a Path) -> VcsFuture<'a, ()>;
    fn diff<'a>(&'a self, worktree_path: &'a Path, base_revision: &'a str)
        -> VcsFuture<'a, String>;
    fn diff_summary<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, (i64, i64, i64)>;
    fn list_untracked<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, Vec<String>>;
    fn diff_untracked_file<'a>(
        &'a self,
        worktree_path: &'a Path,
        rel_path: &'a str,
    ) -> VcsFuture<'a, String>;
    fn status_short<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, String>;
    fn status_porcelain<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, Vec<String>>;
    fn build_worktree_patch<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, WorktreePatch>;
    fn apply_patch<'a>(
        &'a self,
        root: &'a Path,
        patch: &'a str,
        target: ApplyPatchTarget,
        reverse: bool,
    ) -> VcsFuture<'a, ()>;
    fn reset_worktree_to_revision<'a>(
        &'a self,
        root: &'a Path,
        revision: &'a str,
    ) -> VcsFuture<'a, ()>;
    fn delete_branch<'a>(&'a self, root: &'a Path, branch: &'a str) -> VcsFuture<'a, ()>;
}

pub struct GitVcs;

impl VcsDriver for GitVcs {
    fn kind(&self) -> VcsKind {
        VcsKind::Git
    }

    fn assert_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, ()> {
        Box::pin(async move { git::assert_git_repo(root).await })
    }

    fn is_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, bool> {
        Box::pin(async move { Ok(git::is_git_worktree(root).await.unwrap_or(false)) })
    }

    fn is_worktree<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, bool> {
        Box::pin(async move { git::is_git_worktree(worktree_path).await })
    }

    fn rev_parse_head<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, String> {
        Box::pin(async move { git::rev_parse_head(root).await })
    }

    fn rev_parse_ref<'a>(&'a self, root: &'a Path, reference: &'a str) -> VcsFuture<'a, String> {
        Box::pin(async move { git::rev_parse_ref(root, reference).await })
    }

    fn merge_base<'a>(&'a self, root: &'a Path, a: &'a str, b: &'a str) -> VcsFuture<'a, String> {
        Box::pin(async move { git::git_merge_base(root, a, b).await })
    }

    fn is_ancestor<'a>(
        &'a self,
        root: &'a Path,
        ancestor: &'a str,
        descendant: &'a str,
    ) -> VcsFuture<'a, bool> {
        Box::pin(async move { git::git_is_ancestor(root, ancestor, descendant).await })
    }

    fn create_worktree<'a>(
        &'a self,
        workspace_root: &'a Path,
        worktree_path: &'a Path,
        base_revision: &'a str,
        branch_name: &'a str,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(workspace_root)
                .arg("worktree")
                .arg("add")
                .arg(worktree_path);
            if branch_name.trim().is_empty() {
                cmd.arg(base_revision);
            } else if git::branch_exists(workspace_root, branch_name).await? {
                cmd.arg(branch_name);
            } else {
                cmd.arg("-b").arg(branch_name).arg(base_revision);
            }
            let output = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running git worktree add")?;
            if !output.status.success() {
                bail!(
                    "git worktree add failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn remove_worktree<'a>(
        &'a self,
        workspace_root: &'a Path,
        worktree_path: &'a Path,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            let output = Command::new("git")
                .arg("-C")
                .arg(workspace_root)
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(worktree_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running git worktree remove")?;
            if !output.status.success() {
                bail!(
                    "git worktree remove failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn prune_worktrees<'a>(&'a self, workspace_root: &'a Path) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            let output = Command::new("git")
                .arg("-C")
                .arg(workspace_root)
                .arg("worktree")
                .arg("prune")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running git worktree prune")?;
            if !output.status.success() {
                bail!(
                    "git worktree prune failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn diff<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, String> {
        Box::pin(async move { git::git_diff(worktree_path, base_revision).await })
    }

    fn diff_summary<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, (i64, i64, i64)> {
        Box::pin(async move {
            let mut file_count: i64 = 0;
            let mut additions: i64 = 0;
            let mut deletions: i64 = 0;
            let numstats = git::git_diff_numstat(worktree_path, base_revision).await?;
            for (add, del, _path) in numstats {
                file_count += 1;
                additions += add;
                deletions += del;
            }
            Ok((file_count, additions, deletions))
        })
    }

    fn list_untracked<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, Vec<String>> {
        Box::pin(async move { git::list_untracked_files(worktree_path).await })
    }

    fn diff_untracked_file<'a>(
        &'a self,
        worktree_path: &'a Path,
        rel_path: &'a str,
    ) -> VcsFuture<'a, String> {
        Box::pin(async move { git::git_diff_untracked_file(worktree_path, rel_path).await })
    }

    fn status_short<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, String> {
        Box::pin(async move { git::git_status_short(root).await })
    }

    fn status_porcelain<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, Vec<String>> {
        Box::pin(async move { git::git_status_porcelain(root).await })
    }

    fn build_worktree_patch<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, WorktreePatch> {
        Box::pin(async move { patch::build_worktree_patch(worktree_path, base_revision).await })
    }

    fn apply_patch<'a>(
        &'a self,
        root: &'a Path,
        patch: &'a str,
        target: ApplyPatchTarget,
        reverse: bool,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move { git::git_apply_patch(root, patch, target, reverse).await })
    }

    fn reset_worktree_to_revision<'a>(
        &'a self,
        root: &'a Path,
        revision: &'a str,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .arg("reset")
                .arg("--hard")
                .arg(revision)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running git reset --hard")?;
            if !output.status.success() {
                bail!(
                    "git reset --hard failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn delete_branch<'a>(&'a self, root: &'a Path, branch: &'a str) -> VcsFuture<'a, ()> {
        Box::pin(async move { git::delete_branch(root, branch).await })
    }
}

pub struct JjVcs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct JjVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl std::fmt::Display for JjVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

const JJ_MIN_VERSION: JjVersion = JjVersion {
    major: 0,
    minor: 25,
    patch: 0,
};

static JJ_VERSION_OK: OnceLock<JjVersion> = OnceLock::new();

fn parse_jj_version(output: &str) -> Option<JjVersion> {
    for token in output.split_whitespace() {
        let token = token.trim_start_matches('v');
        let mut version = String::new();
        let mut saw_digit = false;
        for ch in token.chars() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                version.push(ch);
                continue;
            }
            if ch == '.' && saw_digit {
                version.push(ch);
                continue;
            }
            break;
        }
        if version.is_empty() {
            continue;
        }
        let parts = version.split('.').collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts.get(2).and_then(|part| part.parse().ok()).unwrap_or(0);
        return Some(JjVersion {
            major,
            minor,
            patch,
        });
    }
    None
}

fn jj_version_supported(version: JjVersion) -> bool {
    (version.major, version.minor, version.patch)
        >= (
            JJ_MIN_VERSION.major,
            JJ_MIN_VERSION.minor,
            JJ_MIN_VERSION.patch,
        )
}

async fn probe_jj_version() -> Result<JjVersion> {
    let output = Command::new("jj")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                anyhow::anyhow!("jj is required for Jujutsu repositories but was not found in PATH")
            } else {
                anyhow::anyhow!("running jj --version failed: {err}")
            }
        })?;
    if !output.status.success() {
        bail!(
            "jj --version failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_jj_version(&stdout)
        .ok_or_else(|| anyhow::anyhow!("unable to parse jj version from `{}`", stdout.trim()))?;
    Ok(version)
}

async fn ensure_jj_usable() -> Result<JjVersion> {
    if let Some(version) = JJ_VERSION_OK.get() {
        return Ok(*version);
    }
    let version = probe_jj_version().await?;
    if !jj_version_supported(version) {
        bail!(
            "jj {} is too old; ctx requires jj >= {}",
            version,
            JJ_MIN_VERSION
        );
    }
    let _ = JJ_VERSION_OK.set(version);
    Ok(version)
}

impl VcsDriver for JjVcs {
    fn kind(&self) -> VcsKind {
        VcsKind::Jj
    }

    fn assert_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            run_jj(root, &["root"]).await?;
            Ok(())
        })
    }

    fn is_repo<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, bool> {
        Box::pin(async move {
            if ensure_jj_usable().await.is_err() {
                return Ok(false);
            }
            let output = jj_command(root)
                .arg("root")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .await;
            match output {
                Ok(output) => Ok(output.status.success()),
                Err(_) => Ok(false),
            }
        })
    }

    fn is_worktree<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, bool> {
        Box::pin(async move {
            if ensure_jj_usable().await.is_err() {
                return Ok(false);
            }
            let output = jj_command(worktree_path)
                .arg("root")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .await;
            match output {
                Ok(output) => Ok(output.status.success()),
                Err(_) => Ok(false),
            }
        })
    }

    fn rev_parse_head<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, String> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            jj_rev_parse(root, "@").await
        })
    }

    fn rev_parse_ref<'a>(&'a self, root: &'a Path, reference: &'a str) -> VcsFuture<'a, String> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            jj_rev_parse(root, reference).await
        })
    }

    fn merge_base<'a>(&'a self, root: &'a Path, a: &'a str, b: &'a str) -> VcsFuture<'a, String> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            jj_merge_base(root, a, b).await
        })
    }

    fn is_ancestor<'a>(
        &'a self,
        root: &'a Path,
        ancestor: &'a str,
        descendant: &'a str,
    ) -> VcsFuture<'a, bool> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            jj_is_ancestor(root, ancestor, descendant).await
        })
    }

    fn create_worktree<'a>(
        &'a self,
        workspace_root: &'a Path,
        worktree_path: &'a Path,
        base_revision: &'a str,
        branch_name: &'a str,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let mut cmd = jj_command(workspace_root);
            cmd.arg("workspace").arg("add");
            if !branch_name.trim().is_empty() {
                cmd.arg("--name").arg(branch_name);
            }
            if !base_revision.trim().is_empty() {
                cmd.arg("--revision").arg(base_revision);
            }
            if let Some(parent) = worktree_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            cmd.arg(worktree_path);
            let output = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running jj workspace add")?;
            if !output.status.success() {
                bail!(
                    "jj workspace add failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn remove_worktree<'a>(
        &'a self,
        _workspace_root: &'a Path,
        worktree_path: &'a Path,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let output = jj_command(worktree_path)
                .arg("workspace")
                .arg("forget")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running jj workspace forget")?;
            if !output.status.success() {
                bail!(
                    "jj workspace forget failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        })
    }

    fn prune_worktrees<'a>(&'a self, workspace_root: &'a Path) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            run_jj(workspace_root, &["workspace", "update-stale"]).await?;
            Ok(())
        })
    }

    fn diff<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, String> {
        Box::pin(async move {
            let output = run_jj(
                worktree_path,
                &["diff", "--git", "--from", base_revision, "--to", "@"],
            )
            .await?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        })
    }

    fn diff_summary<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, (i64, i64, i64)> {
        Box::pin(async move {
            let output = run_jj(
                worktree_path,
                &["diff", "--git", "--from", base_revision, "--to", "@"],
            )
            .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(diff_summary_from_git(&stdout))
        })
    }

    fn list_untracked<'a>(&'a self, worktree_path: &'a Path) -> VcsFuture<'a, Vec<String>> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let output = run_jj(worktree_path, &["status"]).await?;
            let parsed = parse_jj_status_output(&String::from_utf8_lossy(&output.stdout));
            collect_jj_untracked_files(worktree_path, &parsed.untracked).await
        })
    }

    fn diff_untracked_file<'a>(
        &'a self,
        worktree_path: &'a Path,
        rel_path: &'a str,
    ) -> VcsFuture<'a, String> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            git::git_diff_untracked_file(worktree_path, rel_path).await
        })
    }

    fn status_short<'a>(&'a self, _root: &'a Path) -> VcsFuture<'a, String> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            Ok("## @\n".to_string())
        })
    }

    fn status_porcelain<'a>(&'a self, root: &'a Path) -> VcsFuture<'a, Vec<String>> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let output = run_jj(root, &["status"]).await?;
            let parsed = parse_jj_status_output(&String::from_utf8_lossy(&output.stdout));
            let mut entries = Vec::new();
            for entry in parsed.entries {
                entries.push(format!(" {} {}", entry.status, entry.path));
            }
            for entry in parsed.untracked {
                if patch::should_ignore_path(Path::new(&entry.path)) {
                    continue;
                }
                let mut path = entry.path;
                if entry.is_dir {
                    path.push(std::path::MAIN_SEPARATOR);
                }
                entries.push(format!("?? {}", path));
            }
            Ok(entries)
        })
    }

    fn build_worktree_patch<'a>(
        &'a self,
        worktree_path: &'a Path,
        base_revision: &'a str,
    ) -> VcsFuture<'a, WorktreePatch> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let output = run_jj(worktree_path, &["status"]).await?;
            let parsed = parse_jj_status_output(&String::from_utf8_lossy(&output.stdout));
            let untracked = collect_jj_untracked_files(worktree_path, &parsed.untracked).await?;
            let head_revision = jj_rev_parse(worktree_path, "@")
                .await
                .unwrap_or_else(|_| base_revision.to_string());
            let diff_output = run_jj(
                worktree_path,
                &["diff", "--git", "--from", base_revision, "--to", "@"],
            )
            .await?;
            let tracked_diff = String::from_utf8_lossy(&diff_output.stdout).to_string();
            let mut patch_text = tracked_diff.clone();
            let mut changed_files = jj_diff_name_only(worktree_path, base_revision).await?;
            let mut seen: HashSet<String> = changed_files.iter().cloned().collect();
            let (mut file_count, mut line_additions, line_deletions) =
                diff_summary_from_git(&tracked_diff);
            for file in &untracked {
                if patch::should_ignore_path(Path::new(file)) {
                    continue;
                }
                let diff = git_output_allow(
                    worktree_path,
                    &["diff", "--binary", "--no-index", "--", "/dev/null", file],
                    &[0, 1],
                )
                .await?;
                if !diff.is_empty() {
                    patch_text.push_str(&diff);
                }
                if seen.insert(file.clone()) {
                    changed_files.push(file.clone());
                    file_count += 1;
                    if let Ok(lines) = count_file_lines(&worktree_path.join(file)).await {
                        line_additions += lines;
                    }
                }
            }
            Ok(WorktreePatch {
                base_revision: base_revision.to_string(),
                head_revision: head_revision.trim().to_string(),
                patch: patch_text,
                changed_files,
                file_count,
                line_additions,
                line_deletions,
            })
        })
    }

    fn apply_patch<'a>(
        &'a self,
        root: &'a Path,
        patch: &'a str,
        target: ApplyPatchTarget,
        reverse: bool,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            if matches!(target, ApplyPatchTarget::Index) || reverse {
                return git::git_apply_patch(root, patch, target, reverse).await;
            }
            match jj_apply_patch(root, patch).await? {
                JjApplyOutcome::Applied => Ok(()),
                JjApplyOutcome::Unsupported => {
                    git::git_apply_patch(root, patch, target, reverse).await
                }
            }
        })
    }

    fn reset_worktree_to_revision<'a>(
        &'a self,
        root: &'a Path,
        revision: &'a str,
    ) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let output = jj_command(root)
                .arg("edit")
                .arg(revision)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running jj edit")?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if jj_command_unsupported(&stderr) {
                bail!(
                    "jj edit is required to reset worktrees in jj repos, but this jj build does not support it: {}",
                    stderr.trim()
                );
            }
            bail!("jj edit failed: {}", stderr.trim())
        })
    }

    fn delete_branch<'a>(&'a self, root: &'a Path, branch: &'a str) -> VcsFuture<'a, ()> {
        Box::pin(async move {
            ensure_jj_usable().await?;
            let branch = branch.trim();
            if branch.is_empty() {
                return Ok(());
            }
            let output = jj_command(root)
                .arg("bookmark")
                .arg("delete")
                .arg(branch)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("running jj bookmark delete")?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if jj_command_unsupported(&stderr) {
                bail!(
                    "jj bookmark delete is required to remove bookmarks in jj repos, but this jj build does not support it: {}",
                    stderr.trim()
                );
            }
            let lower = stderr.to_lowercase();
            if lower.contains("no such bookmark")
                || lower.contains("no matching bookmarks")
                || lower.contains("no bookmarks")
            {
                return Ok(());
            }
            bail!("jj bookmark delete failed: {}", stderr.trim())
        })
    }
}

pub async fn driver_for_path(root: impl AsRef<Path>) -> Result<Arc<dyn VcsDriver>> {
    let root = root.as_ref();
    if root.join(".jj").exists() {
        let jj = JjVcs;
        jj.assert_repo(root).await?;
        return Ok(Arc::new(JjVcs));
    }
    let jj = JjVcs;
    if jj.is_repo(root).await.unwrap_or(false) {
        ensure_jj_usable().await?;
        return Ok(Arc::new(JjVcs));
    }
    let git = GitVcs;
    if git.is_repo(root).await.unwrap_or(false) {
        return Ok(Arc::new(GitVcs));
    }
    bail!("no vcs repo found at {}", root.display());
}

pub fn driver_for_kind(kind: Option<VcsKind>) -> Arc<dyn VcsDriver> {
    match kind {
        Some(VcsKind::Jj) => Arc::new(JjVcs),
        _ => Arc::new(GitVcs),
    }
}

fn jj_command(root: &Path) -> Command {
    let mut cmd = Command::new("jj");
    cmd.arg("-R")
        .arg(root)
        .arg("--color=never")
        .arg("--no-pager");
    cmd
}

pub async fn jj_command_output(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    ensure_jj_usable().await?;
    let output = jj_command(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("running jj {:?}", args))?;
    Ok(output)
}

async fn run_jj(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = jj_command_output(root, args).await?;
    if !output.status.success() {
        bail!(
            "jj {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
enum JjApplyOutcome {
    Applied,
    Unsupported,
}

fn jj_command_unsupported(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("unrecognized subcommand")
        || lower.contains("unknown subcommand")
        || lower.contains("unknown command")
}

fn jj_command_usage(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("usage:")
        || lower.contains("required arguments")
        || lower.contains("unexpected argument")
}

async fn jj_apply_patch(root: &Path, patch: &str) -> Result<JjApplyOutcome> {
    ensure_jj_usable().await?;
    let output = jj_apply_patch_stdin(root, patch).await?;
    if output.status.success() {
        return Ok(JjApplyOutcome::Applied);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if jj_command_unsupported(&stderr) {
        return Ok(JjApplyOutcome::Unsupported);
    }
    if jj_command_usage(&stderr) {
        return jj_apply_patch_file(root, patch).await;
    }
    bail!("jj apply failed: {}", stderr.trim());
}

async fn jj_apply_patch_stdin(root: &Path, patch: &str) -> Result<std::process::Output> {
    let mut cmd = jj_command(root);
    cmd.arg("apply");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning jj apply")?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(patch.as_bytes())
            .await
            .context("writing patch to jj apply stdin")?;
    }
    let output = child
        .wait_with_output()
        .await
        .context("waiting for jj apply")?;
    Ok(output)
}

async fn jj_apply_patch_file(root: &Path, patch: &str) -> Result<JjApplyOutcome> {
    let path = temp_patch_path();
    fs::write(&path, patch)
        .await
        .context("writing patch for jj apply")?;
    let output = jj_command(root)
        .arg("apply")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running jj apply")?;
    let _ = fs::remove_file(&path).await;
    if output.status.success() {
        return Ok(JjApplyOutcome::Applied);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if jj_command_unsupported(&stderr) || jj_command_usage(&stderr) {
        return Ok(JjApplyOutcome::Unsupported);
    }
    bail!("jj apply failed: {}", stderr.trim());
}

fn normalize_jj_revset(reference: &str) -> String {
    let reference = reference.trim();
    if reference == "HEAD" {
        return "@".to_string();
    }
    if let Some(stripped) = reference.strip_prefix("refs/heads/") {
        return stripped.to_string();
    }
    if let Some(stripped) = reference.strip_prefix("refs/remotes/") {
        return stripped.to_string();
    }
    reference.to_string()
}

async fn jj_revset_first_commit(root: &Path, revset: &str) -> Result<Option<String>> {
    let output = run_jj(
        root,
        &["log", "-r", revset, "--no-graph", "-T", "commit_id"],
    )
    .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string()))
}

async fn jj_rev_parse(root: &Path, rev: &str) -> Result<String> {
    let revset = normalize_jj_revset(rev);
    jj_revset_first_commit(root, &revset)
        .await?
        .ok_or_else(|| anyhow::anyhow!("jj log produced no revision output"))
}

async fn jj_merge_base(root: &Path, a: &str, b: &str) -> Result<String> {
    let a_revset = normalize_jj_revset(a);
    let b_revset = normalize_jj_revset(b);
    let revset = format!("heads(ancestors({a_revset}) & ancestors({b_revset}))");
    jj_revset_first_commit(root, &revset)
        .await?
        .ok_or_else(|| anyhow::anyhow!("jj merge-base produced no common ancestor"))
}

async fn jj_is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let ancestor_revset = normalize_jj_revset(ancestor);
    let descendant_revset = normalize_jj_revset(descendant);
    let revset = format!("({ancestor_revset}) & ancestors({descendant_revset})");
    Ok(jj_revset_first_commit(root, &revset).await?.is_some())
}

fn temp_patch_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("ctx-jj-apply-{nanos}.patch"))
}

async fn git_output_allow(root: &Path, args: &[&str], ok_codes: &[i32]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                anyhow::anyhow!("git is required to generate diffs for untracked files in jj repos")
            } else {
                anyhow::anyhow!("running git {:?} failed: {err}", args)
            }
        })?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        if !ok_codes.contains(&code) {
            bail!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn jj_diff_name_only(root: &Path, base_revision: &str) -> Result<Vec<String>> {
    let output = run_jj(
        root,
        &["diff", "--name-only", "--from", base_revision, "--to", "@"],
    )
    .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in stdout.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        if patch::should_ignore_path(Path::new(path)) {
            continue;
        }
        if seen.insert(path.to_string()) {
            out.push(path.to_string());
        }
    }
    Ok(out)
}

async fn count_file_lines(path: &Path) -> Result<i64> {
    let bytes = fs::read(path).await?;
    if bytes.contains(&0) {
        return Ok(0);
    }
    let mut lines = bytes.iter().filter(|b| **b == b'\n').count() as i64;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        lines += 1;
    }
    Ok(lines)
}

fn diff_summary_from_git(diff: &str) -> (i64, i64, i64) {
    let mut files = 0i64;
    let mut additions = 0i64;
    let mut deletions = 0i64;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            files += 1;
            continue;
        }
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if let Some(first) = line.as_bytes().first() {
            match first {
                b'+' => additions += 1,
                b'-' => deletions += 1,
                _ => {}
            }
        }
    }
    (files, additions, deletions)
}

#[derive(Debug)]
struct JjStatusEntry {
    status: char,
    path: String,
}

#[derive(Debug)]
struct JjUntrackedEntry {
    path: String,
    is_dir: bool,
}

#[derive(Debug)]
struct JjStatusParsed {
    entries: Vec<JjStatusEntry>,
    untracked: Vec<JjUntrackedEntry>,
}

fn parse_jj_status_output(output: &str) -> JjStatusParsed {
    let mut entries = Vec::new();
    let mut untracked = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.len() < 3 {
            continue;
        }
        let mut chars = line.chars();
        let status = chars.next().unwrap_or(' ');
        if chars.next() != Some(' ') {
            continue;
        }
        let mut path = chars.as_str().trim().to_string();
        if path.is_empty() {
            continue;
        }
        match status {
            '?' => {
                let mut is_dir = false;
                if path.ends_with('/') || path.ends_with('\\') {
                    is_dir = true;
                    path = path.trim_end_matches(&['/', '\\'][..]).to_string();
                }
                if !path.is_empty() {
                    untracked.push(JjUntrackedEntry { path, is_dir });
                }
            }
            'M' | 'A' | 'D' | 'R' | 'C' => {
                entries.push(JjStatusEntry { status, path });
            }
            _ => {}
        }
    }
    JjStatusParsed { entries, untracked }
}

async fn collect_jj_untracked_files(
    root: &Path,
    entries: &[JjUntrackedEntry],
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if entry.is_dir {
            collect_jj_untracked_dir(root, &entry.path, &mut out, &mut seen).await?;
        } else if !patch::should_ignore_path(Path::new(&entry.path))
            && seen.insert(entry.path.clone())
        {
            out.push(entry.path.clone());
        }
    }
    out.sort();
    Ok(out)
}

async fn collect_jj_untracked_dir(
    root: &Path,
    rel_dir: &str,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    let mut stack = vec![root.join(rel_dir)];
    while let Some(dir) = stack.pop() {
        let mut dir_entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Some(entry) = dir_entries.next_entry().await? {
            let path = entry.path();
            let rel = match path.strip_prefix(root) {
                Ok(rel) => rel,
                Err(_) => continue,
            };
            if patch::should_ignore_path(rel) {
                continue;
            }
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                stack.push(path);
            } else {
                let rel_str = rel.to_string_lossy().to_string();
                if seen.insert(rel_str.clone()) {
                    out.push(rel_str);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_jj_status_output;

    #[test]
    fn parse_jj_status_output_handles_untracked_dirs() {
        let output = r#"
Working copy changes:
M file.txt
A new.txt
Untracked paths:
? dir/
? stray.txt
Working copy  (@) : abcdef0123
"#;
        let parsed = parse_jj_status_output(output);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].status, 'M');
        assert_eq!(parsed.entries[0].path, "file.txt");
        assert_eq!(parsed.entries[1].status, 'A');
        assert_eq!(parsed.entries[1].path, "new.txt");
        assert_eq!(parsed.untracked.len(), 2);
        assert_eq!(parsed.untracked[0].path, "dir");
        assert!(parsed.untracked[0].is_dir);
        assert_eq!(parsed.untracked[1].path, "stray.txt");
        assert!(!parsed.untracked[1].is_dir);
    }
}
