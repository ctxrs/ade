use gpui::{ClickEvent, Context, Rgba, div, prelude::*, px};

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::state::diff_review::{
    DiffFile, DiffLineKind, DiffPatchAction, DiffReviewState,
};

const MONO_FONT_FAMILY: &str =
    "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace";

pub(super) struct DiffReviewView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) state: &'a DiffReviewState,
}

impl<'a> DiffReviewView<'a> {
    pub(super) fn render(&self, cx: &mut Context<DiffReviewState>) -> impl IntoElement {
        let has_changes = !self.state.diff.trim().is_empty();
        let is_loading = self.state.busy_key.as_deref() == Some("diff:load");
        let mut root = div().flex().flex_col().gap_2();

        if let Some(error) = &self.state.error {
            root = root.child(
                div()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(self.colors.error)
                    .rounded_sm()
                    .bg(self.colors.panel_2)
                    .text_sm()
                    .text_color(self.colors.error)
                    .child(error.clone()),
            );
        }

        if let Some(status) = &self.state.status {
            root = root.child(
                div()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(self.colors.success)
                    .rounded_sm()
                    .bg(self.colors.panel_2)
                    .text_sm()
                    .text_color(self.colors.success)
                    .child(status.clone()),
            );
        }

        if is_loading {
            root = root.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Loading diff..."),
            );
            if !has_changes {
                return root;
            }
        }

        if !has_changes {
            return root.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("No changes."),
            );
        }

        let can_apply_all = self.state.track_id.is_some() && self.state.busy_key.is_none();
        let apply_all_busy = self
            .state
            .busy_key
            .as_deref()
            .map(|key| key.starts_with("diff:apply:"))
            .unwrap_or(false);

        let apply_all_actions = if apply_all_busy {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("Applying...")
        } else {
            let approve_message = "Approved all changes.".to_string();
            let on_approve = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.apply_all_patch(DiffPatchAction::Accept, approve_message.clone(), cx);
            });
            let reject_message = "Rejected all changes.".to_string();
            let on_reject = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.apply_all_patch(DiffPatchAction::Reject, reject_message.clone(), cx);
            });

            let mut reject_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Reject")
                .id("diff-reject-all");
            let mut approve_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Approve")
                .id("diff-approve-all");

            if can_apply_all {
                reject_button = reject_button
                    .text_color(self.colors.error)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(on_reject);
                approve_button = approve_button
                    .text_color(self.colors.success)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(on_approve);
            } else {
                reject_button = reject_button.text_color(self.colors.muted);
                approve_button = approve_button.text_color(self.colors.muted);
            }

            div()
                .flex()
                .items_center()
                .gap_2()
                .child(approve_button)
                .child(reject_button)
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(Icon::new(IconName::Diff, 12.0, self.colors.muted))
                    .child(div().text_color(self.colors.text).child("All changes"))
                    .child(
                        div()
                            .px_1()
                            .py_0()
                            .text_sm()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel)
                            .text_color(self.colors.muted)
                            .child(format!("{}", self.state.files.len())),
                    ),
            )
            .child(apply_all_actions);

        let list = if self.state.files.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No files to review.")
        } else {
            self.state.files.iter().fold(
                div().flex().flex_col().gap_2(),
                |list, file| {
                    let is_selected =
                        self.state.active_file_key.as_deref() == Some(file.key.as_str());
                    let item_bg = if is_selected {
                        self.colors.panel
                    } else {
                        self.colors.panel_2
                    };
                    let item_border = if is_selected {
                        self.colors.border_strong
                    } else {
                        self.colors.border
                    };
                    let select_key = file.key.clone();
                    let on_select = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_file(select_key.clone(), cx);
                    });

                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(file.file_path.clone())
                            .child(div().flex_1())
                            .child(file_summary(file, self.colors))
                            .cursor_pointer()
                            .id(format!("diff-file-{}", file.key))
                            .on_click(on_select),
                    )
                },
            )
        };

        let detail = if let Some(active_key) = self.state.active_file_key.as_deref() {
            if let Some(file) = self.state.files.iter().find(|file| file.key == active_key) {
                let file_busy = self.state.busy_key.as_deref() == Some(file.key.as_str());
                let can_apply = self.state.track_id.is_some() && self.state.busy_key.is_none();
                let patch = file.patch_text();

                let actions = if file_busy {
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Applying...")
                } else {
                    let keep_key = file.key.clone();
                    let keep_patch = patch.clone();
                    let keep_message =
                        format!("Kept changes in {}.", file.file_path.as_str());
                    let on_keep = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.apply_file_patch(
                            keep_key.clone(),
                            DiffPatchAction::Accept,
                            keep_patch.clone(),
                            keep_message.clone(),
                            cx,
                        );
                    });
                    let undo_key = file.key.clone();
                    let undo_patch = patch.clone();
                    let undo_message =
                        format!("Undid changes in {}.", file.file_path.as_str());
                    let on_undo = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.apply_file_patch(
                            undo_key.clone(),
                            DiffPatchAction::Reject,
                            undo_patch.clone(),
                            undo_message.clone(),
                            cx,
                        );
                    });

                    let mut undo_button = div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .child("Undo")
                        .id(format!("diff-file-undo-{}", file.key));
                    let mut keep_button = div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .child("Keep")
                        .id(format!("diff-file-keep-{}", file.key));

                    if can_apply {
                        undo_button = undo_button
                            .text_color(self.colors.error)
                            .cursor_pointer()
                            .active(|style| style.opacity(0.85))
                            .on_click(on_undo);
                        keep_button = keep_button
                            .text_color(self.colors.success)
                            .cursor_pointer()
                            .active(|style| style.opacity(0.85))
                            .on_click(on_keep);
                    } else {
                        undo_button = undo_button.text_color(self.colors.muted);
                        keep_button = keep_button.text_color(self.colors.muted);
                    }

                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(undo_button)
                        .child(keep_button)
                };

                let summary = file_summary(file, self.colors);
                let detail_header = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().child(file.file_path.clone()))
                    .child(summary)
                    .child(div().flex_1())
                    .child(actions);

                let preview = if file.is_binary {
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Binary or metadata-only diff.")
                        .into_any_element()
                } else {
                    render_inline_diff(file, self.state, self.colors, cx).into_any_element()
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(detail_header)
                    .child(
                        div()
                            .border_1()
                            .border_color(self.colors.border)
                            .rounded_sm()
                            .bg(self.colors.panel)
                            .p_2()
                            .child(preview),
                    )
            } else {
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Select a file to review.")
            }
        } else {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("Select a file to review.")
        };

        root.child(header).child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(px(240.0))
                        .child(list),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .p_3()
                        .bg(self.colors.panel_2)
                        .child(detail),
                ),
        )
    }
}

