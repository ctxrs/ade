use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use ctx_core::ids::{SessionId, WorktreeId};
use ctx_core::models::{MergeQueueEntry, VcsKind, Workspace, Worktree};
use ctx_fs::git::delete_branch;
use ctx_fs::vcs::{self, ApplyPatchTarget, VcsDriver};
use ctx_fs::worktrees::{create_worktree, remove_worktree};

use ctx_workspace_config::MergeQueueConfig;

use super::context::{find_checked_out_worktree_for_branch, resolve_target_head};
use super::storage::{merge_queue_worktree_path, read_patch_file, write_log_line};
use super::sync::{
    ensure_jj_working_copy, ensure_merge_queue_repo, ensure_merge_queue_target_branch,
    maybe_sync_canonical_worktree,
};
use super::target::finalize_target_branch;
use super::{
    command_for_shell, merge_queue_command, vcs_driver_for_worktree, MergeQueueHost,
    MergeQueueNotice, QueueError, MERGE_QUEUE_CONFLICT_MESSAGE,
};

pub(super) async fn run_entry_inner<H: MergeQueueHost>(
    state: &Arc<H>,
    workspace: &Workspace,
    entry: &MergeQueueEntry,
    cfg: &MergeQueueConfig,
    log_file: &mut fs::File,
) -> std::result::Result<String, QueueError> {
    let vcs = vcs::driver_for_path(Path::new(&workspace.root_path))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    vcs.assert_repo(Path::new(&workspace.root_path))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    let worktree_path =
        merge_queue_worktree_path(Path::new(&workspace.root_path), workspace.id, entry.id);
    let worktree_branch = if vcs.kind() == VcsKind::Jj {
        format!("ctx-merge-queue-{}", entry.id.0)
    } else {
        format!("ctx-merge-queue/{}", entry.id.0)
    };
    let (repo_root, target_head) = if vcs.kind() == VcsKind::Git {
        let repo_root =
            ensure_merge_queue_repo(state.as_ref(), entry, workspace, cfg, log_file).await?;
        let target_head = ensure_merge_queue_target_branch(
            state.as_ref(),
            entry,
            &repo_root,
            &entry.target_branch,
        )
        .await?;
        let _ = remove_worktree(&repo_root, &worktree_path).await;
        let _ = delete_branch(&repo_root, &worktree_branch).await;
        create_worktree(&repo_root, &worktree_path, &target_head, &worktree_branch)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        (Some(repo_root), target_head)
    } else {
        let target_head =
            resolve_target_head(vcs.as_ref(), &workspace.root_path, &entry.target_branch)
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
        create_worktree(
            &workspace.root_path,
            &worktree_path,
            &target_head,
            &worktree_branch,
        )
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        (None, target_head)
    };
    let git_repo_root = repo_root
        .as_deref()
        .unwrap_or_else(|| Path::new(&workspace.root_path));

    if vcs.kind() == VcsKind::Jj {
        ensure_jj_working_copy(&worktree_path, &target_head, log_file, vcs.as_ref()).await?;
    }

    let result = async {
        let patch = read_patch_file(&entry.patch_path)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        write_log_line(log_file, "apply patch\n")
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        let apply_target = if vcs.kind() == VcsKind::Git {
            ApplyPatchTarget::Index
        } else {
            ApplyPatchTarget::Worktree
        };
        if let Err(err) = apply_patch(
            state.as_ref(),
            entry,
            vcs.as_ref(),
            git_repo_root,
            &worktree_path,
            &patch,
            apply_target,
        )
        .await
        {
            match err {
                QueueError::Conflict { message } => {
                    let _ = write_log_line(log_file, &format!("apply patch conflict: {message}\n"))
                        .await;
                    return Err(QueueError::Conflict {
                        message: MERGE_QUEUE_CONFLICT_MESSAGE.to_string(),
                    });
                }
                other => return Err(other),
            }
        }

        let has_changes = if vcs.kind() == VcsKind::Git {
            has_staged_changes(state.as_ref(), entry, &worktree_path).await?
        } else {
            has_worktree_changes(vcs.as_ref(), &worktree_path, &target_head).await?
        };
        if !has_changes {
            return Err(QueueError::fail(
                "patch did not produce any changes".to_string(),
                None,
                None,
            ));
        }

        let message = entry
            .message
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or("merge queue entry");
        commit_changes(
            state.as_ref(),
            entry,
            &worktree_path,
            vcs.kind(),
            message,
            log_file,
        )
        .await?;
        let commit_sha = vcs
            .rev_parse_head(&worktree_path)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

        for cmd in &cfg.verify_commands {
            run_verify_command(
                state.as_ref(),
                &worktree_path,
                entry,
                cmd,
                &commit_sha,
                log_file,
            )
            .await?;
        }

        let target_checkout = if vcs.kind() == VcsKind::Git {
            find_checked_out_worktree_for_branch(
                state.as_ref(),
                entry,
                git_repo_root,
                &entry.target_branch,
            )
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?
        } else {
            None
        };
        if let Some(path) = target_checkout.as_ref() {
            let dirty = vcs
                .status_porcelain(Path::new(path))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?;
            if !dirty.is_empty() {
                return Err(QueueError::fail(
                    format!(
                        "target branch {} is checked out at {} with uncommitted changes",
                        entry.target_branch, path
                    ),
                    None,
                    Some(commit_sha.clone()),
                ));
            }
        }

        finalize_target_branch(
            state.as_ref(),
            workspace,
            entry,
            cfg,
            vcs.as_ref(),
            git_repo_root,
            target_checkout.as_deref(),
            &target_head,
            &commit_sha,
            log_file,
        )
        .await?;

        if vcs.kind() == VcsKind::Git {
            if let Some(repo_root) = repo_root.as_ref() {
                if let Err(err) = maybe_sync_canonical_worktree(
                    state,
                    workspace,
                    entry,
                    repo_root,
                    &commit_sha,
                    cfg.canonical_sync,
                    log_file,
                )
                .await
                {
                    let _ = write_log_line(log_file, &format!("canonical sync failed: {err:#}\n"))
                        .await;
                    tracing::warn!("merge queue canonical sync failed: {err:#}");
                }
            }
        }

        Ok(commit_sha)
    }
    .await;

    if vcs.kind() == VcsKind::Git {
        if let Some(repo_root) = repo_root.as_ref() {
            let _ = remove_worktree(repo_root, &worktree_path).await;
            let _ = delete_branch(repo_root, &worktree_branch).await;
        }
    } else {
        let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
    }
    result
}

