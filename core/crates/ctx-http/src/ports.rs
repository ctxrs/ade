use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;

use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};

const OUTPUT_TAIL_LIMIT: usize = 256;

#[derive(Debug, Clone)]
pub struct PortObservationContext {
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub terminal_id: TerminalId,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortEntry {
    pub id: String,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<TerminalId>,
    pub host: String,
    pub port: u16,
    pub scheme: String,
    pub source: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct DetectedPort {
    host: String,
    port: u16,
    scheme: String,
}

#[derive(Debug)]
pub struct PortRegistry {
    entries: Mutex<HashMap<String, PortEntry>>,
    tails: Mutex<HashMap<TerminalId, String>>,
    auto_forward: AtomicBool,
}

impl PortRegistry {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            tails: Mutex::new(HashMap::new()),
            auto_forward: AtomicBool::new(true),
        }
    }

    pub fn set_auto_forward(&self, enabled: bool) {
        self.auto_forward.store(enabled, Ordering::Relaxed);
    }

    pub fn list(&self, workspace_id: WorkspaceId) -> Vec<PortEntry> {
        let entries = self.entries.lock().expect("port registry lock");
        let mut list: Vec<PortEntry> = entries
            .values()
            .filter(|entry| entry.workspace_id == workspace_id)
            .cloned()
            .collect();
        list.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
        list
    }

    pub fn get(&self, id: &str) -> Option<PortEntry> {
        let entries = self.entries.lock().expect("port registry lock");
        entries.get(id).cloned()
    }

    pub fn observe_output(&self, ctx: &PortObservationContext, bytes: &[u8]) {
        if !self.auto_forward.load(Ordering::Relaxed) {
            return;
        }

        let chunk = String::from_utf8_lossy(bytes);
        if chunk.trim().is_empty() {
            return;
        }

        let combined = {
            let mut tails = self.tails.lock().expect("port tail lock");
            let prev = tails.get(&ctx.terminal_id).cloned().unwrap_or_default();
            let mut next = format!("{prev}{chunk}");
            if next.len() > OUTPUT_TAIL_LIMIT {
                let start = next.len() - OUTPUT_TAIL_LIMIT;
                next = next[start..].to_string();
            }
            tails.insert(ctx.terminal_id, next.clone());
            next
        };

        let detected = detect_ports(&combined);
        if detected.is_empty() {
            return;
        }

        let now = Utc::now();
        let mut entries = self.entries.lock().expect("port registry lock");
        for port in detected {
            let existing_id = entries.iter().find_map(|(id, entry)| {
                if entry.workspace_id == ctx.workspace_id
                    && entry.port == port.port
                    && entry.host == port.host
                {
                    Some(id.clone())
                } else {
                    None
                }
            });
            if let Some(id) = existing_id {
                if let Some(entry) = entries.get_mut(&id) {
                    entry.last_seen_at = now;
                    entry.task_id = ctx.task_id;
                    entry.session_id = ctx.session_id;
                    entry.worktree_id = ctx.worktree_id;
                    entry.terminal_id = Some(ctx.terminal_id);
                    entry.scheme = port.scheme;
                }
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            entries.insert(
                id.clone(),
                PortEntry {
                    id,
                    workspace_id: ctx.workspace_id,
                    task_id: ctx.task_id,
                    session_id: ctx.session_id,
                    worktree_id: ctx.worktree_id,
                    terminal_id: Some(ctx.terminal_id),
                    host: port.host,
                    port: port.port,
                    scheme: port.scheme,
                    source: "terminal_output".to_string(),
                    first_seen_at: now,
                    last_seen_at: now,
                },
            );
        }
    }
}

impl Default for PortRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_ports(text: &str) -> Vec<DetectedPort> {
    let mut matches = Vec::new();
    let mut seen = HashSet::new();
    for caps in host_port_re().captures_iter(text) {
        let Some(port) = caps
            .name("port")
            .and_then(|m| m.as_str().parse::<u16>().ok())
        else {
            continue;
        };
        let host = caps
            .name("host")
            .map(|m| normalize_host(m.as_str()))
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let scheme = caps
            .name("scheme")
            .map(|m| m.as_str().trim_end_matches("://").to_lowercase())
            .unwrap_or_else(|| "http".to_string());
        let key = (host.clone(), port, scheme.clone());
        if seen.insert(key.clone()) {
            matches.push(DetectedPort { host, port, scheme });
        }
    }

    for caps in port_only_re().captures_iter(text) {
        let Some(port) = caps
            .name("port")
            .and_then(|m| m.as_str().parse::<u16>().ok())
        else {
            continue;
        };
        let key = ("127.0.0.1".to_string(), port, "http".to_string());
        if seen.insert(key.clone()) {
            matches.push(DetectedPort {
                host: key.0,
                port: key.1,
                scheme: key.2,
            });
        }
    }

    matches
}

fn host_port_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<scheme>https?://)?(?P<host>localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])[:](?P<port>\d{2,5})",
        )
        .expect("port regex")
    })
}

fn port_only_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bport\s+(?P<port>\d{2,5})\b").expect("port-only regex"))
}

fn normalize_host(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('[').trim_end_matches(']');
    match trimmed {
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" => "127.0.0.1".to_string(),
        _ => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::ids::{SessionId, TaskId, WorktreeId};

    #[test]
    fn detects_ports_from_terminal_output() {
        let registry = PortRegistry::new();
        let ctx = PortObservationContext {
            workspace_id: WorkspaceId::new(),
            task_id: Some(TaskId::new()),
            session_id: Some(SessionId::new()),
            worktree_id: Some(WorktreeId::new()),
            terminal_id: TerminalId::new(),
        };

        registry.observe_output(&ctx, b"Local: http://localhost:5173/\n");
        registry.observe_output(&ctx, b"Network: http://0.0.0.0:5173/\n");

        let entries = registry.list(ctx.workspace_id);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].port, 5173);
        assert_eq!(entries[0].host, "127.0.0.1");
        assert_eq!(entries[0].scheme, "http");
    }

    #[test]
    fn detects_ports_from_port_only_lines() {
        let registry = PortRegistry::new();
        let ctx = PortObservationContext {
            workspace_id: WorkspaceId::new(),
            task_id: None,
            session_id: None,
            worktree_id: None,
            terminal_id: TerminalId::new(),
        };

        registry.observe_output(&ctx, b"Server listening on port 3000\n");

        let entries = registry.list(ctx.workspace_id);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].port, 3000);
        assert_eq!(entries[0].host, "127.0.0.1");
        assert_eq!(entries[0].scheme, "http");
    }

    #[test]
    fn auto_forward_toggle_disables_detection() {
        let registry = PortRegistry::new();
        registry.set_auto_forward(false);
        let ctx = PortObservationContext {
            workspace_id: WorkspaceId::new(),
            task_id: None,
            session_id: None,
            worktree_id: None,
            terminal_id: TerminalId::new(),
        };
        registry.observe_output(&ctx, b"listening on port 3000\n");
        let entries = registry.list(ctx.workspace_id);
        assert!(entries.is_empty());
    }
}
