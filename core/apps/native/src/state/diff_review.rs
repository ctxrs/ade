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

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffListResizeState {
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

const DIFF_LIST_DEFAULT_WIDTH: f32 = 220.0;
const DIFF_LIST_MIN_WIDTH: f32 = 160.0;
const DIFF_LIST_MAX_WIDTH: f32 = 320.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffSummary {
    pub(crate) file_count: i64,
    pub(crate) additions: i64,
    pub(crate) deletions: i64,
}

#[derive(Debug)]
pub(crate) struct DiffReviewState {
    pub(crate) session_id: Option<SessionId>,
    pub(crate) diff: String,
    pub(crate) files: Vec<DiffFile>,
    pub(crate) active_file_key: Option<String>,
    pub(crate) busy_key: Option<String>,
    pub(crate) git_status: Option<Vec<String>>,
    pub(crate) diff_summary: Option<DiffSummary>,
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
            git_status: None,
            diff_summary: None,
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
            cx.notify();
        }
    }

    pub(crate) fn set_diff(&mut self, diff: String) {
        self.diff = diff;
        self.files = parse_unified_diff(&self.diff);
        self.ensure_active_file();
    }

    pub(crate) fn select_file(&mut self, key: String, cx: &mut Context<Self>) {
        if self.active_file_key.as_deref() == Some(key.as_str()) {
            self.active_file_key = None;
        } else {
            self.active_file_key = Some(key);
        }
        cx.notify();
    }

    pub(crate) fn reload_diff(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id else {
            return;
        };
        self.busy_key = Some("diff:load".to_string());
        self.git_status = None;
        self.diff_summary = None;
        self.error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.get_session_diff(session_id).await
        });

        let status_task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.get_session_diff_status(session_id).await
        });

        let summary_task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.get_session_diff_summary(session_id).await
        });

        cx.spawn(move |this: WeakEntity<DiffReviewState>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.session_id != Some(session_id) {
                        return;
                    }
                    view.busy_key = None;
                    match result {
                        Ok(diff) => {
                            view.set_diff(diff.diff);
                        }
                        Err(err) => {
                            view.error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();

        cx.spawn(move |this: WeakEntity<DiffReviewState>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = status_task.await;
                this.update(&mut cx, |view, cx| {
                    if view.session_id != Some(session_id) {
                        return;
                    }
                    match result {
                        Ok(status) => {
                            view.git_status = Some(status.lines);
                        }
                        Err(_) => {
                            view.git_status = Some(vec!["Git status unavailable.".to_string()]);
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();

        cx.spawn(move |this: WeakEntity<DiffReviewState>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = summary_task.await;
                this.update(&mut cx, |view, cx| {
                    if view.session_id != Some(session_id) {
                        return;
                    }
                    match result {
                        Ok(summary) => {
                            view.diff_summary = Some(DiffSummary {
                                file_count: summary.file_count,
                                additions: summary.additions,
                                deletions: summary.deletions,
                            });
                        }
                        Err(_) => {
                            view.diff_summary = None;
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
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
        self.git_status = None;
        self.diff_summary = None;
        self.error = None;
        self.list_resizing = false;
        self.list_resize_state = None;
    }

    fn ensure_active_file(&mut self) {
        if let Some(active) = &self.active_file_key {
            if self.files.iter().any(|file| &file.key == active) {
                return;
            }
        }
        self.active_file_key = None;
    }
}

#[derive(Clone, Debug)]
struct RawDiffFile {
    key: String,
    old_path: String,
    new_path: String,
    section_lines: Vec<String>,
    header_lines: Vec<String>,
    hunks: Vec<DiffHunk>,
}

fn parse_unified_diff(diff_text: &str) -> Vec<DiffFile> {
    let mut lines = diff_text.split('\n').collect::<Vec<_>>();
    if matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    let mut files = Vec::new();
    let mut current: Option<RawDiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut in_header = false;

    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ") {
            push_current(&mut files, &mut current, &mut current_hunk, &mut in_header);

            let mut parts = line.trim_start_matches("diff --git ").split_whitespace();
            let old_raw = parts.next().unwrap_or("");
            let new_raw = parts.next().unwrap_or("");
            let old_path = old_raw.strip_prefix("a/").unwrap_or(old_raw).to_string();
            let new_path = new_raw.strip_prefix("b/").unwrap_or(new_raw).to_string();
            let key = format!("{old_path}=>{new_path}:{idx}");
            current = Some(RawDiffFile {
                key,
                old_path,
                new_path,
                section_lines: vec![line.to_string()],
                header_lines: vec![line.to_string()],
                hunks: Vec::new(),
            });
            in_header = true;
            continue;
        }

        let Some(current_file) = current.as_mut() else {
            continue;
        };
        current_file.section_lines.push((*line).to_string());

        if line.starts_with("@@ ") {
            if let Some(hunk) = current_hunk.take() {
                current_file.hunks.push(hunk);
            }
            current_hunk = Some(DiffHunk {
                key: format!("{}:h{}:{}", current_file.key, current_file.hunks.len(), idx),
                header_line: (*line).to_string(),
                lines: Vec::new(),
            });
            in_header = false;
            continue;
        }

        if in_header {
            current_file.header_lines.push((*line).to_string());
        } else if let Some(hunk) = current_hunk.as_mut() {
            hunk.lines.push((*line).to_string());
        }
    }

    push_current(&mut files, &mut current, &mut current_hunk, &mut in_header);
    files
        .into_iter()
        .filter(|file| file.section_lines.iter().any(|line| !line.trim().is_empty()))
        .collect()
}

fn push_current(
    files: &mut Vec<DiffFile>,
    current: &mut Option<RawDiffFile>,
    current_hunk: &mut Option<DiffHunk>,
    in_header: &mut bool,
) {
    let Some(mut raw) = current.take() else {
        return;
    };
    if let Some(hunk) = current_hunk.take() {
        raw.hunks.push(hunk);
    }
    files.push(finalize_file(raw));
    *in_header = false;
}

fn finalize_file(raw: RawDiffFile) -> DiffFile {
    let file_path = if !raw.new_path.is_empty() && raw.new_path != "dev/null" {
        raw.new_path.clone()
    } else if !raw.old_path.is_empty() && raw.old_path != "dev/null" {
        raw.old_path.clone()
    } else {
        "(unknown)".to_string()
    };
    let header_text = raw.header_lines.join("\n");
    let is_new = raw.old_path == "dev/null"
        || header_text.contains("new file mode")
        || header_text.contains("--- /dev/null");
    let is_deleted = raw.new_path == "dev/null"
        || header_text.contains("deleted file mode")
        || header_text.contains("+++ /dev/null");
    let patch_text = raw.section_lines.join("\n");
    let is_binary = patch_text.contains("GIT binary patch") || patch_text.contains("Binary files");

    let mut added_lines = 0;
    let mut deleted_lines = 0;
    let mut has_render_lines = false;

    for hunk in &raw.hunks {
        for line in &hunk.lines {
            if line.is_empty() {
                continue;
            }
            let prefix = line.as_bytes()[0] as char;
            match prefix {
                '+' => {
                    if !line.starts_with("+++") {
                        added_lines += 1;
                        has_render_lines = true;
                    }
                }
                '-' => {
                    if !line.starts_with("---") {
                        deleted_lines += 1;
                        has_render_lines = true;
                    }
                }
                ' ' => {
                    has_render_lines = true;
                }
                _ => {}
            }
        }
    }

    DiffFile {
        key: raw.key,
        file_path,
        section_lines: raw.section_lines,
        header_lines: raw.header_lines,
        hunks: raw.hunks,
        is_new,
        is_deleted,
        is_binary: is_binary || !has_render_lines,
        added_lines,
        deleted_lines,
    }
}