async fn apply_patch<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    vcs: &dyn VcsDriver,
    repo_root: &Path,
    worktree_path: &Path,
    patch: &str,
    target: ApplyPatchTarget,
) -> std::result::Result<(), QueueError> {
    if vcs.kind() == VcsKind::Git {
        let output = run_git_apply(
            state,
            entry,
            worktree_path,
            patch,
            matches!(target, ApplyPatchTarget::Index),
            true,
        )
        .await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if matches!(target, ApplyPatchTarget::Index) && stderr.contains("does not exist in index") {
            let output = run_git_apply(state, entry, worktree_path, patch, false, false).await?;
            if output.status.success() {
                stage_worktree(state, entry, worktree_path).await?;
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::Conflict {
                message: if stderr.is_empty() {
                    "patch conflict".to_string()
                } else {
                    stderr
                },
            });
        }
        return Err(QueueError::Conflict {
            message: if stderr.is_empty() {
                "patch conflict".to_string()
            } else {
                stderr
            },
        });
    }

    if vcs.kind() == VcsKind::Jj {
        let rel = worktree_path.strip_prefix(repo_root).map_err(|_| {
            QueueError::fail("jj worktree is outside repo root".to_string(), None, None)
        })?;
        let mut cmd =
            merge_queue_command(state, entry, "git apply", "git", Some(repo_root), &[]).await;
        cmd.arg("-C")
            .arg(repo_root)
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg("--directory")
            .arg(rel)
            .arg("-");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(patch.as_bytes())
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::Conflict {
                message: if stderr.is_empty() {
                    "patch conflict".to_string()
                } else {
                    stderr
                },
            });
        }
        return Ok(());
    }

    vcs.apply_patch(worktree_path, patch, target, false)
        .await
        .map_err(|e| QueueError::Conflict {
            message: e.to_string(),
        })?;
    Ok(())
}