fn file_summary(file: &DiffFile, colors: ThemeColors) -> impl IntoElement {
    if file.is_new {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().text_color(colors.success).child("(New)"))
            .child(
                div()
                    .text_color(colors.success)
                    .child(format!("+{}", file.added_lines)),
            )
    } else if file.is_deleted {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().text_color(colors.error).child("(Deleted)"))
            .child(
                div()
                    .text_color(colors.error)
                    .child(format!("-{}", file.deleted_lines)),
            )
    } else {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_color(colors.success)
                    .child(format!("+{}", file.added_lines)),
            )
            .child(
                div()
                    .text_color(colors.error)
                    .child(format!("-{}", file.deleted_lines)),
            )
    }
}

fn render_inline_diff(
    file: &DiffFile,
    state: &DiffReviewState,
    colors: ThemeColors,
    cx: &mut Context<DiffReviewState>,
) -> impl IntoElement {
    if file.hunks.is_empty() {
        return div()
            .text_sm()
            .text_color(colors.muted)
            .child("No hunks to display.");
    }

    let mut hunks = div().flex().flex_col().gap_2();
    let can_apply = state.track_id.is_some() && state.busy_key.is_none();

    for hunk in &file.hunks {
        let hunk_busy = state.busy_key.as_deref() == Some(hunk.key.as_str());
        let hunk_patch = file.hunk_patch_text(hunk);

            let actions = if hunk_busy {
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child("Applying...")
            } else {
            let keep_key = hunk.key.clone();
            let keep_patch = hunk_patch.clone();
            let keep_message = format!("Kept hunk in {}.", file.file_path.as_str());
            let on_keep = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.apply_hunk_patch(
                    keep_key.clone(),
                    DiffPatchAction::Accept,
                    keep_patch.clone(),
                    keep_message.clone(),
                    cx,
                );
            });

            let undo_key = hunk.key.clone();
            let undo_patch = hunk_patch.clone();
            let undo_message = format!("Undid hunk in {}.", file.file_path.as_str());
            let on_undo = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                view.apply_hunk_patch(
                    undo_key.clone(),
                    DiffPatchAction::Reject,
                    undo_patch.clone(),
                    undo_message.clone(),
                    cx,
                );
            });

            let mut undo_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(colors.border)
                .rounded_sm()
                .child("Undo")
                .id(format!("diff-hunk-undo-{}", hunk.key));
            let mut keep_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(colors.border)
                .rounded_sm()
                .child("Keep")
                .id(format!("diff-hunk-keep-{}", hunk.key));

            if can_apply {
                undo_button = undo_button
                    .text_color(colors.error)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(on_undo);
                keep_button = keep_button
                    .text_color(colors.success)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(on_keep);
            } else {
                undo_button = undo_button.text_color(colors.muted);
                keep_button = keep_button.text_color(colors.muted);
            }

            div()
                .flex()
                .items_center()
                .gap_2()
                .child(undo_button)
                .child(keep_button)
        };

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .bg(colors.panel_2)
            .child(
                div()
                    .text_sm()
                    .font_family(MONO_FONT_FAMILY)
                    .text_color(colors.muted)
                    .child(hunk.header_line.clone()),
            )
            .child(div().flex_1())
            .child(actions);

        let mut lines = div()
            .flex()
            .flex_col()
            .gap_0()
            .px_1()
            .py_1()
            .text_sm()
            .font_family(MONO_FONT_FAMILY)
            .whitespace_nowrap()
            .id(format!("diff-hunk-lines-{}", hunk.key))
            .overflow_x_scroll()
            .w_full();

        for line in &hunk.lines {
            let (kind, content, prefix, is_note) = diff_line_parts(line);
            let color = if is_note {
                colors.muted
            } else {
                diff_line_color(kind, colors)
            };
            let mut row = div()
                .text_color(color)
                .whitespace_nowrap()
                .child(format!("{prefix}{content}"));
            if !is_note {
                if let Some(bg) = diff_line_background(kind, colors) {
                    row = row.bg(bg);
                }
            }
            lines = lines.child(row);
        }

        hunks = hunks.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .border_1()
                .border_color(colors.border)
                .rounded_sm()
                .bg(colors.panel)
                .child(header)
                .child(lines),
        );
    }

    hunks
}

