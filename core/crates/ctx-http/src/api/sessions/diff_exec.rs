use super::*;

enum SandboxExecTarget {
    Podman { container_name: String },
    AvfLinuxVm,
}

async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> anyhow::Result<SandboxExecTarget> {
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    if matches!(
        effective.container.runtime,
        crate::settings::ContainerRuntimeKind::AvfLinuxVm
    ) {
        Ok(SandboxExecTarget::AvfLinuxVm)
    } else {
        Ok(SandboxExecTarget::Podman {
            container_name: workspace_container_name(worktree.workspace_id),
        })
    }
}

async fn container_exec_stdout(
    state: &Arc<AppState>,
    worktree: &Worktree,
    program: &str,
    args: &[&str],
) -> anyhow::Result<Vec<u8>> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
    let target = ensure_container_for_worktree(state, worktree).await?;
    let out = match target {
        SandboxExecTarget::Podman { container_name } => {
            let mut cmd = podman_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(&worktree.root_path)
                .arg(&container_name)
                .arg(program)
                .args(args);
            command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
                .await
                .context("podman exec command timed out")?
        }
        SandboxExecTarget::AvfLinuxVm => {
            crate::workspace_runtime::run_avf_linux_guest_exec_capture(
                &state.core.data_root,
                worktree.workspace_id,
                worktree.id,
                StdPath::new(&worktree.root_path),
                program,
                &args
                    .iter()
                    .map(|arg| (*arg).to_string())
                    .collect::<Vec<_>>(),
                &std::collections::HashMap::new(),
                None,
                false,
            )
            .await
            .context("AVF guest exec command failed")?
        }
    };
    if out.status.success() {
        Ok(out.stdout)
    } else {
        anyhow::bail!("{} {:?} failed: {}", program, args, {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                "unknown sandbox exec failure".to_string()
            }
        });
    }
}

async fn container_diff_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    // Match host behavior by including untracked files, with the same large-file omission rule.
    let script = r#"
set -euo pipefail
base="$1"
git diff "$base"
max_bytes=$((512 * 1024))
while IFS= read -r f; do
  [ -z "$f" ] && continue
  size="$(stat -c %s -- "$f" 2>/dev/null || echo 0)"
  case "$size" in
    ''|*[!0-9]*) size=0 ;;
  esac
  if [ "$size" -gt "$max_bytes" ]; then
    printf '\n# untracked: %s (%s bytes; omitted)\n' "$f" "$size"
    continue
  fi
  patch="$(git diff --no-index -- /dev/null "$f" || true)"
  if [ -n "$patch" ]; then
    printf '\n%s\n' "$patch"
  fi
done < <(git ls-files --others --exclude-standard)
"#;
    let bytes = container_exec_stdout(
        state,
        worktree,
        "bash",
        &["-lc", script, "--", base_commit_sha],
    )
    .await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

async fn container_diff_worktree_summary(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<(i64, i64, i64)> {
    // Match host behavior by including untracked files and best-effort line counts for small files.
    let script = r#"
set -euo pipefail
base="$1"
file_count=0
additions=0
deletions=0
while IFS=$'\t' read -r add del path; do
  [ -z "$path" ] && continue
  file_count=$((file_count+1))
  if [ "$add" != "-" ]; then
    additions=$((additions+add))
  fi
  if [ "$del" != "-" ]; then
    deletions=$((deletions+del))
  fi
done < <(git diff --numstat "$base")

max_bytes=$((512 * 1024))
while IFS= read -r f; do
  [ -z "$f" ] && continue
  file_count=$((file_count+1))
  size="$(stat -c %s -- "$f" 2>/dev/null || echo 0)"
  case "$size" in
    ''|*[!0-9]*) size=0 ;;
  esac
  if [ "$size" -gt "$max_bytes" ]; then
    continue
  fi
  lines="$(awk 'END{print NR}' -- "$f" 2>/dev/null || echo 0)"
  case "$lines" in
    ''|*[!0-9]*) lines=0 ;;
  esac
  additions=$((additions+lines))
done < <(git ls-files --others --exclude-standard)

printf '%s %s %s\n' "$file_count" "$additions" "$deletions"
"#;
    let bytes = container_exec_stdout(
        state,
        worktree,
        "bash",
        &["-lc", script, "--", base_commit_sha],
    )
    .await?;
    let output = String::from_utf8_lossy(&bytes);
    let mut parts = output.split_whitespace();
    let file_count = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let additions = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let deletions = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    Ok((file_count, additions, deletions))
}

pub(super) async fn diff_worktree_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    if is_container_path(StdPath::new(&worktree.root_path)) {
        return container_diff_worktree(state, worktree, base_commit_sha).await;
    }
    ctx_fs::worktrees::diff_worktree(&worktree.root_path, base_commit_sha).await
}

pub(super) async fn diff_worktree_summary_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<(i64, i64, i64)> {
    if is_container_path(StdPath::new(&worktree.root_path)) {
        return container_diff_worktree_summary(state, worktree, base_commit_sha).await;
    }
    ctx_fs::worktrees::diff_worktree_summary(&worktree.root_path, base_commit_sha).await
}
