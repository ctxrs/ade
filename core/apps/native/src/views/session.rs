use gpui::{Context, FocusHandle, ListState, div, prelude::*, px, Entity};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Artifact, MessageAttachment, SessionEvent};

use crate::theme::ThemeColors;

use super::artifacts::ArtifactsView;
use super::composer::ComposerView;
use super::diff_review::DiffReviewView;
use super::messages::{EventsView, MessagesView};
use super::sessions_pane::SessionsPaneView;
use super::super::icons::{Icon, IconName};
use super::super::models::{MessageItem, SessionInfo};
use super::super::state::{
    ArtifactPreviewState, DataLoadState, DiffReviewState, ShellView, StreamStatus,
    TerminalPanelState, WorkspaceItem,
};
use super::super::workspace_summary::{SessionSummaryItem, TaskSummaryItem};

pub(super) struct SessionView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) workspaces: &'a [WorkspaceItem],
    pub(super) selected_workspace: Option<WorkspaceId>,
    pub(super) catchup_active_total: Option<i64>,
    pub(super) catchup_archived_total: Option<i64>,
    pub(super) tasks: &'a [TaskSummaryItem],
    pub(super) session: &'a SessionInfo,
    pub(super) sessions: &'a [SessionSummaryItem],
    pub(super) selected_session: Option<usize>,
    pub(super) messages: &'a [MessageItem],
    pub(super) message_list_state: &'a ListState,
    pub(super) new_message_count: usize,
    pub(super) session_events: &'a [SessionEvent],
    pub(super) artifacts: &'a [Artifact],
    pub(super) selected_artifact: Option<usize>,
    pub(super) artifact_preview: &'a ArtifactPreviewState,
    pub(super) data_state: &'a DataLoadState,
    pub(super) composer_text: &'a str,
    pub(super) composer_cursor: usize,
    pub(super) composer_attachment_text: &'a str,
    pub(super) composer_attachment_cursor: usize,
    pub(super) composer_attachments: &'a [MessageAttachment],
    pub(super) provider_options: Vec<String>,
    pub(super) model_options: Vec<String>,
    pub(super) selected_provider: Option<String>,
    pub(super) selected_model: Option<String>,
    pub(super) provider_menu_open: bool,
    pub(super) model_menu_open: bool,
    pub(super) composer_notice: Option<String>,
    pub(super) composer_attachment_focus: &'a FocusHandle,
    pub(super) composer_focus: &'a FocusHandle,
    pub(super) stream_status: &'a StreamStatus,
    pub(super) resyncing: bool,
    pub(super) show_sessions_pane: bool,
    pub(super) show_diff_pane: bool,
    pub(super) show_artifacts_pane: bool,
    pub(super) show_terminal_panel: bool,
    pub(super) diff_review_state: Entity<DiffReviewState>,
    pub(super) terminal_panel_state: Entity<TerminalPanelState>,
}

