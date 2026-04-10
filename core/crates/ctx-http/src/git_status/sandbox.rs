use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use ctx_core::models::Worktree;
use ctx_fs::vcs;
use ctx_workspace_container::workspace_container_name;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_harness_runtime::sandbox_container_command;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

enum SandboxGitTarget {
    NativeContainer { container_name: String },
    SharedVmContainer,
}

struct SandboxGitContext {
    live_worktree_root: PathBuf,
    target: SandboxGitTarget,
}

async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<SandboxGitContext> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective =
        execution_effective::effective_execution_settings(state, data_plane.workspace.id).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &data_plane.workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::SharedVmContainer
    ) {
        Ok(SandboxGitContext {
            live_worktree_root: data_plane.live_worktree_root,
            target: SandboxGitTarget::SharedVmContainer,
        })
    } else {
        Ok(SandboxGitContext {
            live_worktree_root: data_plane.live_worktree_root,
            target: SandboxGitTarget::NativeContainer {
                container_name: workspace_container_name(worktree.workspace_id),
            },
        })
    }
}

async fn container_git_output(
    state: &Arc<AppState>,
    worktree: &Worktree,
    args: &[&str],
) -> Result<std::process::Output> {
    const SANDBOX_GIT_TIMEOUT: Duration = Duration::from_secs(30);
    let context = ensure_container_for_worktree(state, worktree).await?;
    match context.target {
        SandboxGitTarget::NativeContainer { container_name } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(&context.live_worktree_root)
                .arg(&container_name)
                .arg("git")
                .args(args);
            ctx_sandbox_container_runtime::command_output_with_timeout(cmd, SANDBOX_GIT_TIMEOUT)
                .await
                .context("sandbox exec git timed out")
        }
        SandboxGitTarget::SharedVmContainer => {
            let guest_args = args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            tokio::time::timeout(
                SANDBOX_GIT_TIMEOUT,
                ctx_avf_linux_runtime::run_guest_exec_capture(
                    &state.core.data_root,
                    worktree.workspace_id,
                    worktree.id,
                    &context.live_worktree_root,
                    "git",
                    &guest_args,
                    &HashMap::new(),
                    None,
                    false,
                ),
            )
            .await
            .context("shared VM container exec git timed out")?
        }
    }
}

