use gpui::{ClickEvent, Context, FocusHandle, ListState, div, prelude::*, px};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Artifact, SessionEvent};

use crate::theme::ThemeColors;

use super::artifacts::ArtifactsView;
use super::composer::ComposerView;
use super::messages::{EventsView, MessagesView};
use super::super::icons::{Icon, IconName};
use super::super::models::{MessageItem, SessionInfo};
use super::super::state::{
    ArtifactPreviewState, DataLoadState, ShellView, StreamStatus, WorkspaceItem,
};
use super::super::workspace_summary::{SessionSummaryItem, TaskSummaryItem};

struct SessionListView<'a> {
    colors: ThemeColors,
    sessions: &'a [SessionSummaryItem],
    selected_session: Option<usize>,
}

impl<'a> SessionListView<'a> {
    fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let list = self
            .sessions
            .iter()
            .enumerate()
            .fold(div().flex().flex_col().gap_2(), |list, (index, session)| {
                let is_selected = self.selected_session == Some(index);
                let item_bg = if is_selected {
                    self.colors.panel_2
                } else {
                    self.colors.panel
                };
                let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.select_session(index, cx);
                });
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(self.colors.border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(session.title.as_str())
                        .child(
                            div()
                                .px_2()
                                .py_0()
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child(session.status.as_str()),
                        )
                        .on_click(on_click),
                )
            });

        div()
            .id("session-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Sessions")
                    .child(format!("{}", self.sessions.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}

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
    pub(super) composer_focus: &'a FocusHandle,
    pub(super) stream_status: &'a StreamStatus,
}

impl<'a> SessionView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let mut data_block = div().flex().flex_col().text_sm().text_color(self.colors.muted);
        match self.data_state {
            DataLoadState::Loading => {
                data_block = data_block.child("Loading workspace data...");
            }
            DataLoadState::Error(err) => {
                data_block = data_block.child("Workspace data unavailable");
                data_block = data_block.child(format!("Error: {err}"));
            }
            DataLoadState::Loaded => {
                data_block = data_block.child(format!("Workspaces: {}", self.workspaces.len()));
                if self.workspaces.is_empty() {
                    data_block = data_block.child("Workspace list: none");
                } else {
                    data_block = data_block.child("Workspace list:");
                    for workspace in self.workspaces {
                        data_block = data_block.child(format!("- {}", workspace.name));
                    }
                }
                let selected = self
                    .selected_workspace
                    .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
                    .map(|ws| ws.name.as_str())
                    .unwrap_or("None");
                data_block = data_block.child(format!("Selected workspace: {selected}"));
                if let Some(active_total) = self.catchup_active_total {
                    data_block = data_block.child(format!("Active tasks (total): {active_total}"));
                }
                if let Some(archived_total) = self.catchup_archived_total {
                    data_block = data_block.child(format!(
                        "Archived tasks (total): {archived_total}"
                    ));
                }
                if self.tasks.is_empty() {
                    data_block = data_block.child("Catchup tasks: none");
                } else {
                    data_block = data_block.child("Catchup tasks:");
                    for task in self.tasks {
                        data_block = data_block.child(format!(
                            "- {} ({})",
                            task.title,
                            task.status.label()
                        ));
                    }
                }
            }
        }
        let has_session = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .is_some();
        let can_send = has_session && !self.composer_text.trim().is_empty();
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
        let mut stream_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .text_sm()
            .text_color(self.colors.muted)
            .child(format!("Stream: {}", self.stream_status.label()));
        if let Some(detail) = self.stream_status.detail() {
            stream_block = stream_block.child(detail);
        }
        div()
            .id("session-view")
            .flex()
            .flex_col()
            .flex_1()
            .p_4()
            .bg(self.colors.panel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(self.colors.text)
                            .child(self.session.title.as_str()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .bg(self.colors.panel_2)
                                    .border_1()
                                    .border_color(self.colors.border)
                                    .rounded_sm()
                                    .child(self.session.status.as_str()),
                            )
                            .child(interrupt_button)
                            .child(cancel_button),
                    ),
            )
            .child(div().h(px(12.0)))
            .child(
                SessionListView {
                    colors: self.colors,
                    sessions: self.sessions,
                    selected_session: self.selected_session,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(self.colors.border)
                    .rounded_sm()
                    .p_3()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(self.session.detail.as_str())
                    .child(div().h(px(8.0)))
                    .child(stream_block),
            )
            .child(div().h(px(16.0)))
            .child(
                EventsView {
                    colors: self.colors,
                    events: self.session_events,
                }
                .render(),
            )
            .child(div().h(px(16.0)))
            .child(
                MessagesView {
                    colors: self.colors,
                    messages: self.messages,
                    message_list_state: self.message_list_state,
                    new_message_count: self.new_message_count,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(
                ArtifactsView {
                    colors: self.colors,
                    artifacts: self.artifacts,
                    selected_artifact: self.selected_artifact,
                    artifact_preview: self.artifact_preview,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(
                ComposerView {
                    colors: self.colors,
                    composer_text: self.composer_text,
                    composer_cursor: self.composer_cursor,
                    can_send,
                    focus_handle: self.composer_focus,
                }
                .render(cx),
            )
            .child(div().h(px(16.0)))
            .child(data_block)
    }
}
