use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;

use ctx_core::models::{
    Worktree, WorktreeVcsBaseResolution, WorktreeVcsComputeState, WorktreeVcsFreshness,
    WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFile,
    WorktreeVcsTouchedFiles,
};
use ctx_fs::patch::should_ignore_path;
use ctx_fs::vcs::{self, VcsDriver};

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::container_fs::is_container_path;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::harness_runtime::{podman_command, workspace_container_name};
use crate::settings::ContainerRuntimeKind;
mod parse;
use parse::{parse_git_status_entries, parse_git_status_short};

const GIT_STATUS_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_MAX_INTERVAL_MS: u64 = 2000;
const GIT_STATUS_WATCH_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_POLL_INTERVAL_MS: u64 = 60_000;
const WORKTREE_VCS_SUMMARY_DEBOUNCE_MS: u64 = 750;
const WORKTREE_VCS_TOUCHED_FILES_CAP: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSnapshot {
    pub raw: String,
    pub summary_line: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub entries: Vec<GitStatusEntry>,
    pub entries_total_count: i64,
    pub entries_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

fn vcs_driver_for_worktree(worktree: &Worktree) -> Arc<dyn VcsDriver> {
    vcs::driver_for_kind(worktree.vcs_kind.clone())
}

enum SandboxGitTarget {
    Podman { container_name: String },
    AvfLinuxVm,
}

async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<SandboxGitTarget> {
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    state
        .execution
        .harness
        .ensure_workspace_container(&workspace, &effective, &state.core.daemon_url)
        .await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::AvfLinuxVm
    ) {
        Ok(SandboxGitTarget::AvfLinuxVm)
    } else {
        Ok(SandboxGitTarget::Podman {
            container_name: workspace_container_name(worktree.workspace_id),
        })
    }
}

async fn container_git_output(
    state: &Arc<AppState>,
    worktree: &Worktree,
    args: &[&str],
) -> Result<std::process::Output> {
    const SANDBOX_GIT_TIMEOUT: Duration = Duration::from_secs(30);
    let target = ensure_container_for_worktree(state, worktree).await?;
    match target {
        SandboxGitTarget::Podman { container_name } => {
            let mut cmd = podman_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(&worktree.root_path)
                .arg(&container_name)
                .arg("git")
                .args(args);
            crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_GIT_TIMEOUT)
                .await
                .context("podman exec git timed out")
        }
        SandboxGitTarget::AvfLinuxVm => {
            let guest_args = args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            tokio::time::timeout(
                SANDBOX_GIT_TIMEOUT,
                crate::workspace_runtime::run_avf_linux_guest_exec_capture(
                    &state.core.data_root,
                    worktree.workspace_id,
                    worktree.id,
                    Path::new(&worktree.root_path),
                    "git",
                    &guest_args,
                    &HashMap::new(),
                    None,
                    false,
                ),
            )
            .await
            .context("AVF guest exec git timed out")?
        }
    }
}

