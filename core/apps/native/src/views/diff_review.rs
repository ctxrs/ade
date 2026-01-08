use gpui::{ClickEvent, Context, div, prelude::*, px};

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::state::diff_review::{
    DiffFile, DiffLineKind, DiffPatchAction, DiffReviewState,
};

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
                .child("Reject");
            let mut approve_button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child("Approve");

            if can_apply_all {
                reject_button = reject_button
                    .text_color(self.colors.error)
                    .cursor_pointer()
                    .active(|this| this.opacity(0.85))
                    .on_click(on_reject);
                approve_button = approve_button
                    .text_color(self.colors.success)
                    .cursor_pointer()
                    .active(|this| this.opacity(0.85))
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
                            .child(file.file_path.as_str())
                            .child(div().flex_1())
                            .child(file_summary(file, self.colors))
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
                        .child("Undo");
                    let mut keep_button = div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .child("Keep");

                    if can_apply {
                        undo_button = undo_button
                            .text_color(self.colors.error)
                            .cursor_pointer()
                            .active(|this| this.opacity(0.85))
                            .on_click(on_undo);
                        keep_button = keep_button
                            .text_color(self.colors.success)
                            .cursor_pointer()
                            .active(|this| this.opacity(0.85))
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
                    .child(div().text_sm().child(file.file_path.as_str()))
                    .child(summary)
                    .child(div().flex_1())
                    .child(actions);

                let preview = if file.is_binary {
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Binary or metadata-only diff.")
                } else {
                    render_diff_lines(file, self.colors)
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

fn render_diff_lines(file: &DiffFile, colors: ThemeColors) -> impl IntoElement {
    let mut lines = div().flex().flex_col().gap_1();
    for line in &file.render_lines {
        let prefix = match line.kind {
            DiffLineKind::Add => "+",
            DiffLineKind::Del => "-",
            DiffLineKind::Context => " ",
        };
        let text = format!("{prefix}{}", line.text);
        let color = diff_line_color(line.kind, colors);
        lines = lines.child(div().text_sm().text_color(color).child(text));
    }
    lines
}

fn diff_line_color(kind: DiffLineKind, colors: ThemeColors) -> gpui::Rgba {
    match kind {
        DiffLineKind::Add => colors.success,
        DiffLineKind::Del => colors.error,
        DiffLineKind::Context => colors.text,
    }
}
