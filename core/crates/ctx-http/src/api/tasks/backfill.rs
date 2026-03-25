use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use ctx_core::models::{ExecutionEnvironment, SandboxBinding, Workspace, Worktree};
use ctx_fs::worktrees::managed_worktree_path;
use ctx_store::Store;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerMountMode, ExecutionMode};
use crate::{settings, workspace_config};
use ctx_core::ids::WorkspaceId;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SandboxBindingBackfillStats {
    pub scanned_worktrees: usize,
    pub repaired_bindings: usize,
    pub repaired_session_metadata: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacySandboxRootKind {
    GuestRoot,
    AvfShadowRoot,
}

struct BackfilledSandboxWorktree {
    worktree: Worktree,
    binding: SandboxBinding,
}

fn legacy_guest_worktree_root(worktree: &Worktree) -> PathBuf {
    crate::disk_isolated::container_worktree_root(worktree.id)
}

fn legacy_avf_shadow_root(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: ctx_core::ids::WorktreeId,
) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    Some(
        data_root
            .join("managed")
            .join("vms")
            .join("avf-linux")
            .join(std::env::consts::OS)
            .join(std::env::consts::ARCH)
            .join("shared")
            .join("worktrees")
            .join(workspace_id.0.to_string())
            .join(worktree_id.0.to_string())
            .join("shadow-root"),
    )
}

fn legacy_sandbox_root_kind(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree: &Worktree,
) -> Option<LegacySandboxRootKind> {
    let root = Path::new(&worktree.root_path);
    if root == legacy_guest_worktree_root(worktree) {
        return Some(LegacySandboxRootKind::GuestRoot);
    }
    if legacy_avf_shadow_root(data_root, workspace_id, worktree.id)
        .as_ref()
        .is_some_and(|candidate| root == candidate)
    {
        return Some(LegacySandboxRootKind::AvfShadowRoot);
    }
    None
}

fn is_legacy_sandbox_candidate(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree: &Worktree,
    sessions: &[ctx_core::models::Session],
) -> bool {
    if sessions
        .iter()
        .any(|session| matches!(session.execution_environment, ExecutionEnvironment::Sandbox))
    {
        return true;
    }
    legacy_sandbox_root_kind(data_root, workspace_id, worktree).is_some()
}

fn legacy_binding_created_at(
    worktree: &Worktree,
    sessions: &[ctx_core::models::Session],
) -> DateTime<Utc> {
    sessions
        .iter()
        .filter(|session| matches!(session.execution_environment, ExecutionEnvironment::Sandbox))
        .map(|session| session.created_at)
        .min()
        .unwrap_or(worktree.created_at)
}

fn legacy_branch_name(worktree: &Worktree) -> Result<&str> {
    worktree
        .git_branch
        .as_deref()
        .or(worktree.vcs_ref.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("legacy sandbox worktree is missing branch metadata"))
}

