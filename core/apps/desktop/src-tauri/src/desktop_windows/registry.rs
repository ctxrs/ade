use std::collections::{HashMap, HashSet};

use ctx_desktop_ipc::{
    DesktopDockRecentLocalWorkspace as DockRecentLocalWorkspaceEntry,
};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Default)]
pub(crate) struct WorkspaceWindowRegistry {
    by_window: std::sync::Mutex<HashMap<String, HashSet<String>>>,
    recent_workspaces: std::sync::Mutex<Vec<RecentWorkspaceEntry>>,
    dock_recent_local_workspaces: std::sync::Mutex<Vec<DockRecentLocalWorkspaceEntry>>,
}

const MAX_RECENT_WORKSPACES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecentWorkspaceEntry {
    workspace_id: String,
    label: String,
}

impl WorkspaceWindowRegistry {
    pub(super) fn register(&self, window_label: &str, workspace_id: &str) {
        let mut map = match self.by_window.lock() {
            Ok(map) => map,
            Err(_) => return,
        };
        map.entry(window_label.to_string())
            .or_default()
            .insert(workspace_id.to_string());
    }

    pub(super) fn unregister_window(&self, window_label: &str) {
        let mut map = match self.by_window.lock() {
            Ok(map) => map,
            Err(_) => return,
        };
        map.remove(window_label);
    }

    pub(super) fn set_window_workspaces(&self, window_label: &str, workspace_ids: Vec<String>) {
        let mut map = match self.by_window.lock() {
            Ok(map) => map,
            Err(_) => return,
        };
        let mut set = HashSet::new();
        for id in workspace_ids {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                continue;
            }
            set.insert(trimmed.to_string());
        }
        if set.is_empty() {
            map.remove(window_label);
        } else {
            map.insert(window_label.to_string(), set);
        }
    }

    pub(super) fn window_for_workspace(&self, workspace_id: &str) -> Option<String> {
        let map = self.by_window.lock().ok()?;
        map.iter().find_map(|(label, ids)| {
            if ids.contains(workspace_id) {
                Some(label.clone())
            } else {
                None
            }
        })
    }

    pub(super) fn workspace_ids(&self) -> Vec<String> {
        let map = match self.by_window.lock() {
            Ok(map) => map,
            Err(_) => return Vec::new(),
        };
        let mut out = HashSet::new();
        for ids in map.values() {
            for id in ids {
                out.insert(id.clone());
            }
        }
        out.into_iter().collect()
    }

    pub(super) fn record_recent_workspace(&self, workspace_id: &str, workspace_label: Option<&str>) {
        let workspace_id = workspace_id.trim();
        if workspace_id.is_empty() {
            return;
        }

        let label = workspace_label
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(workspace_id)
            .to_string();

        let mut recent = match self.recent_workspaces.lock() {
            Ok(recent) => recent,
            Err(_) => return,
        };
        recent.retain(|entry| entry.workspace_id != workspace_id);
        recent.insert(
            0,
            RecentWorkspaceEntry {
                workspace_id: workspace_id.to_string(),
                label,
            },
        );
        if recent.len() > MAX_RECENT_WORKSPACES {
            recent.truncate(MAX_RECENT_WORKSPACES);
        }
    }

    fn recent_workspaces(&self) -> Vec<RecentWorkspaceEntry> {
        match self.recent_workspaces.lock() {
            Ok(recent) => recent.clone(),
            Err(_) => Vec::new(),
        }
    }

    pub(super) fn set_dock_recent_local_workspaces(&self, entries: Vec<DockRecentLocalWorkspaceEntry>) {
        let mut dedup = HashSet::new();
        let mut normalized = Vec::new();
        for entry in entries {
            let root_path = entry.root_path.trim();
            if root_path.is_empty() {
                continue;
            }
            if !dedup.insert(root_path.to_string()) {
                continue;
            }
            let label = entry.label.trim();
            normalized.push(DockRecentLocalWorkspaceEntry {
                label: if label.is_empty() {
                    root_path.to_string()
                } else {
                    label.to_string()
                },
                root_path: root_path.to_string(),
            });
            if normalized.len() >= MAX_RECENT_WORKSPACES {
                break;
            }
        }

        let mut guard = match self.dock_recent_local_workspaces.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        *guard = normalized;
    }

    pub(super) fn dock_recent_local_workspaces(&self) -> Vec<DockRecentLocalWorkspaceEntry> {
        match self.dock_recent_local_workspaces.lock() {
            Ok(entries) => entries.clone(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_workspaces_are_deduplicated_and_ordered() {
        let registry = WorkspaceWindowRegistry::default();

        registry.record_recent_workspace("ws-a", Some("Workspace A"));
        registry.record_recent_workspace("ws-b", Some("Workspace B"));
        registry.record_recent_workspace("ws-a", Some("Workspace A Renamed"));

        let recent = registry.recent_workspaces();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].workspace_id, "ws-a");
        assert_eq!(recent[0].label, "Workspace A Renamed");
        assert_eq!(recent[1].workspace_id, "ws-b");
    }

    #[test]
    fn recent_workspaces_are_trimmed_to_max_size() {
        let registry = WorkspaceWindowRegistry::default();
        for idx in 0..(MAX_RECENT_WORKSPACES + 4) {
            registry.record_recent_workspace(&format!("ws-{idx}"), Some(&format!("Workspace {idx}")));
        }

        let recent = registry.recent_workspaces();
        assert_eq!(recent.len(), MAX_RECENT_WORKSPACES);
        assert_eq!(
            recent[0].workspace_id,
            format!("ws-{}", MAX_RECENT_WORKSPACES + 3)
        );
    }

    #[test]
    fn recent_workspace_uses_workspace_id_when_label_missing() {
        let registry = WorkspaceWindowRegistry::default();
        registry.record_recent_workspace("ws-abc", Some("   "));
        let recent = registry.recent_workspaces();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].label, "ws-abc");
    }

    #[test]
    fn set_window_workspaces_replaces_stale_workspace_mappings() {
        let registry = WorkspaceWindowRegistry::default();
        registry.register("window-a", "ws-a");
        registry.register("window-a", "ws-b");
        registry.set_window_workspaces("window-a", vec!["ws-c".to_string()]);

        assert_eq!(registry.window_for_workspace("ws-a"), None);
        assert_eq!(registry.window_for_workspace("ws-b"), None);
        assert_eq!(registry.window_for_workspace("ws-c").as_deref(), Some("window-a"));
    }

    #[test]
    fn dock_recent_local_workspaces_are_deduped_and_trimmed() {
        let registry = WorkspaceWindowRegistry::default();
        registry.set_dock_recent_local_workspaces(vec![
            DockRecentLocalWorkspaceEntry {
                label: "Alpha".to_string(),
                root_path: "/tmp/alpha".to_string(),
            },
            DockRecentLocalWorkspaceEntry {
                label: "Alpha Duplicate".to_string(),
                root_path: "/tmp/alpha".to_string(),
            },
            DockRecentLocalWorkspaceEntry {
                label: "  ".to_string(),
                root_path: "/tmp/beta".to_string(),
            },
        ]);

        let entries = registry.dock_recent_local_workspaces();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Alpha");
        assert_eq!(entries[0].root_path, "/tmp/alpha");
        assert_eq!(entries[1].label, "/tmp/beta");
        assert_eq!(entries[1].root_path, "/tmp/beta");
    }
}