async fn container_git_stdout(
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

async fn container_git_status_short(state: &Arc<AppState>, worktree: &Worktree) -> Result<String> {
    let bytes =
        container_git_stdout(state, worktree, &["status", "-sb", "--untracked-files=all"]).await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

async fn container_git_status_porcelain(
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

async fn container_git_list_untracked(
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

async fn container_git_diff_name_status(
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

async fn container_git_rev_parse(
    state: &Arc<AppState>,
    worktree: &Worktree,
    reference: &str,
) -> Result<String> {
    let bytes = container_git_stdout(state, worktree, &["rev-parse", reference]).await?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
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
    let out = match target {
        SandboxGitTarget::Podman { container_name } => {
            let mut cmd = podman_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(&worktree.root_path)
                .arg(&container_name)
                .arg("bash")
                .arg("-lc")
                .arg(script);
            tokio::time::timeout(Duration::from_secs(30), cmd.output())
                .await
                .context("podman exec timed out")??
        }
        SandboxGitTarget::AvfLinuxVm => tokio::time::timeout(
            Duration::from_secs(30),
            crate::workspace_runtime::run_avf_linux_guest_exec_capture(
                &state.core.data_root,
                worktree.workspace_id,
                worktree.id,
                Path::new(&worktree.root_path),
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

async fn container_diff_worktree_summary(
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

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

fn snapshot_fingerprint(snapshot: &WorktreeVcsSnapshot) -> String {
    let mut copy = snapshot.clone();
    copy.rev = 0;
    copy.emitted_at_ms = 0;
    serde_json::to_string(&copy).unwrap_or_default()
}

fn build_touched_files(entries: &[WorktreeVcsTouchedFile]) -> WorktreeVcsTouchedFiles {
    let total_count = entries.len() as i64;
    let truncated = entries.len() > WORKTREE_VCS_TOUCHED_FILES_CAP;
    let mut items = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        items.push(entry.clone());
    }
    WorktreeVcsTouchedFiles {
        items,
        truncated,
        total_count: Some(total_count),
    }
}

fn build_git_status_entries(entries: &[GitStatusEntry]) -> Vec<WorktreeVcsTouchedFile> {
    let mut out = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        out.push(WorktreeVcsTouchedFile {
            path: entry.path.clone(),
            orig_path: entry.orig_path.clone(),
            index_status: Some(entry.index_status.clone()),
            worktree_status: Some(entry.worktree_status.clone()),
        });
    }
    out
}

fn build_git_status_summary(
    snapshot: &GitStatusSnapshot,
    entries: Vec<WorktreeVcsTouchedFile>,
) -> WorktreeVcsGitStatusSummary {
    WorktreeVcsGitStatusSummary {
        raw: snapshot.raw.clone(),
        summary_line: snapshot.summary_line.clone(),
        branch: snapshot.branch.clone(),
        upstream: snapshot.upstream.clone(),
        ahead: snapshot.ahead,
        behind: snapshot.behind,
        detached: snapshot.detached,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked: snapshot.untracked,
        entries,
    }
}

async fn load_diff_touched_entries(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<WorktreeVcsTouchedFile>> {
    let root = Path::new(&worktree.root_path);
    let entries: Vec<(String, String, Option<String>)> = if is_container_path(root) {
        container_git_diff_name_status(state, worktree, base_commit_sha).await?
    } else {
        let driver = vcs_driver_for_worktree(worktree);
        driver
            .diff_name_status(root, base_commit_sha)
            .await?
            .into_iter()
            .map(|entry| (entry.status, entry.path, entry.orig_path))
            .collect()
    };
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for (status, path, orig_path) in entries {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let status = status.chars().next().unwrap_or('M').to_string();
        items.push(WorktreeVcsTouchedFile {
            path,
            orig_path,
            index_status: Some(status),
            worktree_status: None,
        });
    }
    let untracked = if is_container_path(root) {
        container_git_list_untracked(state, worktree)
            .await
            .unwrap_or_default()
    } else {
        let driver = vcs_driver_for_worktree(worktree);
        driver.list_untracked(root).await.unwrap_or_default()
    };
    for path in untracked {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        items.push(WorktreeVcsTouchedFile {
            path,
            orig_path: None,
            index_status: Some("?".to_string()),
            worktree_status: None,
        });
    }
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
async fn build_worktree_vcs_snapshot_from_parts(
    state: &Arc<AppState>,
    worktree: &Worktree,
    git_status: WorktreeVcsGitStatusSummary,
    touched_files: WorktreeVcsTouchedFiles,
    summary: WorktreeVcsSummary,
    compute_state: WorktreeVcsComputeState,
    resolution: Option<crate::api::sessions::WorktreeDiffBaseResolution>,
    available: bool,
    unavailable_reason: Option<ctx_core::models::DiffUnavailableReason>,
) -> Result<WorktreeVcsSnapshot> {
    let resolution = match resolution {
        Some(resolution) => resolution,
        None => {
            let workspace = state
                .global_store()
                .get_workspace(worktree.workspace_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
            let store = state.store_for_worktree(worktree.id).await?;
            resolve_diff_base_with_meta(&store, &workspace, worktree, &SessionDiffQuery::default())
                .await
        }
    };
    let base_commit_sha = resolution.base_commit_sha.clone();
    let root = Path::new(&worktree.root_path);
    let (head_commit_sha, target_branch_commit_sha) = if is_container_path(root) {
        let head = container_git_rev_parse(state, worktree, "HEAD")
            .await
            .unwrap_or_else(|_| base_commit_sha.clone());
        let target = match resolution.target_branch.as_ref() {
            Some(target_branch) => container_git_rev_parse(state, worktree, target_branch)
                .await
                .ok(),
            None => None,
        };
        (head, target)
    } else {
        let driver = vcs::driver_for_path(root).await.ok();
        let head = match driver.as_ref() {
            Some(driver) => driver
                .rev_parse_head(root)
                .await
                .unwrap_or_else(|_| base_commit_sha.clone()),
            None => base_commit_sha.clone(),
        };
        let target = match (driver.as_ref(), resolution.target_branch.as_ref()) {
            (Some(driver), Some(target_branch)) => {
                driver.rev_parse_ref(root, target_branch).await.ok()
            }
            _ => None,
        };
        (head, target)
    };
    let base_resolution = WorktreeVcsBaseResolution {
        kind: resolution.kind,
        target_source: resolution.target_source,
        error: resolution.error,
    };
    let freshness = derive_worktree_vcs_freshness(&compute_state, &summary);
    Ok(WorktreeVcsSnapshot {
        worktree_id: worktree.id,
        rev: 0,
        emitted_at_ms: 0,
        base_commit_sha,
        head_commit_sha,
        target_branch: resolution.target_branch,
        target_branch_commit_sha,
        base_resolution,
        compute_state,
        summary,
        git_status,
        touched_files,
        freshness,
        available,
        unavailable_reason,
        schema_version: 1,
    })
}

async fn publish_no_repo_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    resolution: crate::api::sessions::WorktreeDiffBaseResolution,
    force_emit: bool,
) -> Result<()> {
    publish_unavailable_snapshot(
        state,
        worktree,
        resolution,
        force_emit,
        ctx_core::models::DiffUnavailableReason::NoRepo,
    )
    .await
}

async fn publish_unavailable_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    resolution: crate::api::sessions::WorktreeDiffBaseResolution,
    force_emit: bool,
    reason: ctx_core::models::DiffUnavailableReason,
) -> Result<()> {
    let snapshot = build_worktree_vcs_snapshot_from_parts(
        state,
        worktree,
        WorktreeVcsGitStatusSummary::default(),
        WorktreeVcsTouchedFiles::default(),
        WorktreeVcsSummary::default(),
        WorktreeVcsComputeState::Ready,
        Some(resolution),
        false,
        Some(reason),
    )
    .await?;
    if let Some(snapshot) = upsert_worktree_vcs_snapshot(state, snapshot, force_emit, None).await {
        if state.is_worktree_vcs_active(worktree.id).await {
            state
                .workspaces
                .workspace_active_snapshot
                .publish_worktree_vcs_snapshot(worktree.workspace_id, snapshot)
                .await;
        }
    }
    Ok(())
}

fn summary_from_counts(
    file_count: i64,
    line_additions: i64,
    line_deletions: i64,
) -> WorktreeVcsSummary {
    let line_count = line_additions + line_deletions;
    WorktreeVcsSummary {
        file_count: Some(file_count),
        line_additions: Some(line_additions),
        line_deletions: Some(line_deletions),
        line_count: Some(line_count),
    }
}

fn summary_has_counts(summary: &WorktreeVcsSummary) -> bool {
    summary.file_count.is_some()
        || summary.line_additions.is_some()
        || summary.line_deletions.is_some()
        || summary.line_count.is_some()
}

fn derive_worktree_vcs_freshness(
    compute_state: &WorktreeVcsComputeState,
    summary: &WorktreeVcsSummary,
) -> WorktreeVcsFreshness {
    match compute_state {
        WorktreeVcsComputeState::Ready => WorktreeVcsFreshness::Fresh,
        WorktreeVcsComputeState::Error => WorktreeVcsFreshness::Error,
        WorktreeVcsComputeState::Computing => {
            if summary_has_counts(summary) {
                WorktreeVcsFreshness::Stale
            } else {
                WorktreeVcsFreshness::Refreshing
            }
        }
    }
}

pub async fn load_git_status_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<GitStatusSnapshot> {
    let root = Path::new(&worktree.root_path);
    let status_text = if is_container_path(root) {
        container_git_status_short(state, worktree).await?
    } else {
        let vcs = vcs_driver_for_worktree(worktree);
        vcs.status_short(root).await?
    };
    let branch_info = parse_git_status_short(&status_text);
    let entries = if is_container_path(root) {
        container_git_status_porcelain(state, worktree).await?
    } else {
        let vcs = vcs_driver_for_worktree(worktree);
        vcs.status_porcelain(root).await?
    };
    let parsed_entries = parse_git_status_entries(&entries);
    Ok(GitStatusSnapshot {
        raw: status_text,
        summary_line: branch_info.summary_line,
        branch: branch_info.branch,
        upstream: branch_info.upstream,
        ahead: branch_info.ahead,
        behind: branch_info.behind,
        detached: branch_info.detached,
        staged: parsed_entries.staged,
        unstaged: parsed_entries.unstaged,
        untracked: parsed_entries.untracked,
        entries: parsed_entries.entries,
        entries_total_count: parsed_entries.total_count,
        entries_truncated: parsed_entries.truncated,
    })
}

async fn upsert_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    mut snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let now = Instant::now();
    let fingerprint = snapshot_fingerprint(&snapshot);
    let active = state.workspaces.worktree_vcs_active.lock().await;
    if active.get(&snapshot.worktree_id).copied().unwrap_or(0) == 0 {
        return None;
    }
    let mut cache = state.workspaces.worktree_vcs_snapshots.lock().await;
    let entry = cache.entry(snapshot.worktree_id).or_insert_with(|| {
        crate::daemon::TimedEntry::new(crate::daemon::WorktreeVcsSnapshotCacheEntry {
            snapshot: snapshot.clone(),
            fingerprint: String::new(),
            emitted_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
            last_change_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
            last_summary_at: None,
        })
    });
    entry.touch_at(now);
    let is_first = entry.value.fingerprint.is_empty();
    if entry.value.fingerprint == fingerprint && !force_emit {
        return None;
    }
    let since_change = now.duration_since(entry.value.last_change_at);
    let since_emit = now.duration_since(entry.value.emitted_at);
    if !force_emit
        && !is_first
        && since_emit < Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS)
        && since_change < Duration::from_millis(GIT_STATUS_DEBOUNCE_MS)
    {
        return None;
    }
    let next_rev = entry.value.snapshot.rev.saturating_add(1);
    snapshot.rev = next_rev;
    snapshot.emitted_at_ms = now_epoch_ms();
    if snapshot.schema_version == 0 {
        snapshot.schema_version = 1;
    }
    entry.value.snapshot = snapshot.clone();
    entry.value.fingerprint = fingerprint;
    entry.value.emitted_at = now;
    entry.value.last_change_at = now;
    if let Some(summary_at) = summary_at {
        entry.value.last_summary_at = Some(summary_at);
    }
    Some(snapshot)
}

pub async fn refresh_worktree_vcs_summary(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let cached_summary = {
        let mut cache = state.workspaces.worktree_vcs_snapshots.lock().await;
        if let Some(entry) = cache.get_mut(&worktree.id) {
            entry.touch();
            entry.value.snapshot.summary.clone()
        } else {
            WorktreeVcsSummary::default()
        }
    };
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
    let store = state.store_for_worktree(worktree.id).await?;
    let resolution =
        resolve_diff_base_with_meta(&store, &workspace, &worktree, &SessionDiffQuery::default())
            .await;
    if let Some(reason) = resolution.unavailable_reason.clone() {
        return publish_unavailable_snapshot(&state, &worktree, resolution, false, reason).await;
    }
    let git_snapshot = match load_git_status_snapshot(&state, &worktree).await {
        Ok(snapshot) => snapshot,
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
            return publish_no_repo_snapshot(&state, &worktree, resolution, false).await;
        }
        Err(err) => return Err(err),
    };
    let git_status_entries = build_git_status_entries(&git_snapshot.entries);
    let git_status = build_git_status_summary(&git_snapshot, git_status_entries);
    let diff_entries =
        match load_diff_touched_entries(&state, &worktree, &resolution.base_commit_sha).await {
            Ok(entries) => entries,
            Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
                return publish_no_repo_snapshot(&state, &worktree, resolution, false).await;
            }
            Err(err) => return Err(err),
        };
    let touched_files = build_touched_files(&diff_entries);
    let summary_result = if is_container_path(Path::new(&worktree.root_path)) {
        container_diff_worktree_summary(&state, &worktree, &resolution.base_commit_sha).await
    } else {
        ctx_fs::worktrees::diff_worktree_summary(&worktree.root_path, &resolution.base_commit_sha)
            .await
    };
    let (summary, compute_state, summary_at, available, unavailable_reason) = match summary_result {
        Ok((file_count, line_additions, line_deletions)) => (
            summary_from_counts(file_count, line_additions, line_deletions),
            WorktreeVcsComputeState::Ready,
            Some(Instant::now()),
            true,
            None,
        ),
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => (
            WorktreeVcsSummary::default(),
            WorktreeVcsComputeState::Ready,
            None,
            false,
            Some(ctx_core::models::DiffUnavailableReason::NoRepo),
        ),
        Err(err) => {
            tracing::warn!(
                worktree_id = %worktree.id.0,
                "worktree diff summary failed: {err:#}"
            );
            (
                cached_summary,
                WorktreeVcsComputeState::Error,
                None,
                true,
                None,
            )
        }
    };
    let snapshot = build_worktree_vcs_snapshot_from_parts(
        &state,
        &worktree,
        git_status,
        touched_files,
        summary,
        compute_state,
        Some(resolution),
        available,
        unavailable_reason,
    )
    .await?;
    if let Some(snapshot) = upsert_worktree_vcs_snapshot(&state, snapshot, false, summary_at).await
    {
        if state.is_worktree_vcs_active(worktree.id).await {
            state
                .workspaces
                .workspace_active_snapshot
                .publish_worktree_vcs_snapshot(worktree.workspace_id, snapshot)
                .await;
        }
    }
    Ok(())
}

pub async fn schedule_worktree_vcs_summary_refresh(state: Arc<AppState>, worktree: Worktree) {
    let worktree_id = worktree.id;
    let generation = {
        let mut gens = state.workspaces.worktree_vcs_summary_gen.lock().await;
        let entry = gens.entry(worktree_id).or_insert(0);
        *entry += 1;
        *entry
    };
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(WORKTREE_VCS_SUMMARY_DEBOUNCE_MS)).await;
        let current = {
            let gens = state.workspaces.worktree_vcs_summary_gen.lock().await;
            gens.get(&worktree_id).copied().unwrap_or(0)
        };
        if current != generation {
            return;
        }
        if !state.is_worktree_vcs_active(worktree_id).await {
            return;
        }
        if let Err(err) = refresh_worktree_vcs_summary(state.clone(), worktree).await {
            tracing::warn!(
                worktree_id = %worktree_id.0,
                "worktree diff summary refresh failed: {err:#}"
            );
        }
    });
}

pub async fn emit_worktree_vcs_snapshot_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
    force_emit: bool,
) -> Result<()> {
    let active = state.is_worktree_vcs_active(worktree.id).await;
    if !active {
        return Ok(());
    }
    let store = state.store_for_worktree(worktree.id).await?;
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
    let resolution =
        resolve_diff_base_with_meta(&store, &workspace, worktree, &SessionDiffQuery::default())
            .await;
    if let Some(reason) = resolution.unavailable_reason.clone() {
        return publish_unavailable_snapshot(state, worktree, resolution, force_emit, reason).await;
    }
    let git_snapshot = match load_git_status_snapshot(state, worktree).await {
        Ok(snapshot) => snapshot,
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
            return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
        }
        Err(err) => return Err(err),
    };
    let git_status_entries = build_git_status_entries(&git_snapshot.entries);
    let git_status = build_git_status_summary(&git_snapshot, git_status_entries);
    let diff_entries =
        match load_diff_touched_entries(state, worktree, &resolution.base_commit_sha).await {
            Ok(entries) => entries,
            Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
                return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
            }
            Err(err) => return Err(err),
        };
    let touched_files = build_touched_files(&diff_entries);
    let (cached_summary, cached_available, cached_unavailable_reason) = {
        let cache = state.workspaces.worktree_vcs_snapshots.lock().await;
        cache
            .get(&worktree.id)
            .map(|entry| {
                (
                    entry.value.snapshot.summary.clone(),
                    entry.value.snapshot.available,
                    entry.value.snapshot.unavailable_reason.clone(),
                )
            })
            .unwrap_or((WorktreeVcsSummary::default(), true, None))
    };
    let compute_state = if active {
        WorktreeVcsComputeState::Computing
    } else {
        let has_summary = summary_has_counts(&cached_summary);
        if has_summary {
            WorktreeVcsComputeState::Ready
        } else {
            WorktreeVcsComputeState::Computing
        }
    };
    let snapshot = build_worktree_vcs_snapshot_from_parts(
        state,
        worktree,
        git_status,
        touched_files,
        cached_summary,
        compute_state,
        Some(resolution),
        cached_available,
        cached_unavailable_reason,
    )
    .await?;
    let mut published = false;
    if let Some(snapshot) = upsert_worktree_vcs_snapshot(state, snapshot, force_emit, None).await {
        if active {
            state
                .workspaces
                .workspace_active_snapshot
                .publish_worktree_vcs_snapshot(worktree.workspace_id, snapshot)
                .await;
            published = true;
        }
    }
    if active && published {
        schedule_worktree_vcs_summary_refresh(state.clone(), worktree.clone()).await;
    }
    Ok(())
}