impl<'a> SessionView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let has_session = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .is_some();
        let can_send = has_session
            && (!self.composer_text.trim().is_empty() || !self.composer_attachments.is_empty());
        let interrupt_color = if has_session {
            self.colors.text
        } else {
            self.colors.muted
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
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child(interrupt_label);

        if has_session {
            interrupt_button = interrupt_button
                .bg(self.colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_interrupt_click));
        } else {
            interrupt_button = interrupt_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }

        let cancel_color = if has_session {
            self.colors.warning
        } else {
            self.colors.muted
        };
        let cancel_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::Cancel, 12.0, cancel_color))
            .child("Cancel");
        let mut cancel_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child(cancel_label);

        if has_session {
            cancel_button = cancel_button
                .bg(self.colors.panel)
                .text_color(self.colors.warning)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_cancel_click));
        } else {
            cancel_button = cancel_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }
        let pane_toggle = |label: &str, active: bool, on_click| {
            let mut button = div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .child(label);
            if active {
                button = button.bg(self.colors.panel).text_color(self.colors.text);
            } else {
                button = button.bg(self.colors.panel_2).text_color(self.colors.muted);
            }
            button
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(on_click)
        };

        let sessions_toggle = pane_toggle(
            "Sessions",
            self.show_sessions_pane,
            cx.listener(ShellView::toggle_sessions_pane),
        );
        let diff_toggle = pane_toggle(
            "Diff",
            self.show_diff_pane,
            cx.listener(ShellView::toggle_diff_pane),
        );
        let artifacts_toggle = pane_toggle(
            "Artifacts",
            self.show_artifacts_pane,
            cx.listener(ShellView::toggle_artifacts_pane),
        );
        let terminal_toggle = pane_toggle(
            "Terminal",
            self.show_terminal_panel,
            cx.listener(ShellView::toggle_terminal_panel),
        );

        let status_pill = div()
            .px_2()
            .py_0()
            .text_sm()
            .bg(self.colors.panel_2)
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child(self.session.status.as_str());

        let header_row = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_lg().child(self.session.title.as_str()))
                    .child(status_pill),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(sessions_toggle)
                    .child(diff_toggle)
                    .child(artifacts_toggle)
                    .child(terminal_toggle)
                    .child(interrupt_button)
                    .child(cancel_button),
            );

        let header_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(header_row)
            .child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(self.session.detail.as_str()),
            );

        let notice_block = match self.data_state {
            DataLoadState::Loading => Some(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .border_1()
                    .border_color(self.colors.border)
                    .rounded_sm()
                    .bg(self.colors.panel_2)
                    .child("Loading workspace data..."),
            ),
            DataLoadState::Error(err) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(self.colors.error)
                    .border_1()
                    .border_color(self.colors.error)
                    .rounded_sm()
                    .bg(self.colors.panel_2)
                    .child("Workspace data unavailable")
                    .child(format!("Error: {err}")),
            ),
            DataLoadState::Loaded => None,
        };

        let mut thread_stack = div().flex().flex_col().gap_3().flex_1();
        if let Some(notice_block) = notice_block {
            thread_stack = thread_stack.child(notice_block);
        }
        thread_stack = thread_stack
            .child(
                EventsView {
                    colors: self.colors,
                    events: self.session_events,
                }
                .render(),
            )
            .child(
                MessagesView {
                    colors: self.colors,
                    messages: self.messages,
                    message_list_state: self.message_list_state,
                    new_message_count: self.new_message_count,
                }
                .render(cx),
            );

        let composer = ComposerView {
            colors: self.colors,
            composer_text: self.composer_text,
            composer_cursor: self.composer_cursor,
            can_send,
            focus_handle: self.composer_focus,
            composer_attachment_text: self.composer_attachment_text,
            composer_attachment_cursor: self.composer_attachment_cursor,
            composer_attachments: self.composer_attachments,
            provider_options: &self.provider_options,
            model_options: &self.model_options,
            selected_provider: self.selected_provider.as_deref(),
            selected_model: self.selected_model.as_deref(),
            provider_menu_open: self.provider_menu_open,
            model_menu_open: self.model_menu_open,
            composer_notice: self.composer_notice.as_deref(),
            attachment_focus: self.composer_attachment_focus,
        }
        .render(cx);

        let center_column = div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .child(header_block)
            .child(thread_stack)
            .child(composer);

        let show_right_pane =
            self.show_sessions_pane || self.show_diff_pane || self.show_artifacts_pane;
        let mut content_row = div().flex().flex_row().gap_3().flex_1().child(center_column);

        if show_right_pane {
            let mut right_pane = div()
                .flex()
                .flex_col()
                .gap_3()
                .w(px(320.0))
                .pl_3()
                .border_l_1()
                .border_color(self.colors.border);

            if self.show_sessions_pane {
                right_pane = right_pane.child(
                    div()
                        .id("sessions-pane")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(self.colors.panel_2)
                        .p_2()
                        .child(
                            SessionsPaneView {
                                colors: self.colors,
                                sessions: self.sessions,
                                selected_session: self.selected_session,
                                stream_status: self.stream_status,
                                resyncing: self.resyncing,
                            }
                            .render(cx)
                            .into_any_element(),
                        ),
                );
            }

            if self.show_diff_pane {
                let diff_review_view = cx.update_entity(&self.diff_review_state, |state, cx| {
                    DiffReviewView {
                        colors: self.colors,
                        state,
                    }
                    .render(cx)
                    .into_any_element()
                });
                right_pane = right_pane.child(
                    div()
                        .id("diff-review-pane")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(self.colors.panel_2)
                        .p_2()
                        .child(diff_review_view),
                );
            }

            if self.show_artifacts_pane {
                right_pane = right_pane.child(
                    div()
                        .id("artifacts-pane")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(self.colors.panel_2)
                        .p_2()
                        .child(
                            ArtifactsView {
                                colors: self.colors,
                                artifacts: self.artifacts,
                                selected_artifact: self.selected_artifact,
                                artifact_preview: self.artifact_preview,
                            }
                            .render(cx)
                            .into_any_element(),
                        ),
                );
            }

            content_row = content_row.child(right_pane);
        }

        let mut root = div()
            .id("session-view")
            .flex()
            .flex_col()
            .flex_1()
            .p_4()
            .bg(self.colors.panel)
            .child(content_row);

        if self.show_terminal_panel {
            root = root
                .child(div().h(px(12.0)))
                .child(
                    div()
                        .id("terminal-pane")
                        .border_t_1()
                        .border_color(self.colors.border)
                        .pt_3()
                        .child(self.terminal_panel_state.clone()),
                );
        }

        root
    }
}
