use gpui::{AsyncApp, Context, WeakEntity};
use gpui_tokio::Tokio;

use ctx_core::ids::SessionId;

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
    pub(crate) session_id: Option<SessionId>,
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
            session_id: None,
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

    pub(crate) fn set_session_id(
        &mut self,
        session_id: Option<SessionId>,
        cx: &mut Context<Self>,
    ) {
        if self.session_id == session_id {
            return;
        }
        self.session_id = session_id;
        self.reset_state();
        if self.session_id.is_some() {
            self.reload_diff(cx);
        } else {
            self.status = Some("Select a session to view diff.".to_string());
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
        let Some(session_id) = self.session_id else {
            self.reset_state();
            self.status = Some("Select a session to view diff.".to_string());
            cx.notify();
            return;
        };
        self.busy_key = Some("diff:load".to_string());
        self.status = None;
        self.error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let diff = client.get_session_diff(session_id).await?;
            Ok((session_id, diff.diff))
        });

        cx.spawn(move |this: WeakEntity<DiffReviewState>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| match result {
                    Ok((session_id, diff)) => {
                        if view.session_id != Some(session_id) {
                            return;
                        }
                        view.busy_key = None;
                        view.status = None;
                        view.error = None;
                        view.set_diff(diff);
                        cx.notify();
                    }
                    Err(err) => {
                        view.busy_key = None;
                        view.status = None;
                        view.error = Some(err.to_string());
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
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

    fn set_diff(&mut self, diff: String) {
        self.diff = diff;
        self.files = parse_unified_diff(&self.diff);
        if let Some(active) = self.active_file_key.as_ref() {
            if self.files.iter().any(|file| &file.key == active) {
                return;
            }
        }
        self.active_file_key = self.files.first().map(|file| file.key.clone());
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
        let Some(session_id) = self.session_id else {
            return;
        };
        if patch.trim().is_empty() {
            return;
        }
        self.busy_key = Some(busy_key.clone());
        self.status = None;
        self.error = None;
        cx.notify();

        let action = action.as_str().to_string();
        let patch_value = patch.clone();
        let status_value = status_message.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let diff = client
                .apply_session_diff_patch(session_id, &action, &patch_value)
                .await?;
            Ok((session_id, diff.diff, status_value))
        });

        cx.spawn(move |this: WeakEntity<DiffReviewState>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| match result {
                    Ok((session_id, diff, status_message)) => {
                        if view.session_id != Some(session_id) {
                            return;
                        }
                        view.busy_key = None;
                        view.status = Some(status_message);
                        view.error = None;
                        view.set_diff(diff);
                        cx.notify();
                    }
                    Err(err) => {
                        view.busy_key = None;
                        view.status = None;
                        view.error = Some(err.to_string());
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }
}

#[derive(Clone, Debug)]
struct DiffFileSeed {
    key: String,
    old_path: String,
    new_path: String,
    section_lines: Vec<String>,
    header_lines: Vec<String>,
    hunks: Vec<DiffHunk>,
}

fn parse_unified_diff(diff_text: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current: Option<DiffFileSeed> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut in_header = false;

    let mut lines = diff_text.lines().map(str::to_string).collect::<Vec<_>>();
    if lines.last().map(|line| line.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    let push_current = |files: &mut Vec<DiffFile>,
                        current: &mut Option<DiffFileSeed>,
                        current_hunk: &mut Option<DiffHunk>| {
        if let Some(mut seed) = current.take() {
            if let Some(hunk) = current_hunk.take() {
                seed.hunks.push(hunk);
            }
            let file = finalize_file(seed);
            if file.section_lines.iter().any(|line| !line.trim().is_empty()) {
                files.push(file);
            }
        }
    };

    for (idx, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            push_current(&mut files, &mut current, &mut current_hunk);
            let mut parts = rest.split_whitespace();
            let old_path = parts
                .next()
                .unwrap_or_default()
                .trim_start_matches("a/")
                .to_string();
            let new_path = parts
                .next()
                .unwrap_or_default()
                .trim_start_matches("b/")
                .to_string();
            let key = format!("{old_path}=>{new_path}:{idx}");
            current = Some(DiffFileSeed {
                key,
                old_path,
                new_path,
                section_lines: vec![line.clone()],
                header_lines: vec![line.clone()],
                hunks: Vec::new(),
            });
            in_header = true;
            continue;
        }

        let Some(seed) = current.as_mut() else {
            continue;
        };
        seed.section_lines.push(line.clone());

        if line.starts_with("@@") {
            if let Some(hunk) = current_hunk.take() {
                seed.hunks.push(hunk);
            }
            current_hunk = Some(DiffHunk {
                key: format!("{}:h{}:{}", seed.key, seed.hunks.len(), idx),
                header_line: line.clone(),
                lines: Vec::new(),
            });
            in_header = false;
            continue;
        }

        if in_header {
            seed.header_lines.push(line.clone());
        } else if let Some(hunk) = current_hunk.as_mut() {
            hunk.lines.push(line.clone());
        }
    }

    push_current(&mut files, &mut current, &mut current_hunk);

    files
}

fn finalize_file(seed: DiffFileSeed) -> DiffFile {
    let file_path = if !seed.new_path.is_empty() && seed.new_path != "dev/null" {
        seed.new_path.clone()
    } else if !seed.old_path.is_empty() && seed.old_path != "dev/null" {
        seed.old_path.clone()
    } else {
        "(unknown)".to_string()
    };

    let header_text = seed.header_lines.join("\n");
    let is_new = seed.old_path == "dev/null"
        || header_text.contains("new file mode")
        || header_text.contains("--- /dev/null");
    let is_deleted = seed.new_path == "dev/null"
        || header_text.contains("deleted file mode")
        || header_text.contains("+++ /dev/null");
    let patch_text = seed.section_lines.join("\n");
    let mut is_binary = patch_text.contains("GIT binary patch") || patch_text.contains("Binary files");

    let mut added_lines = 0;
    let mut deleted_lines = 0;
    for hunk in &seed.hunks {
        for line in &hunk.lines {
            if line.starts_with('+') {
                if !line.starts_with("+++") {
                    added_lines += 1;
                }
            } else if line.starts_with('-') {
                if !line.starts_with("---") {
                    deleted_lines += 1;
                }
            }
        }
    }

    if seed.hunks.is_empty() {
        is_binary = true;
    }

    DiffFile {
        key: seed.key,
        file_path,
        section_lines: seed.section_lines,
        header_lines: seed.header_lines,
        hunks: seed.hunks,
        is_new,
        is_deleted,
        is_binary,
        added_lines,
        deleted_lines,
    }
}