fn diff_line_color(kind: DiffLineKind, colors: ThemeColors) -> gpui::Rgba {
    match kind {
        DiffLineKind::Add => colors.success,
        DiffLineKind::Del => colors.error,
        DiffLineKind::Context => colors.text,
    }
}

fn diff_line_background(kind: DiffLineKind, colors: ThemeColors) -> Option<Rgba> {
    let alpha = 0.12;
    match kind {
        DiffLineKind::Add => Some(blend_tint(colors.panel, colors.success, alpha)),
        DiffLineKind::Del => Some(blend_tint(colors.panel, colors.error, alpha)),
        DiffLineKind::Context => None,
    }
}

fn blend_tint(base: Rgba, tint: Rgba, alpha: f32) -> Rgba {
    base.blend(Rgba {
        r: tint.r,
        g: tint.g,
        b: tint.b,
        a: alpha,
    })
}

fn diff_line_parts(line: &str) -> (DiffLineKind, &str, char, bool) {
    let mut chars = line.chars();
    match chars.next() {
        Some('+') => (DiffLineKind::Add, &line[1..], '+', false),
        Some('-') => (DiffLineKind::Del, &line[1..], '-', false),
        Some(' ') => (DiffLineKind::Context, &line[1..], ' ', false),
        Some('\\') => (DiffLineKind::Context, line, ' ', true),
        Some(_) | None => (DiffLineKind::Context, line, ' ', false),
    }
}