async fn run_git_apply<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
    patch: &str,
    apply_index: bool,
    three_way: bool,
) -> std::result::Result<std::process::Output, QueueError> {
    let mut cmd =
        merge_queue_command(state, entry, "git apply", "git", Some(worktree_path), &[]).await;
    cmd.arg("-C")
        .arg(worktree_path)
        .arg("apply")
        .arg("--whitespace=nowarn");
    if three_way {
        cmd.arg("--3way");
    }
    if apply_index {
        cmd.arg("--index");
    }
    cmd.arg("-");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    }

    child
        .wait_with_output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))
}

async fn stage_worktree<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
) -> std::result::Result<(), QueueError> {
    let mut cmd =
        merge_queue_command(state, entry, "git add -A", "git", Some(worktree_path), &[]).await;
    let output = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["add", "-A"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(QueueError::fail(
            format!("git add failed: {stderr}"),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }
    Ok(())
}

async fn has_staged_changes<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
) -> std::result::Result<bool, QueueError> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git diff --cached --quiet",
        "git",
        Some(worktree_path),
        &[],
    )
    .await;
    let status = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["diff", "--cached", "--quiet"])
        .status()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    Ok(!status.success())
}

async fn has_worktree_changes(
    vcs: &dyn VcsDriver,
    worktree_path: &Path,
    base_revision: &str,
) -> std::result::Result<bool, QueueError> {
    let diff = vcs
        .diff(worktree_path, base_revision)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    Ok(!diff.trim().is_empty())
}

async fn commit_changes<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
    vcs_kind: VcsKind,
    message: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    match vcs_kind {
        VcsKind::Git => {
            let mut cmd =
                merge_queue_command(state, entry, "git commit", "git", Some(worktree_path), &[])
                    .await;
            let output = cmd
                .arg("-C")
                .arg(worktree_path)
                .args([
                    "-c",
                    "user.name=ctx",
                    "-c",
                    "user.email=ctx@local",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    message,
                ])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            if !output.status.success() {
                return Err(QueueError::fail(
                    "git commit failed".to_string(),
                    Some(output.status.code().unwrap_or(1) as i64),
                    None,
                ));
            }
            Ok(())
        }
        VcsKind::Jj => {
            let mut cmd =
                merge_queue_command(state, entry, "jj describe", "jj", Some(worktree_path), &[])
                    .await;
            let output = cmd
                .arg("-R")
                .arg(worktree_path)
                .arg("--color=never")
                .arg("--no-pager")
                .args(["describe", "-m", message])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            if !output.status.success() {
                return Err(QueueError::fail(
                    "jj describe failed".to_string(),
                    Some(output.status.code().unwrap_or(1) as i64),
                    None,
                ));
            }
            Ok(())
        }
        VcsKind::Hg | VcsKind::Svn | VcsKind::P4 | VcsKind::Other => Err(QueueError::fail(
            format!("merge queue does not support {vcs_kind:?} commits"),
            None,
            None,
        )),
    }
}

async fn run_verify_command<H: MergeQueueHost>(
    state: &H,
    worktree_path: &Path,
    entry: &MergeQueueEntry,
    command: &str,
    commit_sha: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    write_log_line(log_file, &format!("verify: {command}\n"))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
    let envs = vec![
        (
            "CTX_MERGE_QUEUE_ENTRY_ID".to_string(),
            entry.id.0.to_string(),
        ),
        (
            "CTX_WORKTREE_ROOT".to_string(),
            worktree_path.to_string_lossy().to_string(),
        ),
        ("CTX_TARGET_BRANCH".to_string(), entry.target_branch.clone()),
    ];
    let mut cmd = command_for_shell(state, entry, command, worktree_path, &envs).await;
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
    if !output.status.success() {
        return Err(QueueError::fail(
            format!("verify failed: {command}"),
            Some(output.status.code().unwrap_or(1) as i64),
            Some(commit_sha.to_string()),
        ));
    }
    Ok(())
}

