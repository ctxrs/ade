use std::collections::HashSet;
use std::time::Instant;

use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

use super::{AppState, CacheSweepConfig, CacheSweepStats};

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
