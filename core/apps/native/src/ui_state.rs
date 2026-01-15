use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use ctx_core::ids::WorkspaceId;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RecentWorkspaceEntry {
    pub(crate) name: String,
    pub(crate) root_path: String,
    pub(crate) updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct NativeUiState {
    #[serde(default)]
    pub(crate) sidebar_width_by_workspace: HashMap<String, f32>,
    #[serde(default)]
    pub(crate) sidebar_collapsed_by_workspace: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) archived_collapsed_by_workspace: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) sessions_pane_open_by_scope: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) diff_pane_open_by_scope: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) artifacts_pane_open_by_scope: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) terminal_panel_open_by_workspace: HashMap<String, bool>,
    #[serde(default)]
    pub(crate) archive_confirm_dismissed: bool,
    #[serde(default)]
    pub(crate) recent_workspaces: Vec<RecentWorkspaceEntry>,
}

pub(crate) struct UiStateStore {
    path: PathBuf,
    state: NativeUiState,
}

impl UiStateStore {
    pub(crate) fn load() -> Self {
        let path = ui_state_path();
        let state = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<NativeUiState>(&bytes).ok())
            .unwrap_or_default();
        Self { path, state }
    }

    pub(crate) fn sidebar_width(&self, workspace_id: WorkspaceId) -> Option<f32> {
        self.state
            .sidebar_width_by_workspace
            .get(&workspace_key(workspace_id))
            .copied()
    }

    pub(crate) fn set_sidebar_width(&mut self, workspace_id: WorkspaceId, width: f32) {
        self.state
            .sidebar_width_by_workspace
            .insert(workspace_key(workspace_id), width);
        self.save();
    }

    pub(crate) fn sidebar_collapsed(&self, workspace_id: WorkspaceId) -> Option<bool> {
        self.state
            .sidebar_collapsed_by_workspace
            .get(&workspace_key(workspace_id))
            .copied()
    }

    pub(crate) fn set_sidebar_collapsed(&mut self, workspace_id: WorkspaceId, collapsed: bool) {
        self.state
            .sidebar_collapsed_by_workspace
            .insert(workspace_key(workspace_id), collapsed);
        self.save();
    }

    pub(crate) fn archived_collapsed(&self, workspace_id: WorkspaceId) -> Option<bool> {
        self.state
            .archived_collapsed_by_workspace
            .get(&workspace_key(workspace_id))
            .copied()
    }

    pub(crate) fn set_archived_collapsed(&mut self, workspace_id: WorkspaceId, collapsed: bool) {
        self.state
            .archived_collapsed_by_workspace
            .insert(workspace_key(workspace_id), collapsed);
        self.save();
    }

    pub(crate) fn archive_confirm_dismissed(&self) -> bool {
        self.state.archive_confirm_dismissed
    }

    pub(crate) fn sessions_pane_open(
        &self,
        workspace_id: WorkspaceId,
        scope: &str,
    ) -> Option<bool> {
        self.state
            .sessions_pane_open_by_scope
            .get(&workspace_scope_key(workspace_id, scope))
            .copied()
    }

    pub(crate) fn diff_pane_open(
        &self,
        workspace_id: WorkspaceId,
        scope: &str,
    ) -> Option<bool> {
        self.state
            .diff_pane_open_by_scope
            .get(&workspace_scope_key(workspace_id, scope))
            .copied()
    }

    pub(crate) fn artifacts_pane_open(
        &self,
        workspace_id: WorkspaceId,
        scope: &str,
    ) -> Option<bool> {
        self.state
            .artifacts_pane_open_by_scope
            .get(&workspace_scope_key(workspace_id, scope))
            .copied()
    }

    pub(crate) fn terminal_panel_open(&self, workspace_id: WorkspaceId) -> Option<bool> {
        self.state
            .terminal_panel_open_by_workspace
            .get(&workspace_key(workspace_id))
            .copied()
    }

    #[allow(dead_code)]
    pub(crate) fn set_archive_confirm_dismissed(&mut self, dismissed: bool) {
        self.state.archive_confirm_dismissed = dismissed;
        self.save();
    }

    pub(crate) fn recent_workspaces(&self) -> Vec<RecentWorkspaceEntry> {
        self.state.recent_workspaces.clone()
    }

    pub(crate) fn record_recent_workspace(&mut self, name: &str, root_path: &str) {
        if name.trim().is_empty() || root_path.trim().is_empty() {
            return;
        }
        let mut entries = self.state.recent_workspaces.clone();
        if let Some(pos) = entries
            .iter()
            .position(|entry| entry.root_path == root_path)
        {
            entries.remove(pos);
        }
        entries.insert(
            0,
            RecentWorkspaceEntry {
                name: name.to_string(),
                root_path: root_path.to_string(),
                updated_at_ms: now_ms(),
            },
        );
        entries.truncate(50);
        self.state.recent_workspaces = entries;
        self.save();
    }

    pub(crate) fn clear_recent_workspaces(&mut self) {
        if self.state.recent_workspaces.is_empty() {
            return;
        }
        self.state.recent_workspaces.clear();
        self.save();
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("ctx-native: failed to create ui state dir: {err}");
                return;
            }
        }
        match serde_json::to_vec_pretty(&self.state) {
            Ok(payload) => {
                if let Err(err) = fs::write(&self.path, payload) {
                    eprintln!("ctx-native: failed to write ui state: {err}");
                }
            }
            Err(err) => {
                eprintln!("ctx-native: failed to serialize ui state: {err}");
            }
        }
    }
}

fn workspace_key(workspace_id: WorkspaceId) -> String {
    workspace_id.0.to_string()
}

fn workspace_scope_key(workspace_id: WorkspaceId, scope: &str) -> String {
    format!("{}::{scope}", workspace_id.0)
}

fn ui_state_path() -> PathBuf {
    let base = env::var("CTX_DATA_DIR")
        .ok()
        .or_else(|| env::var("HOME").ok())
        .or_else(|| env::var("USERPROFILE").ok())
        .or_else(|| {
            let drive = env::var("HOMEDRIVE").ok()?;
            let path = env::var("HOMEPATH").ok()?;
            Some(format!("{}{}", drive, path))
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    base.join(".ctx").join("native_ui_state.json")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
