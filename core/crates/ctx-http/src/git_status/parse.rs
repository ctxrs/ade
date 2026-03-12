use super::{GitStatusEntry, WORKTREE_VCS_TOUCHED_FILES_CAP};

#[derive(Debug)]
pub(super) struct GitStatusBranchInfo {
    pub(super) summary_line: String,
    pub(super) branch: Option<String>,
    pub(super) upstream: Option<String>,
    pub(super) ahead: i64,
    pub(super) behind: i64,
    pub(super) detached: bool,
}

pub(super) struct ParsedGitStatusEntries {
    pub(super) entries: Vec<GitStatusEntry>,
    pub(super) staged: i64,
    pub(super) unstaged: i64,
    pub(super) untracked: i64,
    pub(super) total_count: i64,
    pub(super) truncated: bool,
}

pub(super) fn parse_git_status_short(output: &str) -> GitStatusBranchInfo {
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

pub(super) fn parse_git_status_entries(entries: &[String]) -> ParsedGitStatusEntries {
    let mut out = Vec::new();
    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;
    let mut total_count = 0;
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
        if out.len() < WORKTREE_VCS_TOUCHED_FILES_CAP {
            out.push(GitStatusEntry {
                path: current_path,
                orig_path,
                index_status: index_status.to_string(),
                worktree_status: worktree_status.to_string(),
            });
        }
        i += 1;
    }
    ParsedGitStatusEntries {
        entries: out,
        staged,
        unstaged,
        untracked,
        total_count,
        truncated: total_count as usize > WORKTREE_VCS_TOUCHED_FILES_CAP,
    }
}

fn looks_like_porcelain_status(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[2] == b' '
}
