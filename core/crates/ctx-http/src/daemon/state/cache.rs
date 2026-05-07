use std::collections::HashSet;
use std::time::{Duration, Instant};

use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

use super::AppState;

const DEFAULT_SESSION_CACHE_TTL_HOURS: u64 = 24;
const DEFAULT_WORKSPACE_CACHE_TTL_DAYS: u64 = 1;
const DEFAULT_CACHE_SWEEP_INTERVAL_SECS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug)]
pub struct CacheSweepConfig {
    pub session_ttl: Duration,
    pub workspace_ttl: Duration,
    pub interval: Duration,
}

impl CacheSweepConfig {
    pub fn from_env() -> Self {
        let session_ttl_hours = std::env::var("CTX_SESSION_CACHE_TTL_HOURS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_SESSION_CACHE_TTL_HOURS);
        let workspace_ttl_days = std::env::var("CTX_WORKSPACE_CACHE_TTL_DAYS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_WORKSPACE_CACHE_TTL_DAYS);
        let interval_secs = std::env::var("CTX_CACHE_SWEEP_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_SWEEP_INTERVAL_SECS);
        Self {
            session_ttl: Duration::from_secs(session_ttl_hours * 60 * 60),
            workspace_ttl: Duration::from_secs(workspace_ttl_days * 24 * 60 * 60),
            interval: Duration::from_secs(interval_secs.max(30)),
        }
    }
}

#[derive(Default, Debug)]
pub struct CacheSweepStats {
    pub session_head_evicted: usize,
    pub session_meta_evicted: usize,
    pub schedulers_evicted: usize,
    pub broadcasters_evicted: usize,
    pub session_event_heads_evicted: usize,
    pub file_completions_evicted: usize,
    pub workspace_file_completions_evicted: usize,
    pub git_status_evicted: usize,
    pub worktree_vcs_evicted: usize,
    pub workspace_snapshot_evicted: usize,
    pub workspace_heads_evicted: usize,
    pub worktree_bootstrap_evicted: usize,
    pub workspace_stores_evicted: usize,
}

impl CacheSweepStats {
    pub fn total_evicted(&self) -> usize {
        self.session_head_evicted
            + self.session_meta_evicted
            + self.schedulers_evicted
            + self.broadcasters_evicted
            + self.session_event_heads_evicted
            + self.file_completions_evicted
            + self.workspace_file_completions_evicted
            + self.git_status_evicted
            + self.worktree_vcs_evicted
            + self.workspace_snapshot_evicted
            + self.workspace_heads_evicted
            + self.worktree_bootstrap_evicted
            + self.workspace_stores_evicted
    }
}

#[derive(Debug)]
pub struct TimedEntry<T> {
    pub(crate) value: T,
    pub(crate) last_access: Instant,
}

impl<T> TimedEntry<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value,
            last_access: Instant::now(),
        }
    }

    pub(crate) fn touch(&mut self) {
        self.last_access = Instant::now();
    }

    pub(crate) fn touch_at(&mut self, now: Instant) {
        self.last_access = now;
    }
}

impl AppState {
    pub async fn sweep_idle_caches(
        &self,
        now: Instant,
        config: CacheSweepConfig,
    ) -> CacheSweepStats {
        let mut stats = CacheSweepStats::default();
        let running_sessions = {
            let set = self.sessions.running_sessions.lock().await;
            set.iter().copied().collect::<HashSet<_>>()
        };
        {
            let mut cache = self.sessions.session_head_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_head_evicted += expired.len();
        }
        {
            let mut cache = self.sessions.session_meta_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_meta_evicted += expired.len();
        }
        {
            let mut map = self.sessions.schedulers.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if entry.value.is_closed()
                        || now.duration_since(entry.last_access) >= config.session_ttl
                    {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.schedulers_evicted += expired.len();
        }
        {
            let mut map = self.sessions.broadcasters.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.broadcasters_evicted += expired.len();
        }
        {
            let mut map = self.sessions.session_event_heads.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.session_event_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.file_completions_cache.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.file_completions_evicted += expired.len();
        }
        {
            let mut cache = self
                .workspaces
                .workspace_file_completions_cache
                .lock()
                .await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_file_completions_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.git_status_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.git_status_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_vcs_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_vcs_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_snapshot_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_snapshot_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_heads_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_bootstrap_gates.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_bootstrap_evicted += expired.len();
        }
        let active_workspaces = self.protected_workspace_store_ids().await;

        stats.workspace_stores_evicted = self
            .core
            .stores
            .evict_idle_workspaces(config.workspace_ttl, &active_workspaces)
            .await;

        self.emit_cache_evicted("session_head", stats.session_head_evicted)
            .await;
        self.emit_cache_evicted("session_meta", stats.session_meta_evicted)
            .await;
        self.emit_cache_evicted("scheduler", stats.schedulers_evicted)
            .await;
        self.emit_cache_evicted("broadcaster", stats.broadcasters_evicted)
            .await;
        self.emit_cache_evicted("session_event_head", stats.session_event_heads_evicted)
            .await;
        self.emit_cache_evicted("file_completions", stats.file_completions_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_file_completions",
            stats.workspace_file_completions_evicted,
        )
        .await;
        self.emit_cache_evicted("git_status", stats.git_status_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_active_snapshot",
            stats.workspace_snapshot_evicted,
        )
        .await;
        self.emit_cache_evicted("workspace_active_heads", stats.workspace_heads_evicted)
            .await;
        self.emit_cache_evicted("worktree_bootstrap", stats.worktree_bootstrap_evicted)
            .await;
        self.emit_cache_evicted("workspace_store", stats.workspace_stores_evicted)
            .await;

        stats
    }
}
