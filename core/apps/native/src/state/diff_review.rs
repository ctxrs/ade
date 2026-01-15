use gpui::Context;

use ctx_core::ids::WorktreeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Add,
    Del,
    Context,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffHunk {
    pub(crate) key: String,
    pub(crate) header_line: String,
    pub(crate) lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffFile {
    pub(crate) key: String,
    pub(crate) file_path: String,
    pub(crate) section_lines: Vec<String>,
    pub(crate) header_lines: Vec<String>,
    pub(crate) hunks: Vec<DiffHunk>,
    pub(crate) is_new: bool,
    pub(crate) is_deleted: bool,
    pub(crate) is_binary: bool,
    pub(crate) added_lines: usize,
    pub(crate) deleted_lines: usize,
}

impl DiffFile {
    pub(crate) fn patch_text(&self) -> String {
        let mut patch = self.section_lines.join("\n");
        patch.push('\n');
        patch
    }

    pub(crate) fn hunk_patch_text(&self, hunk: &DiffHunk) -> String {
        let mut lines =
            Vec::with_capacity(self.header_lines.len() + hunk.lines.len() + 1);
        lines.extend(self.header_lines.iter().cloned());
        lines.push(hunk.header_line.clone());
        lines.extend(hunk.lines.iter().cloned());
        let mut patch = lines.join("\n");
        patch.push('\n');
        patch
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffPatchAction {
    Accept,
    Reject,
}

impl DiffPatchAction {
    fn as_str(self) -> &'static str {
        match self {
            DiffPatchAction::Accept => "accept",
            DiffPatchAction::Reject => "reject",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffListResizeState {
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

const DIFF_LIST_DEFAULT_WIDTH: f32 = 220.0;
const DIFF_LIST_MIN_WIDTH: f32 = 160.0;
const DIFF_LIST_MAX_WIDTH: f32 = 320.0;

#[derive(Debug)]
pub(crate) struct DiffReviewState {
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) diff: String,
    pub(crate) files: Vec<DiffFile>,
    pub(crate) active_file_key: Option<String>,
    pub(crate) busy_key: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) list_width: f32,
    pub(crate) list_resizing: bool,
    pub(crate) list_resize_state: Option<DiffListResizeState>,
}

impl DiffReviewState {
    pub(crate) fn new() -> Self {
        Self {
            worktree_id: None,
            diff: String::new(),
            files: Vec::new(),
            active_file_key: None,
            busy_key: None,
            status: None,
            error: None,
            list_width: DIFF_LIST_DEFAULT_WIDTH,
            list_resizing: false,
            list_resize_state: None,
        }
    }

    pub(crate) fn set_worktree_id(
        &mut self,
        worktree_id: Option<WorktreeId>,
        cx: &mut Context<Self>,
    ) {
        if self.worktree_id == worktree_id {
            return;
        }
        self.worktree_id = worktree_id;
        self.reset_state();
        if self.worktree_id.is_some() {
            self.reload_diff(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn select_file(&mut self, key: String, cx: &mut Context<Self>) {
        if self.active_file_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.active_file_key = Some(key);
        cx.notify();
    }

    pub(crate) fn reload_diff(&mut self, cx: &mut Context<Self>) {
        if self.worktree_id.is_none() {
            return;
        }
        self.busy_key = None;
        self.status = Some("Diff unavailable in trackless mode.".to_string());
        self.error = None;
        self.diff.clear();
        self.files.clear();
        self.active_file_key = None;
        cx.notify();
    }

    pub(crate) fn apply_file_patch(
        &mut self,
        file_key: String,
        action: DiffPatchAction,
        patch: String,
        status_message: String,
        cx: &mut Context<Self>,
    ) {
        self.apply_patch(file_key, action, patch, status_message, cx);
    }

    pub(crate) fn apply_hunk_patch(
        &mut self,
        hunk_key: String,
        action: DiffPatchAction,
        patch: String,
        status_message: String,
        cx: &mut Context<Self>,
    ) {
        self.apply_patch(hunk_key, action, patch, status_message, cx);
    }

    pub(crate) fn apply_all_patch(
        &mut self,
        action: DiffPatchAction,
        status_message: String,
        cx: &mut Context<Self>,
    ) {
        if self.diff.trim().is_empty() {
            return;
        }
        let patch = self.diff.clone();
        self.apply_patch(
            format!("diff:apply:{}", action.as_str()),
            action,
            patch,
            status_message,
            cx,
        );
    }

    pub(crate) fn set_list_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let clamped = width.round().clamp(DIFF_LIST_MIN_WIDTH, DIFF_LIST_MAX_WIDTH);
        if (self.list_width - clamped).abs() < f32::EPSILON {
            return;
        }
        self.list_width = clamped;
        cx.notify();
    }

    pub(crate) fn reset_list_width(&mut self, cx: &mut Context<Self>) {
        self.set_list_width(DIFF_LIST_DEFAULT_WIDTH, cx);
    }

    fn reset_state(&mut self) {
        self.diff.clear();
        self.files.clear();
        self.active_file_key = None;
        self.busy_key = None;
        self.status = None;
        self.error = None;
        self.list_resizing = false;
        self.list_resize_state = None;
    }

    fn apply_patch(
        &mut self,
        busy_key: String,
        action: DiffPatchAction,
        patch: String,
        status_message: String,
        cx: &mut Context<Self>,
    ) {
        let _ = (busy_key, action, patch, status_message);
        self.busy_key = None;
        self.status = Some("Diff unavailable in trackless mode.".to_string());
        self.error = None;
        cx.notify();
    }
}
