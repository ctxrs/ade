use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;

use ctx_core::ids::{SessionId, WorktreeId};
use ctx_core::models::{SessionEventType, SessionStatus, Worktree};
use ctx_fs::git::{assert_git_repo, git_status_porcelain, git_status_short};
use ctx_fs::patch::should_ignore_path;

use crate::daemon::AppState;

const GIT_STATUS_DEBOUNCE_MS: u64 = 1500;
const GIT_STATUS_WATCH_DEBOUNCE_MS: u64 = 500;

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
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug)]
struct GitStatusBranchInfo {
    summary_line: String,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: i64,
    behind: i64,
    detached: bool,
}

pub async fn load_git_status_snapshot(root: &Path) -> Result<GitStatusSnapshot> {
    let status_text = git_status_short(root).await?;
    let branch_info = parse_git_status_short(&status_text);
    let entries = git_status_porcelain(root).await?;
    let parsed_entries = parse_git_status_entries(&entries);
    let (staged, unstaged, untracked) = count_git_status_entries(&parsed_entries);
    Ok(GitStatusSnapshot {
        raw: status_text,
        summary_line: branch_info.summary_line,
        branch: branch_info.branch,
        upstream: branch_info.upstream,
        ahead: branch_info.ahead,
        behind: branch_info.behind,
        detached: branch_info.detached,
        staged,
        unstaged,
        untracked,
        entries: parsed_entries,
    })
}

pub async fn emit_git_status_snapshot_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<()> {
    let sessions = state.store.list_sessions_for_worktree(worktree.id).await?;
    let active_session_ids: Vec<SessionId> = sessions
        .into_iter()
        .filter(|session| matches!(session.status, SessionStatus::Active))
        .map(|session| session.id)
        .collect();
    if active_session_ids.is_empty() {
        return Ok(());
    }
    let snapshot = load_git_status_snapshot(Path::new(&worktree.root_path)).await?;
    emit_git_status_snapshot_for_sessions(state, &active_session_ids, worktree.id, &snapshot).await;
    Ok(())
}

pub async fn emit_git_status_snapshot_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
    worktree_id: WorktreeId,
    snapshot: &GitStatusSnapshot,
) {
    if session_ids.is_empty() {
        return;
    }
    let summary = serde_json::json!({
        "summary_line": snapshot.summary_line,
        "branch": snapshot.branch,
        "upstream": snapshot.upstream,
        "ahead": snapshot.ahead,
        "behind": snapshot.behind,
        "detached": snapshot.detached,
        "staged": snapshot.staged,
        "unstaged": snapshot.unstaged,
        "untracked": snapshot.untracked,
    });
    let payload = serde_json::json!({
        "kind": "git_status_snapshot",
        "worktree_id": worktree_id.0.to_string(),
        "summary": summary,
        "entries": &snapshot.entries,
    });
    let payload_raw = match serde_json::to_string(&payload) {
        Ok(value) => value,
        Err(_) => return,
    };
    let now = Instant::now();
    {
        let mut cache = state.git_status_snapshots.lock().await;
        let entry = cache.entry(worktree_id).or_insert_with(|| {
            crate::daemon::GitStatusSnapshotCacheEntry {
                payload: String::new(),
                emitted_at: now - Duration::from_millis(GIT_STATUS_DEBOUNCE_MS + 1),
            }
        });
        if entry.payload == payload_raw {
            return;
        }
        if now.duration_since(entry.emitted_at) < Duration::from_millis(GIT_STATUS_DEBOUNCE_MS) {
            return;
        }
        entry.payload = payload_raw;
        entry.emitted_at = now;
    }

    for session_id in session_ids {
        let notice = state
            .store
            .append_session_event(
                *session_id,
                None,
                None,
                SessionEventType::Notice,
                payload.clone(),
            )
            .await;
        if let Ok(event) = notice {
            state.publish_event(event).await;
        }
    }
}

pub async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let root = Path::new(&worktree.root_path);
    assert_git_repo(root).await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut watcher = watcher(tx)?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .context("watching worktree for git status")?;

    let debounce = Duration::from_millis(GIT_STATUS_WATCH_DEBOUNCE_MS);
    let mut pending = false;
    let timer = tokio::time::sleep(debounce);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                if should_ignore_event(&event) {
                    continue;
                }
                pending = true;
                timer.as_mut().reset(tokio::time::Instant::now() + debounce);
            }
            _ = &mut timer, if pending => {
                pending = false;
                if let Err(err) = emit_git_status_snapshot_for_worktree(&state, &worktree).await {
                    tracing::warn!(worktree_id = %worktree.id.0, "git status snapshot failed: {err:#}");
                }
            }
        }
    }
    Ok(())
}

fn parse_git_status_short(output: &str) -> GitStatusBranchInfo {
    let mut info = GitStatusBranchInfo {
        summary_line: String::new(),
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        detached: false,
    };
    let mut lines = output.lines();
    let Some(line) = lines.next() else {
        return info;
    };
    info.summary_line = line.trim().to_string();
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

fn count_git_status_entries(entries: &[GitStatusEntry]) -> (i64, i64, i64) {
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    for entry in entries {
        let index_status = entry.index_status.chars().next().unwrap_or(' ');
        let worktree_status = entry.worktree_status.chars().next().unwrap_or(' ');
        if index_status == '?' && worktree_status == '?' {
            untracked += 1;
            continue;
        }
        if index_status != ' ' {
            staged += 1;
        }
        if worktree_status != ' ' {
            unstaged += 1;
        }
    }
    (staged, unstaged, untracked)
}

fn parse_git_status_entries(entries: &[String]) -> Vec<GitStatusEntry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let raw = entries[i].trim_end();
        if raw.len() < 3 {
            i += 1;
            continue;
        }
        let bytes = raw.as_bytes();
        if bytes.len() < 3 || bytes[2] != b' ' {
            i += 1;
            continue;
        }
        let mut chars = raw.chars();
        let index_status = chars.next().unwrap_or(' ');
        let worktree_status = chars.next().unwrap_or(' ');
        let path = raw[3..].trim();
        if path.is_empty() {
            i += 1;
            continue;
        }
        let mut current_path = path.to_string();
        let mut orig_path = None;
        if (index_status == 'R' || index_status == 'C') && i + 1 < entries.len() {
            let next_raw = entries[i + 1].trim_end();
            if !looks_like_porcelain_status(next_raw) && !next_raw.is_empty() {
                orig_path = Some(current_path);
                current_path = next_raw.to_string();
                i += 1;
            }
        }
        out.push(GitStatusEntry {
            path: current_path,
            orig_path,
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        });
        i += 1;
    }
    out
}

fn looks_like_porcelain_status(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[2] == b' '
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

fn watcher(tx: mpsc::UnboundedSender<Event>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    Ok(watcher)
}