pub async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let root = Path::new(&worktree.root_path);
    if is_container_path(root) {
        // Disk-isolated worktrees live inside the harness container; host filesystem watchers
        // cannot observe changes. Polling keeps VCS snapshots up to date.
        let _ = container_git_stdout(&state, &worktree, &["rev-parse", "--is-inside-work-tree"])
            .await?;
        return run_git_status_poller(state, worktree).await;
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    vcs.assert_repo(root).await?;

    let (tx, mut rx) = mpsc::channel::<()>(1);
    let mut watcher = watcher(tx)?;
    if let Err(err) = watcher.watch(root, RecursiveMode::Recursive) {
        // On hosts with low watch limits (or many concurrent watchers), file watching can fail with
        // ENOSPC/too-many-watches. Falling back to polling keeps git status updates flowing and
        // avoids flaking tests that rely on live status changes.
        tracing::warn!(
            worktree_id = %worktree.id.0,
            "git status watcher unavailable; falling back to polling: {err:#}"
        );
        return run_git_status_poller(state, worktree).await;
    }

    let debounce = Duration::from_millis(GIT_STATUS_WATCH_DEBOUNCE_MS);
    let mut pending = false;
    let timer = tokio::time::sleep(debounce);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            signal = rx.recv() => {
                let Some(()) = signal else {
                    break;
                };
                pending = true;
                timer.as_mut().reset(tokio::time::Instant::now() + debounce);
            }
            _ = &mut timer, if pending => {
                pending = false;
                if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, false).await {
                    tracing::warn!(worktree_id = %worktree.id.0, "git status snapshot failed: {err:#}");
                }
            }
        }
    }
    Ok(())
}

async fn run_git_status_poller(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(GIT_STATUS_POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, false).await {
            tracing::warn!(worktree_id = %worktree.id.0, "git status snapshot failed: {err:#}");
        }
    }
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

fn watcher(tx: mpsc::Sender<()>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            if should_ignore_event(&event) {
                return;
            }
            let _ = tx.try_send(());
        }
    })?;
    Ok(watcher)
}
