use gpui::{ClickEvent, Context, FontWeight, Rgba, StyleRefinement, div, prelude::*, px};
use gpui_component::{ElementExt, Icon as ComponentIcon, IconName as ComponentIconName};

use crate::{automation_tree, theme::ThemeMetrics};

use super::artifacts::ArtifactsView;
use super::composer::{ComposerVariant, ComposerView};
use super::diff_review::DiffReviewView;
use super::messages::ThreadListView;
use super::sessions_pane::SessionsPaneView;
use super::super::icons::{Icon, IconName};
use super::super::state::{ComposerMenuId, DataLoadState, RightPaneMode, ShellView};

pub(crate) struct SessionView<'a> {
    pub(super) shell: &'a ShellView,
    pub(super) resyncing: bool,
}

fn tint(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

impl<'a> SessionView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let shell = self.shell;
        let metrics = ThemeMetrics::default();
        let show_sessions_pane = matches!(shell.right_pane, Some(RightPaneMode::Sessions));
        let show_diff_pane = matches!(shell.right_pane, Some(RightPaneMode::Diff));
        let show_artifacts_pane = matches!(shell.right_pane, Some(RightPaneMode::Artifacts));
        if shell.new_task_mode {
            let composer = ComposerView {
                shell,
                variant: ComposerVariant::NewTask,
            }
            .render(cx);
            let composer_stack = div()
                .w_full()
                .max_w(px(825.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(composer);
            return div()
                .id("session-view")
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(div().w_full().flex().justify_center().child(composer_stack));
        }
        let has_session = shell
            .selected_session
            .and_then(|index| shell.sessions.get(index))
            .is_some();
        if !show_sessions_pane {
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
        if !show_diff_pane {
            automation_tree::register_hidden(
                "diff-pane",
                "pane",
                Some("Diff"),
                Some("app-shell"),
            );
        }
        if !show_artifacts_pane {
            automation_tree::register_hidden(
                "artifacts-pane",
                "pane",
                Some("Artifacts"),
                Some("app-shell"),
            );
        }
        /*
        let sessions_toggle = {
            let (bg, color) = if show_sessions_pane {
                (toggle_active_bg, toggle_active_color)
            } else {
                (toggle_idle_bg, toggle_idle_color)
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

        let toggle_active_bg = tint(shell.colors.accent, 0.22);
        let toggle_active_color = tint(shell.colors.text, 0.92);
        let toggle_idle_bg = tint(shell.colors.panel, 0.0);
        let toggle_idle_color = tint(shell.colors.text, 0.62);

        let diff_toggle = {
            let (bg, color) = if show_diff_pane {
                (toggle_active_bg, toggle_active_color)
            } else {
                (toggle_idle_bg, toggle_idle_color)
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
                .hover(|style| style.bg(tint(shell.colors.text, 0.06)))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_diff_pane))
        };

        let artifacts_toggle = {
            let (bg, color) = if show_artifacts_pane {
                (toggle_active_bg, toggle_active_color)
            } else {
                (toggle_idle_bg, toggle_idle_color)
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
                .hover(|style| style.bg(tint(shell.colors.text, 0.06)))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(ShellView::toggle_artifacts_pane))
        };

        let terminal_toggle = {
            let (bg, color) = if shell.show_terminal_panel {
                (toggle_active_bg, toggle_active_color)
            } else {
                (toggle_idle_bg, toggle_idle_color)
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
                .hover(|style| style.bg(tint(shell.colors.text, 0.06)))
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
                .text_sm()
                .text_color(shell.colors.muted)
                .child(short_id)
                .child(
                    ComponentIcon::new(ComponentIconName::Copy)
                        .size(px(12.0))
                        .text_color(shell.colors.muted),
                )
                .cursor_pointer()
                .id("session-worktree-chip")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.copy_text_with_key(copy_key.clone(), copy_value.clone(), cx);
                }))
        });

        let mut title_row = div()
            .flex()
            .items_center()
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

        let view = cx.entity();
        let mut overflow_toggle = div()
            .w(px(22.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .bg(toggle_idle_bg)
            .child(Icon::new(IconName::Ellipsis, 14.0, toggle_idle_color))
            .id("session-verbosity")
            .on_prepaint(move |bounds, window, cx| {
                view.update(cx, |view, cx| {
                    view.update_composer_menu_trigger_bounds(
                        ComposerMenuId::Verbosity,
                        bounds,
                        window,
                        cx,
                    );
                });
            });
        if has_session {
            overflow_toggle = overflow_toggle
                .cursor_pointer()
                .hover(|style: StyleRefinement| style.bg(tint(shell.colors.text, 0.06)))
                .active(|style: StyleRefinement| style.opacity(0.85))
                .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                    view.toggle_menu(ComposerMenuId::Verbosity, window, cx);
                }));
        } else {
            overflow_toggle = overflow_toggle.opacity(0.55);
        }

        let pane_toggle_row = div()
            .flex()
            .items_center()
            .gap(px(metrics.spacing.md))
            .child(artifacts_toggle)
            .child(diff_toggle)
            // .child(sessions_toggle)
            .child(terminal_toggle)
            .child(overflow_toggle);

        let control_row = div().flex().items_center().child(pane_toggle_row);
        let header_base_padding = metrics.spacing.xxl;
        let header_left_padding = header_base_padding + if shell.sidebar_collapsed { 28.0 } else { 0.0 };

        let header_block = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(metrics.spacing.xxl))
            .pl(px(header_left_padding))
            .pr(px(header_base_padding))
            .pt(px(14.0))
            .pb(px(metrics.spacing.xl))
            .border_b_1()
            .border_color(shell.colors.border)
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
            .gap(px(metrics.spacing.md))
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .child(header_block)
            .child(thread_stack)
            .child(composer);

        let show_right_pane =
            show_sessions_pane || show_diff_pane || show_artifacts_pane;
        let mut content_row = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .child(center_column);

        if show_right_pane {
            let splitter = div()
                .w(px(6.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .cursor_col_resize()
                .bg(Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                })
                .child(div().w(px(1.0)).h_full().bg(shell.colors.border));

            let mut right_pane = div()
                .flex()
                .flex_col()
                .w(px(480.0))
                .min_w(px(320.0))
                .min_h(px(0.0));

            if show_sessions_pane {
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
                        .gap(px(0.0))
                        .bg(Rgba {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.01,
                        })
                        .min_h(px(0.0))
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
            } else if show_diff_pane {
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
                        .id("diff-pane")
                        .flex()
                        .flex_col()
                        .gap(px(0.0))
                        .flex_1()
                        .min_h(px(0.0))
                        .bg(shell.colors.panel)
                        .child(diff_review_view),
                );
            } else if show_artifacts_pane {
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
                        .gap(px(0.0))
                        .flex_1()
                        .min_h(px(0.0))
                        .bg(shell.colors.panel)
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

            content_row = content_row.child(splitter).child(right_pane);
        }

        let root = div()
            .id("session-view")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .gap(px(metrics.spacing.md))
            .pb(px(metrics.spacing.xl))
            .bg(shell.colors.bg)
            .child(content_row);

        root
    }
}
