use gpui::{
    ClickEvent, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Rgba, div,
    prelude::*, px,
};
use gpui_component::scroll::ScrollableElement;

use crate::theme::{ThemeColors, ThemeMetrics};

use super::super::icons::{Icon, IconName};
use super::super::state::diff_review::{
    DiffFile, DiffLineKind, DiffListResizeState, DiffPatchAction, DiffReviewState,
};

const MONO_FONT_FAMILY: &str =
    "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace";

pub(super) struct DiffReviewView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) state: &'a DiffReviewState,
}

impl<'a> DiffReviewView<'a> {
    pub(super) fn render(&self, cx: &mut Context<DiffReviewState>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let has_changes = !self.state.diff.trim().is_empty();
        let is_loading = self.state.busy_key.as_deref() == Some("diff:load");
        let mut root = div()
            .flex()
            .flex_col()
            .gap_2()
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _window, cx| {
                let Some(state) = view.list_resize_state else {
                    return;
                };
                let delta = f32::from(event.position.x) - state.start_x;
                view.set_list_width(state.start_width + delta, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _window, cx| {
                    if view.list_resize_state.is_some() || view.list_resizing {
                        view.list_resizing = false;
                        view.list_resize_state = None;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _window, cx| {
                    if view.list_resize_state.is_some() || view.list_resizing {
                        view.list_resizing = false;
                        view.list_resize_state = None;
                        cx.notify();
                    }
                }),
            );

        if let Some(error) = &self.state.error {
            root = root.child(
                div()
                    .px(px(metrics.spacing.md))
                    .py(px(metrics.spacing.sm))
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
                    .px(px(metrics.spacing.md))
                    .py(px(metrics.spacing.sm))
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

        let can_apply_all = self.state.worktree_id.is_some() && self.state.busy_key.is_none();
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
                .px(px(metrics.spacing.xxl))
                .py(px(metrics.spacing.sm))
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_full()
                .child("Reject")
                .bg(self.colors.panel)
                .id("diff-reject-all");
            let mut approve_button = div()
                .px(px(metrics.spacing.xxl))
                .py(px(metrics.spacing.sm))
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_full()
                .child("Approve")
                .bg(self.colors.accent)
                .text_color(self.colors.text)
                .id("diff-approve-all");

            if can_apply_all {
                reject_button = reject_button
                    .text_color(self.colors.text)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.92))
                    .on_click(on_reject);
                approve_button = approve_button
                    .cursor_pointer()
                    .active(|style| style.opacity(0.92))
                    .on_click(on_approve);
            } else {
                reject_button = reject_button.text_color(self.colors.muted).bg(self.colors.panel_2);
                approve_button = approve_button.text_color(self.colors.muted).bg(self.colors.panel_2);
            }

            div()
                .flex()
                .items_center()
                .gap_2()
                .child(approve_button)
                .child(reject_button)
        };

        let can_refresh = self.state.worktree_id.is_some() && self.state.busy_key.is_none();
        let on_refresh = cx.listener(|view, _: &ClickEvent, _window, cx| {
            view.reload_diff(cx);
        });
        let refresh_icon_color = if can_refresh {
            self.colors.text
        } else {
            self.colors.muted
        };
        let refresh_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::Refresh, 12.0, refresh_icon_color))
            .child("Refresh");
        let mut refresh_button = div()
            .px(px(metrics.spacing.lg))
            .py(px(metrics.spacing.xs))
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_full()
            .child(refresh_label)
            .id("diff-refresh");
        if can_refresh {
            refresh_button = refresh_button
                .bg(self.colors.panel)
                .text_color(self.colors.text)
                .cursor_pointer()
                .active(|style| style.opacity(0.92))
                .on_click(on_refresh);
        } else {
            refresh_button = refresh_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }

        let file_count = self.state.files.len();
        let total_added: usize = self.state.files.iter().map(|file| file.added_lines).sum();
        let total_deleted: usize = self.state.files.iter().map(|file| file.deleted_lines).sum();
        let pending_label = format!(
            "{} Pending Change{}",
            file_count,
            if file_count == 1 { "" } else { "s" }
        );
        let totals_pill = div()
            .px(px(metrics.spacing.lg))
            .py(px(metrics.spacing.xxs))
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_full()
            .bg(self.colors.panel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_color(self.colors.success)
                            .child(format!("+{total_added}")),
                    )
                    .child(
                        div()
                            .text_color(self.colors.error)
                            .child(format!("-{total_deleted}")),
                    ),
            );
        let pending_pill = div()
            .px(px(metrics.spacing.lg))
            .py(px(metrics.spacing.xxs))
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_full()
            .bg(self.colors.panel)
            .text_color(self.colors.text)
            .child(pending_label);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(metrics.spacing.xl))
            .py(px(metrics.spacing.lg))
            .border_b_1()
            .border_color(self.colors.border)
            .bg(self.colors.panel)
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Diff, 14.0, self.colors.muted))
                    .child(div().text_color(self.colors.text).child("All changes"))
                    .child(pending_pill)
                    .child(totals_pill),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(refresh_button)
                    .child(apply_all_actions),
            );

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
                            .px(px(metrics.spacing.xl))
                            .py(px(metrics.spacing.md))
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
                let can_apply = self.state.worktree_id.is_some() && self.state.busy_key.is_none();
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
                        .px(px(metrics.spacing.xxl))
                        .py(px(metrics.spacing.sm))
                        .text_sm()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_full()
                        .bg(self.colors.panel)
                        .child("Undo")
                        .id(format!("diff-file-undo-{}", file.key));
                    let mut keep_button = div()
                        .px(px(metrics.spacing.xxl))
                        .py(px(metrics.spacing.sm))
                        .text_sm()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_full()
                        .bg(self.colors.panel)
                        .child("Keep")
                        .id(format!("diff-file-keep-{}", file.key));

                    if can_apply {
                        undo_button = undo_button
                            .text_color(self.colors.error)
                            .cursor_pointer()
                            .active(|style| style.opacity(0.92))
                            .on_click(on_undo);
                        keep_button = keep_button
                            .text_color(self.colors.success)
                            .cursor_pointer()
                            .active(|style| style.opacity(0.92))
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
                    .gap_3()
                    .px(px(metrics.spacing.xl))
                    .py(px(metrics.spacing.md))
                    .bg(self.colors.panel)
                    .border_b_1()
                    .border_color(self.colors.border)
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
                            .p(px(metrics.spacing.xl))
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

        let resizer_color = if self.state.list_resizing {
            self.colors.accent
        } else {
            self.colors.border
        };
        let resizer = div()
            .w(px(6.0))
            .flex_none()
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    if event.click_count >= 2 {
                        view.reset_list_width(cx);
                        view.list_resizing = false;
                        view.list_resize_state = None;
                        cx.notify();
                        return;
                    }
                    view.list_resizing = true;
                    view.list_resize_state = Some(DiffListResizeState {
                        start_x: f32::from(event.position.x),
                        start_width: view.list_width,
                    });
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(2.0))
                    .h_full()
                    .rounded_full()
                    .bg(resizer_color)
                    .hover(|style| style.bg(self.colors.accent)),
            );

        let list_container = div()
            .flex()
            .flex_col()
            .w(px(self.state.list_width))
            .min_w(px(0.0))
            .p(px(metrics.spacing.xl))
            .gap_2()
            .overflow_y_scrollbar()
            .child(list);
        let detail_container = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .border_l_1()
            .border_color(self.colors.border)
            .bg(self.colors.panel_2)
            .overflow_y_scrollbar()
            .child(detail);

        root.child(header).child(
            div()
                .flex()
                .flex_row()
                .gap_0()
                .min_h(px(0.0))
                .child(list_container)
                .child(resizer)
                .child(detail_container),
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

    let metrics = ThemeMetrics::default();
    let mut hunks = div().flex().flex_col().gap_2();
    let can_apply = state.worktree_id.is_some() && state.busy_key.is_none();

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
                .px(px(metrics.spacing.xxl))
                .py(px(metrics.spacing.sm))
                .text_sm()
                .border_1()
                .border_color(colors.border)
                .rounded_full()
                .bg(colors.panel)
                .child("Undo")
                .id(format!("diff-hunk-undo-{}", hunk.key));
            let mut keep_button = div()
                .px(px(metrics.spacing.xxl))
                .py(px(metrics.spacing.sm))
                .text_sm()
                .border_1()
                .border_color(colors.border)
                .rounded_full()
                .bg(colors.panel)
                .child("Keep")
                .id(format!("diff-hunk-keep-{}", hunk.key));

            if can_apply {
                undo_button = undo_button
                    .text_color(colors.error)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.92))
                    .on_click(on_undo);
                keep_button = keep_button
                    .text_color(colors.success)
                    .cursor_pointer()
                    .active(|style| style.opacity(0.92))
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
            .gap_3()
            .px(px(metrics.spacing.xl))
            .py(px(metrics.spacing.md))
            .bg(colors.panel)
            .border_b_1()
            .border_color(colors.border)
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
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
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