pub(super) async fn container_git_stdout(
    state: &Arc<AppState>,
    worktree: &Worktree,
    args: &[&str],
) -> Result<Vec<u8>> {
    let out = container_git_output(state, worktree, args).await?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

pub(crate) async fn container_git_status_short(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<String> {
    let bytes =
        container_git_stdout(state, worktree, &["status", "-sb", "--untracked-files=all"]).await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub(crate) async fn container_git_status_porcelain(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<Vec<String>> {
    let bytes = container_git_stdout(state, worktree, &["status", "--porcelain", "-z"]).await?;
    let mut out = Vec::new();
    for entry in bytes.split(|b| *b == 0) {
        if entry.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(entry).to_string());
    }
    Ok(out)
}

pub(crate) async fn container_git_list_untracked(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<Vec<String>> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    let mut out = Vec::new();
    for part in bytes.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        out.push(String::from_utf8_lossy(part).to_string());
    }
    Ok(out)
}

pub(crate) async fn container_git_diff_name_status(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, String, Option<String>)>> {
    let bytes = container_git_stdout(
        state,
        worktree,
        &["diff", "--name-status", "-z", base_commit_sha],
    )
    .await?;
    let mut out = Vec::new();
    let mut parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .peekable();
    while let Some(part) = parts.next() {
        let Some(tab_idx) = part.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let status = String::from_utf8_lossy(&part[..tab_idx]).to_string();
        let path = String::from_utf8_lossy(&part[tab_idx + 1..]).to_string();
        if status.is_empty() || path.trim().is_empty() {
            continue;
        }
        let status_char = status.chars().next().unwrap_or('M');
        if status_char == 'R' || status_char == 'C' {
            let Some(next_path) = parts.next() else {
                continue;
            };
            let new_path = String::from_utf8_lossy(next_path).to_string();
            if new_path.trim().is_empty() {
                continue;
            }
            out.push((status, new_path, Some(path)));
        } else {
            out.push((status, path, None));
        }
    }
    Ok(out)
}

pub(crate) async fn container_git_rev_parse(
    state: &Arc<AppState>,
    worktree: &Worktree,
    reference: &str,
) -> Result<String> {
    let bytes = container_git_stdout(state, worktree, &["rev-parse", reference]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

async fn container_git_merge_base(
    state: &Arc<AppState>,
    worktree: &Worktree,
    target_branch: &str,
) -> Result<String> {
    let bytes =
        container_git_stdout(state, worktree, &["merge-base", target_branch, "HEAD"]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

pub(crate) async fn worktree_rev_parse_head(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<String> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_git_rev_parse(state, worktree, "HEAD").await;
    }
    let root = data_plane.live_worktree_root.as_path();
    let driver = vcs::driver_for_path(root).await?;
    driver.rev_parse_head(root).await
}

pub(crate) async fn worktree_merge_base(
    state: &Arc<AppState>,
    worktree: &Worktree,
    target_branch: &str,
) -> Result<String> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_git_merge_base(state, worktree, target_branch).await;
    }
    let root = data_plane.live_worktree_root.as_path();
    let driver = vcs::driver_for_path(root).await?;
    driver.merge_base(root, target_branch, "HEAD").await
}

async fn container_untracked_summary(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<(i64, i64)> {
    // Match host semantics in `ctx_fs::worktrees::diff_worktree_summary`:
    // - count untracked files as changed files
    // - include a best-effort line count for "small" untracked files
    let script = r#"
set -e
max_bytes=$((512 * 1024))
count=0
adds=0
while IFS= read -r f; do
  [ -z "$f" ] && continue
  count=$((count+1))
  # Skip huge files.
  size="$(stat -c %s -- "$f" 2>/dev/null || echo 0)"
  case "$size" in
    ''|*[!0-9]*) size=0 ;;
  esac
  if [ "$size" -gt "$max_bytes" ]; then
    continue
  fi
  # awk counts a final non-newline-terminated line as 1.
  lines="$(awk 'END{print NR}' -- "$f" 2>/dev/null || echo 0)"
  case "$lines" in
    ''|*[!0-9]*) lines=0 ;;
  esac
  adds=$((adds+lines))
done < <(git ls-files --others --exclude-standard)
printf '%s %s\n' "$count" "$adds"
"#;
    let target = ensure_container_for_worktree(state, worktree).await?;
    let out = match target.target {
        SandboxGitTarget::NativeContainer { container_name } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(&target.live_worktree_root)
                .arg(&container_name)
                .arg("bash")
                .arg("-lc")
                .arg(script);
            tokio::time::timeout(Duration::from_secs(30), cmd.output())
                .await
                .context("sandbox exec timed out")??
        }
        SandboxGitTarget::SharedVmContainer => tokio::time::timeout(
            Duration::from_secs(30),
            ctx_avf_linux_runtime::run_guest_exec_capture(
                &state.core.data_root,
                worktree.workspace_id,
                worktree.id,
                &target.live_worktree_root,
                "bash",
                &["-lc".to_string(), script.to_string()],
                &HashMap::new(),
                None,
                false,
            ),
        )
        .await
        .context("AVF guest exec timed out")??,
    };
    if !out.status.success() {
        anyhow::bail!(
            "untracked summary failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    let mut parts = txt.split_whitespace();
    let count = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let adds = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    Ok((count, adds))
}

pub(crate) async fn container_diff_worktree_summary(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<(i64, i64, i64)> {
    let bytes =
        container_git_stdout(state, worktree, &["diff", "--numstat", base_commit_sha]).await?;
    let stdout = String::from_utf8_lossy(&bytes);
    let mut file_count = 0i64;
    let mut additions = 0i64;
    let mut deletions = 0i64;
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let add = parts.next().unwrap_or("0");
        let del = parts.next().unwrap_or("0");
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        file_count += 1;
        if add != "-" {
            additions += add.parse::<i64>().unwrap_or(0);
        }
        if del != "-" {
            deletions += del.parse::<i64>().unwrap_or(0);
        }
    }
    let (untracked_count, untracked_additions) = container_untracked_summary(state, worktree)
        .await
        .unwrap_or((0, 0));
    file_count += untracked_count;
    additions += untracked_additions;
    Ok((file_count, additions, deletions))
}