async fn move_git_worktree(workspace_root: &Path, from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating canonical worktree parent dir")?;
    }

    if tokio::fs::metadata(to).await.is_ok() {
        if super::is_git_worktree(to).await? {
            bail!(
                "canonical managed worktree root already exists at {}",
                to.display()
            );
        }
        tokio::fs::remove_dir_all(to)
            .await
            .with_context(|| format!("removing stale canonical worktree root {}", to.display()))?;
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("worktree")
        .arg("move")
        .arg(from)
        .arg(to)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree move")?;
    if !output.status.success() {
        bail!(
            "git worktree move failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn normalize_legacy_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    root_kind: Option<LegacySandboxRootKind>,
) -> Result<Worktree> {
    let canonical_root = managed_worktree_path(&state.core.data_root, workspace.id, worktree.id);
    let canonical_root_string = canonical_root.to_string_lossy().to_string();
    let current_root = PathBuf::from(&worktree.root_path);
    let branch_name = legacy_branch_name(worktree)?;

    if matches!(root_kind, Some(LegacySandboxRootKind::AvfShadowRoot)) {
        super::ensure_worktree_attached(
            &workspace.root_path,
            &canonical_root,
            &worktree.base_commit_sha,
            branch_name,
        )
        .await
        .with_context(|| {
            format!(
                "reattaching legacy AVF shadow worktree {} into canonical managed root",
                worktree.id.0
            )
        })?;
        if tokio::fs::metadata(&current_root).await.is_ok() {
            tokio::fs::remove_dir_all(&current_root)
                .await
                .with_context(|| {
                    format!(
                        "removing legacy AVF shadow worktree {} at {}",
                        worktree.id.0,
                        current_root.display()
                    )
                })?;
        }
    } else if current_root == canonical_root {
        super::ensure_worktree_attached(
            &workspace.root_path,
            &canonical_root,
            &worktree.base_commit_sha,
            branch_name,
        )
        .await
        .with_context(|| {
            format!(
                "reattaching canonical legacy sandbox worktree {}",
                worktree.id.0
            )
        })?;
    } else if tokio::fs::metadata(&canonical_root).await.is_ok()
        && super::is_git_worktree(&canonical_root).await?
    {
        if tokio::fs::metadata(&current_root).await.is_ok() {
            bail!(
                "legacy sandbox worktree {} has both legacy root {} and canonical root {}",
                worktree.id.0,
                current_root.display(),
                canonical_root.display()
            );
        }
    } else if tokio::fs::metadata(&current_root).await.is_ok() {
        if !super::is_git_worktree(&current_root).await? {
            bail!(
                "legacy sandbox worktree root exists but is not a git worktree: {}",
                current_root.display()
            );
        }
        move_git_worktree(
            &PathBuf::from(&workspace.root_path),
            &current_root,
            &canonical_root,
        )
        .await
        .with_context(|| {
            format!(
                "moving legacy sandbox worktree {} into canonical managed root",
                worktree.id.0
            )
        })?;
    } else {
        super::ensure_worktree_attached(
            &workspace.root_path,
            &canonical_root,
            &worktree.base_commit_sha,
            branch_name,
        )
        .await
        .with_context(|| {
            format!(
                "reattaching legacy sandbox worktree {} into canonical managed root",
                worktree.id.0
            )
        })?;
    }

    if worktree.root_path == canonical_root_string {
        return Ok(worktree.clone());
    }

    let mut updated = worktree.clone();
    updated.root_path = canonical_root_string;
    Ok(updated)
}

async fn backfill_worktree_binding(
    state: &AppState,
    workspace: &Workspace,
    store: &Store,
    worktree: &Worktree,
    sessions: &[ctx_core::models::Session],
) -> Result<Option<BackfilledSandboxWorktree>> {
    let root_kind = legacy_sandbox_root_kind(&state.core.data_root, workspace.id, worktree);
    let host_sessions = sessions
        .iter()
        .filter(|session| matches!(session.execution_environment, ExecutionEnvironment::Host))
        .count();
    let sandbox_sessions = sessions
        .iter()
        .filter(|session| matches!(session.execution_environment, ExecutionEnvironment::Sandbox))
        .count();
    if host_sessions > 0 && sandbox_sessions > 0 {
        bail!(
            "legacy sandbox worktree {} has mixed host and sandbox sessions; manual repair is required",
            worktree.id.0
        );
    }
    if host_sessions > 0 && root_kind.is_some() {
        bail!(
            "legacy sandbox worktree {} uses a sandbox-only legacy root but only host sessions; manual repair is required",
            worktree.id.0
        );
    }

    let repaired_worktree =
        normalize_legacy_worktree_root(state, workspace, worktree, root_kind).await?;

    let settings_data = settings::load_settings(state.global_store())
        .await
        .context("loading daemon settings for legacy sandbox backfill")?;
    let mut effective = settings_data.execution.clone().unwrap_or_default();
    if let Some(ov) = workspace_config::load_execution_settings_override(store)
        .await
        .context("loading workspace execution override for legacy sandbox backfill")?
    {
        workspace_config::apply_execution_settings_override(&mut effective, &ov);
    }
    execution_effective::apply_execution_environment(&mut effective, ExecutionEnvironment::Sandbox);
    effective.mode = ExecutionMode::Sandbox;
    effective.container.mount_mode = ContainerMountMode::DiskIsolated;
    // Legacy beta worktrees only persisted host-vs-sandbox, not a full immutable runtime snapshot.
    // Freeze them to the platform's canonical sandbox runtime at migration time so the migrated
    // binding becomes authoritative going forward instead of continuing to drift with workspace
    // defaults.
    effective.container.runtime = if cfg!(target_os = "macos") {
        crate::settings::ContainerRuntimeKind::SharedVmContainer
    } else {
        crate::settings::ContainerRuntimeKind::NativeContainer
    };

    let canonical_root = PathBuf::from(&repaired_worktree.root_path);
    let binding = super::materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        &repaired_worktree,
        &canonical_root,
        &effective,
        legacy_binding_created_at(worktree, sessions),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("legacy sandbox backfill unexpectedly resolved to host"))?;
    Ok(Some(BackfilledSandboxWorktree {
        worktree: repaired_worktree,
        binding,
    }))
}

async fn reconcile_worktree_session_execution_environment(
    store: &Store,
    worktree_id: ctx_core::ids::WorktreeId,
    execution_environment: ExecutionEnvironment,
) -> Result<usize> {
    let sessions = store
        .list_sessions_for_worktree(worktree_id)
        .await
        .with_context(|| format!("listing sessions for worktree {}", worktree_id.0))?;
    let mut repaired = 0;
    for session in sessions {
        if session.execution_environment == execution_environment {
            continue;
        }
        store
            .update_session_execution_environment(session.id, execution_environment)
            .await
            .with_context(|| {
                format!(
                    "repairing session {} execution_environment for worktree {}",
                    session.id.0, worktree_id.0
                )
            })?;
        repaired += 1;
    }
    Ok(repaired)
}

pub(crate) async fn ensure_workspace_sandbox_bindings_backfilled(
    state: &AppState,
    workspace: &Workspace,
    store: &Store,
) -> Result<SandboxBindingBackfillStats> {
    let mut stats = SandboxBindingBackfillStats::default();
    let worktrees = store
        .list_worktrees(workspace.id)
        .await
        .with_context(|| format!("listing worktrees for workspace {}", workspace.id.0))?;

    for worktree in worktrees {
        stats.scanned_worktrees += 1;
        if store
            .get_sandbox_binding(worktree.id)
            .await
            .with_context(|| format!("loading sandbox binding for worktree {}", worktree.id.0))?
            .is_some()
        {
            continue;
        }
        let sessions = store
            .list_sessions_for_worktree(worktree.id)
            .await
            .with_context(|| format!("listing sessions for worktree {}", worktree.id.0))?;
        let mut binding = store
            .get_sandbox_binding(worktree.id)
            .await
            .with_context(|| format!("loading sandbox binding for worktree {}", worktree.id.0))?;
        if binding.is_none()
            && is_legacy_sandbox_candidate(
                &state.core.data_root,
                workspace.id,
                &worktree,
                &sessions,
            )
        {
            let repaired =
                backfill_worktree_binding(state, workspace, store, &worktree, &sessions).await?;
            if let Some(repaired) = repaired.as_ref() {
                store
                    .upsert_sandbox_binding(repaired.binding.clone())
                    .await
                    .with_context(|| {
                        format!("saving sandbox binding for worktree {}", worktree.id.0)
                    })?;
                if repaired.worktree.root_path != worktree.root_path {
                    store
                        .update_worktree_root_path(worktree.id, &repaired.worktree.root_path)
                        .await
                        .with_context(|| {
                            format!(
                                "updating legacy sandbox worktree {} root path to canonical managed root",
                                worktree.id.0
                            )
                        })?;
                }
                stats.repaired_bindings += 1;
                binding = Some(repaired.binding.clone());
            }
        }
        if binding.is_some() {
            stats.repaired_session_metadata += reconcile_worktree_session_execution_environment(
                store,
                worktree.id,
                ExecutionEnvironment::Sandbox,
            )
            .await?;
        }
    }

    Ok(stats)
}

#[cfg(test)]
#[path = "backfill/tests.rs"]
mod tests;
