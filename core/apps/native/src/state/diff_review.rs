use gpui::Context;
use gpui_tokio::Tokio;

use ctx_core::ids::TrackId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Add,
    Del,
    Context,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffRenderLine {
    pub(crate) text: String,
    pub(crate) kind: DiffLineKind,
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
    pub(crate) old_path: String,
    pub(crate) new_path: String,
    pub(crate) file_path: String,
    pub(crate) section_lines: Vec<String>,
    pub(crate) header_lines: Vec<String>,
    pub(crate) hunks: Vec<DiffHunk>,
    pub(crate) is_new: bool,
    pub(crate) is_deleted: bool,
    pub(crate) is_binary: bool,
    pub(crate) added_lines: usize,
    pub(crate) deleted_lines: usize,
    pub(crate) render_lines: Vec<DiffRenderLine>,
}

impl DiffFile {
    pub(crate) fn patch_text(&self) -> String {
        let mut patch = self.section_lines.join("\n");
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

#[derive(Debug)]
pub(crate) struct DiffReviewState {
    pub(crate) track_id: Option<TrackId>,
    pub(crate) diff: String,
    pub(crate) files: Vec<DiffFile>,
    pub(crate) active_file_key: Option<String>,
    pub(crate) busy_key: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
}

impl DiffReviewState {
    pub(crate) fn new() -> Self {
        Self {
            track_id: None,
            diff: String::new(),
            files: Vec::new(),
            active_file_key: None,
            busy_key: None,
            status: None,
            error: None,
        }
    }

    pub(crate) fn set_track_id(&mut self, track_id: Option<TrackId>, cx: &mut Context<Self>) {
        if self.track_id == track_id {
            return;
        }
        self.track_id = track_id;
        self.reset_state();
        if self.track_id.is_some() {
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
            return;
        }
        self.active_file_key = Some(key);
        cx.notify();
    }

    pub(crate) fn reload_diff(&mut self, cx: &mut Context<Self>) {
        let Some(track_id) = self.track_id else {
            return;
        };
        self.busy_key = Some("diff:load".to_string());
        self.status = None;
        self.error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.get_track_diff(track_id).await
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.track_id != Some(track_id) {
                    return;
                }
                view.busy_key = None;
                match result {
                    Ok(diff) => {
                        view.set_diff(diff);
                    }
                    Err(err) => {
                        view.error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
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

    fn reset_state(&mut self) {
        self.diff.clear();
        self.files.clear();
        self.active_file_key = None;
        self.busy_key = None;
        self.status = None;
        self.error = None;
    }

    fn ensure_active_file(&mut self) {
        if let Some(active) = &self.active_file_key {
            if self.files.iter().any(|file| &file.key == active) {
                return;
            }
        }
        self.active_file_key = self.files.first().map(|file| file.key.clone());
    }

    fn apply_patch(
        &mut self,
        busy_key: String,
        action: DiffPatchAction,
        patch: String,
        status_message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(track_id) = self.track_id else {
            return;
        };
        if self.busy_key.is_some() {
            return;
        }
        self.busy_key = Some(busy_key);
        self.status = None;
        self.error = None;
        cx.notify();

        let action_label = action.as_str().to_string();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client
                .apply_track_diff_patch(track_id, &action_label, &patch)
                .await
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.track_id != Some(track_id) {
                    return;
                }
                view.busy_key = None;
                match result {
                    Ok(diff) => {
                        view.set_diff(diff);
                        view.status = Some(status_message);
                    }
                    Err(err) => {
                        view.error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
    let mut render_lines = Vec::new();

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
                        render_lines.push(DiffRenderLine {
                            text: line[1..].to_string(),
                            kind: DiffLineKind::Add,
                        });
                    }
                }
                '-' => {
                    if !line.starts_with("---") {
                        deleted_lines += 1;
                        render_lines.push(DiffRenderLine {
                            text: line[1..].to_string(),
                            kind: DiffLineKind::Del,
                        });
                    }
                }
                ' ' => {
                    render_lines.push(DiffRenderLine {
                        text: line[1..].to_string(),
                        kind: DiffLineKind::Context,
                    });
                }
                _ => {}
            }
        }
    }

    DiffFile {
        key: raw.key,
        old_path: raw.old_path,
        new_path: raw.new_path,
        file_path,
        section_lines: raw.section_lines,
        header_lines: raw.header_lines,
        hunks: raw.hunks,
        is_new,
        is_deleted,
        is_binary: is_binary || render_lines.is_empty(),
        added_lines,
        deleted_lines,
        render_lines,
    }
}