pub(super) async fn maybe_sync_originating_worktree<H: MergeQueueHost>(
    state: &Arc<H>,
    workspace: &Workspace,
    entry: &MergeQueueEntry,
    commit_sha: &str,
) -> Result<()> {
    let Some(worktree_id) = entry.worktree_id else {
        return Ok(());
    };
    let store = H::worktree_store(state.as_ref(), worktree_id).await?;
    let Some(worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(());
    };
    if worktree.workspace_id != workspace.id {
        return Ok(());
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    let worktree_root = Path::new(&worktree.root_path);
    vcs.assert_repo(worktree_root).await?;
    let previous_head = match vcs.kind() {
        VcsKind::Jj => {
            let Some(expected_head) = entry.head_commit_sha.as_deref() else {
                return Ok(());
            };
            let current_head = vcs
                .rev_parse_head(worktree_root)
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            if current_head.trim() != expected_head.trim() {
                return Ok(());
            }
            current_head
        }
        _ => {
            let dirty = vcs.status_porcelain(worktree_root).await?;
            if !dirty.is_empty() {
                return Ok(());
            }
            vcs.rev_parse_head(worktree_root)
                .await
                .unwrap_or_else(|_| "unknown".to_string())
        }
    };
    if vcs.kind() == VcsKind::Git {
        reset_worktree_to_commit(state.as_ref(), entry, &worktree.root_path, commit_sha).await?;
    } else {
        reset_worktree_to_revision(vcs.as_ref(), &worktree.root_path, commit_sha).await?;
    }
    let updated = store
        .update_worktree_base_commit(worktree_id, commit_sha)
        .await?;
    if !updated {
        return Ok(());
    }
    if let Some(session_id) = entry.session_id {
        emit_merge_queue_sync_notice(
            state,
            session_id,
            &worktree,
            &entry.target_branch,
            &previous_head,
            commit_sha,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn emit_merge_queue_sync_notice<H: MergeQueueHost>(
    state: &Arc<H>,
    session_id: SessionId,
    worktree: &Worktree,
    target_branch: &str,
    previous_commit_sha: &str,
    commit_sha: &str,
) -> Result<()> {
    let previous_short = previous_commit_sha.get(0..8).unwrap_or(previous_commit_sha);
    let short_sha = commit_sha.get(0..8).unwrap_or(commit_sha);
    let message = format!(
        "merge queue applied; reset worktree from {previous_short} to {target_branch} ({short_sha})"
    );
    H::publish_notice(
        state,
        MergeQueueNotice::Sync {
            session_id,
            worktree_id: worktree.id,
            target_branch: target_branch.to_string(),
            previous_commit_sha: previous_commit_sha.to_string(),
            commit_sha: commit_sha.to_string(),
            message,
        },
    )
    .await
}

pub(super) async fn emit_merge_queue_canonical_sync_notice<H: MergeQueueHost>(
    state: &Arc<H>,
    session_id: SessionId,
    worktree_id: Option<WorktreeId>,
    target_branch: &str,
    commit_sha: &str,
    status: &str,
    message: &str,
) -> Result<()> {
    H::publish_notice(
        state,
        MergeQueueNotice::CanonicalSync {
            session_id,
            worktree_id,
            target_branch: target_branch.to_string(),
            commit_sha: commit_sha.to_string(),
            status: status.to_string(),
            message: message.to_string(),
        },
    )
    .await
}

pub(super) async fn reset_worktree_to_revision(
    vcs: &dyn VcsDriver,
    worktree_path: &str,
    revision: &str,
) -> Result<()> {
    vcs.reset_worktree_to_revision(Path::new(worktree_path), revision)
        .await
        .context("resetting worktree to revision")?;
    Ok(())
}

pub(super) async fn reset_worktree_to_commit<H: MergeQueueHost>(
    state: &H,
    entry: &MergeQueueEntry,
    worktree_path: &str,
    commit_sha: &str,
) -> Result<()> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git reset --hard",
        "git",
        Some(Path::new(worktree_path)),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["reset", "--hard", commit_sha])
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
}
