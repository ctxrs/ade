use gpui::{ClickEvent, Context, FontWeight, div, prelude::*, px};

use crate::{automation_tree, theme::ThemeMetrics};

use super::artifacts::ArtifactsView;
use super::composer::{ComposerVariant, ComposerView};
use super::diff_review::DiffReviewView;
use super::messages::ThreadListView;
use super::sessions_pane::SessionsPaneView;
use super::super::icons::{Icon, IconName};
use super::super::state::{DataLoadState, ShellView};

pub(crate) struct SessionView<'a> {
    pub(super) shell: &'a ShellView,
    pub(super) resyncing: bool,
}

impl<'a> SessionView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;
        let metrics = ThemeMetrics::default();
        if shell.new_task_mode {
            let composer = ComposerView {
                shell,
                variant: ComposerVariant::NewTask,
            }
            .render(cx);
            return div()
                .id("session-view")
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(div().w_full().flex().justify_center().child(composer));
        }
        let has_session = shell
            .selected_session
            .and_then(|index| shell.sessions.get(index))
            .is_some();
        if !shell.show_sessions_pane {
            automation_tree::register_hidden(
                "sessions-list",
                "list",
                Some("Sessions"),
                Some("app-shell"),
            );
            automation_tree::register_hidden(
                "sessions-pane",
                "pane",
                Some("Sessions"),
                Some("app-shell"),
            );
        }
        if !shell.show_diff_pane {
            automation_tree::register_hidden(
                "diff-pane",
                "pane",
                Some("Diff"),
                Some("app-shell"),
            );
        }
        if !shell.show_artifacts_pane {
            automation_tree::register_hidden(
                "artifacts-pane",
                "pane",
                Some("Artifacts"),
                Some("app-shell"),
            );
        }
        if !shell.show_terminal_panel {
            automation_tree::register_hidden(
                "terminal-panel",
                "pane",
                Some("Terminal"),
                Some("app-shell"),
            );
        }
        let interrupt_color = if has_session {
            shell.colors.text
        } else {
            shell.colors.muted
        };
        let interrupt_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(
                IconName::Interrupt,
                12.0,
                interrupt_color,
            ))
            .child("Interrupt");
        let mut interrupt_button = div()
            // Match web pill-button-inner: 4px x 10px
            .px(px(metrics.spacing.lg))
            .py(px(metrics.spacing.xs))
            .text_sm()
            .border_1()
            .border_color(shell.colors.border)
            .rounded_full()
            .child(interrupt_label)
            .id("session-interrupt");

        if has_session {
            interrupt_button = interrupt_button
                .bg(shell.colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::on_interrupt_click));
        } else {
            interrupt_button = interrupt_button
                .bg(shell.colors.panel_2)
                .text_color(shell.colors.muted);
        }

        let cancel_color = if has_session {
            shell.colors.warning
        } else {
            shell.colors.muted
        };
        let cancel_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::Cancel, 12.0, cancel_color))
            .child("Cancel");
        let mut cancel_button = div()
            // Match web pill-button-inner: 4px x 10px
            .px(px(metrics.spacing.lg))
            .py(px(metrics.spacing.xs))
            .text_sm()
            .border_1()
            .border_color(shell.colors.border)
            .rounded_full()
            .child(cancel_label)
            .id("session-cancel");

        if has_session {
            cancel_button = cancel_button
                .bg(shell.colors.panel)
                .text_color(shell.colors.warning)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::on_cancel_click));
        } else {
            cancel_button = cancel_button
                .bg(shell.colors.panel_2)
                .text_color(shell.colors.muted);
        }
        /*
        let sessions_toggle = {
            let (bg, color) = if shell.show_sessions_pane {
                (shell.colors.panel, shell.colors.text)
            } else {
                (shell.colors.panel_2, shell.colors.muted)
            };
            div()
                // Match web .wb-icon 22x22, radius 8
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(bg)
                .child(Icon::new(IconName::Sessions, 14.0, color))
                .cursor_pointer()
                .id("pane-toggle-sessions")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_sessions_pane))
        };
        */

        let diff_toggle = {
            let (bg, color) = if shell.show_diff_pane {
                (shell.colors.panel, shell.colors.text)
            } else {
                (shell.colors.panel_2, shell.colors.muted)
            };
            div()
                // Match web .wb-icon 22x22, radius 8
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(bg)
                .child(Icon::new(IconName::Diff, 14.0, color))
                .cursor_pointer()
                .id("pane-toggle-diff")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_diff_pane))
        };

        let artifacts_toggle = {
            let (bg, color) = if shell.show_artifacts_pane {
                (shell.colors.panel, shell.colors.text)
            } else {
                (shell.colors.panel_2, shell.colors.muted)
            };
            div()
                // Match web .wb-icon 22x22, radius 8
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(bg)
                .child(Icon::new(IconName::Image, 14.0, color))
                .cursor_pointer()
                .id("pane-toggle-artifacts")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_artifacts_pane))
        };

        let terminal_toggle = {
            let (bg, color) = if shell.show_terminal_panel {
                (shell.colors.panel, shell.colors.text)
            } else {
                (shell.colors.panel_2, shell.colors.muted)
            };
            div()
                // Match web .wb-icon 22x22, radius 8
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(bg)
                .child(Icon::new(IconName::Terminal, 14.0, color))
                .cursor_pointer()
                .id("pane-toggle-terminal")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_terminal_panel))
        };

        let selected_session_id = shell
            .selected_session
            .and_then(|index| shell.sessions.get(index))
            .map(|summary| summary.session_id);
        let mut worktree_id = None;
        if let Some(session_id) = selected_session_id {
            if let Some(summary) = shell.session_summary_map.get(&session_id) {
                worktree_id = Some(summary.session.worktree_id.0.to_string());
            }
        }

        let worktree_chip = worktree_id.as_ref().map(|worktree_id| {
            let short_id = worktree_id.chars().take(8).collect::<String>();
            let copy_key = format!("worktree:{worktree_id}");
            let copy_value = worktree_id.clone();
            div()
                .flex()
                .items_center()
                .gap(px(metrics.spacing.xs))
                .px(px(metrics.spacing.sm))
                .py(px(metrics.spacing.xxs))
                .rounded_full()
                .border_1()
                .border_color(shell.colors.border)
                .bg(shell.colors.panel)
                .text_sm()
                .text_color(shell.colors.muted)
                .child(Icon::new(IconName::Folder, 12.0, shell.colors.text))
                .child(format!("Worktree {short_id}"))
                .cursor_pointer()
                .id("session-worktree-chip")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.copy_text_with_key(copy_key.clone(), copy_value.clone(), cx);
                }))
        });

        let mut title_row = div()
            .flex()
            .items_end()
            .gap(px(metrics.spacing.md))
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight(500.0))
                    .child(shell.session.title.clone()),
            );
        if let Some(chip) = worktree_chip {
            title_row = title_row
                .child(div().text_color(shell.colors.muted).child("·"))
                .child(chip);
        }

        let pane_toggle_row = div()
            .flex()
            .items_center()
            .gap(px(metrics.spacing.md))
            .child(artifacts_toggle)
            .child(diff_toggle)
            // .child(sessions_toggle)
            .child(terminal_toggle);

        let control_row = div()
            .flex()
            .items_center()
            .gap(px(metrics.spacing.lg))
            .child(pane_toggle_row)
            .child(interrupt_button)
            .child(cancel_button);

        let header_block = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(metrics.spacing.lg))
            .child(title_row)
            .child(control_row);
        let notice_block = match &shell.data_state {
            DataLoadState::Loading => Some(
                div()
                    .px(px(metrics.spacing.md))
                    .py(px(metrics.spacing.sm))
                    .text_sm()
                    .text_color(shell.colors.muted)
                    .border_1()
                    .border_color(shell.colors.border)
                    .rounded_sm()
                    .bg(shell.colors.panel_2)
                    .child("Loading workspace data..."),
            ),
            DataLoadState::Error(err) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(metrics.spacing.md))
                    .px(px(metrics.spacing.md))
                    .py(px(metrics.spacing.sm))
                    .text_sm()
                    .text_color(shell.colors.error)
                    .border_1()
                    .border_color(shell.colors.error)
                    .rounded_sm()
                    .bg(shell.colors.panel_2)
                    .child("Workspace data unavailable")
                    .child(format!("Error: {err}")),
            ),
            DataLoadState::Loaded => None,
        };

        let mut thread_stack = div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.gutter))
            .flex_1()
            .min_h(px(0.0));
        if let Some(notice_block) = notice_block {
            thread_stack = thread_stack.child(notice_block);
        }
        thread_stack = thread_stack.child(
            ThreadListView {
                shell,
                items: &shell.thread_items,
                list_state: &shell.thread_list_state,
                new_item_count: shell.new_thread_item_count,
            }
            .render(cx),
        );

        let composer = ComposerView {
            shell,
            variant: ComposerVariant::ActiveSession,
        }
        .render(cx);

        let center_column = div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.xxl))
            .flex_1()
            .min_h(px(0.0))
            .child(header_block)
            .child(thread_stack)
            .child(composer);

        let show_right_pane =
            shell.show_sessions_pane || shell.show_diff_pane || shell.show_artifacts_pane;
        let mut content_row = div()
            .flex()
            .flex_row()
            .gap(px(metrics.spacing.gutter))
            .flex_1()
            .min_h(px(0.0))
            .child(center_column);

        if show_right_pane {
            let right_pane_handle = div()
                .w(px(metrics.spacing.xl))
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .child(
                    div()
                        .w(px(2.0))
                        .h(px(metrics.spacing.gutter * 2.0))
                        .rounded_sm()
                        .bg(shell.colors.border),
                );
            let mut right_pane = div()
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.gutter))
                .w(px(360.0))
                .pl(px(metrics.spacing.sm))
                .min_h(px(0.0));

            if shell.show_sessions_pane {
                right_pane = right_pane.child(
                    div()
                        .on_children_prepainted(automation_tree::track_children_bounds(
                            "sessions-pane",
                            "pane",
                            Some("Sessions"),
                            Some("app-shell"),
                        ))
                        .id("sessions-pane")
                        .flex()
                        .flex_col()
                        .gap(px(metrics.spacing.xl))
                        .border_1()
                        .border_color(shell.colors.border)
                        .rounded_sm()
                        .bg(shell.colors.panel_2)
                        .p(px(metrics.spacing.xl))
                        .child(
                            SessionsPaneView {
                                colors: shell.colors,
                                sessions: &shell.sessions,
                                selected_session: shell.selected_session,
                                stream_status: &shell.stream_status,
                                resyncing: self.resyncing,
                            }
                            .render(cx)
                            .into_any_element(),
                        ),
                );
            }

            if shell.show_diff_pane {
                let diff_review_view = cx.update_entity(&shell.diff_review_state, |state, cx| {
                    DiffReviewView {
                        colors: shell.colors,
                        state,
                    }
                    .render(cx)
                    .into_any_element()
                });
                right_pane = right_pane.child(
                    div()
                        .on_children_prepainted(automation_tree::track_children_bounds(
                            "diff-pane",
                            "pane",
                            Some("Diff"),
                            Some("app-shell"),
                        ))
                        .id("diff-review-pane")
                        .flex()
                        .flex_col()
                        .gap(px(metrics.spacing.xl))
                        .flex_1()
                        .min_h(px(0.0))
                        .border_1()
                        .border_color(shell.colors.border)
                        .rounded_sm()
                        .bg(shell.colors.panel_2)
                        .p(px(metrics.spacing.xl))
                        .child(diff_review_view),
                );
            }

            if shell.show_artifacts_pane {
                right_pane = right_pane.child(
                    div()
                        .on_children_prepainted(automation_tree::track_children_bounds(
                            "artifacts-pane",
                            "pane",
                            Some("Artifacts"),
                            Some("app-shell"),
                        ))
                        .id("artifacts-pane")
                        .flex()
                        .flex_col()
                        .gap(px(metrics.spacing.xl))
                        .flex_1()
                        .min_h(px(0.0))
                        .border_1()
                        .border_color(shell.colors.border)
                        .rounded_sm()
                        .bg(shell.colors.panel_2)
                        .p(px(metrics.spacing.xl))
                        .child(
                            ArtifactsView {
                                colors: shell.colors,
                                artifacts: &shell.artifacts,
                                selected_artifact: shell.selected_artifact,
                                artifact_preview: &shell.artifact_preview,
                            }
                            .render(cx)
                            .into_any_element(),
                        ),
                );
            }

            content_row = content_row.child(right_pane_handle).child(right_pane);
        }

        let mut root = div()
            .id("session-view")
            .flex()
            .flex_col()
            .flex_1()
            .p(px(metrics.spacing.gutter))
            .bg(shell.colors.panel)
            .child(content_row);

        if shell.show_terminal_panel {
            root = root
                .child(
                    div()
                        .h(px(metrics.spacing.xl))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(metrics.spacing.gutter * 2.0))
                                .h(px(2.0))
                                .rounded_sm()
                                .bg(shell.colors.border),
                        ),
                )
                .child(
                    div()
                        .id("terminal-pane")
                        .border_t_1()
                        .border_color(shell.colors.border)
                        .pt(px(metrics.spacing.xl))
                        .child(shell.terminal_panel_state.clone()),
                );
        }

        root
    }
}
