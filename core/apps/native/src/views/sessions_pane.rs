use gpui::{Context, div, prelude::*, px, ClickEvent};

use crate::theme::ThemeColors;

use super::super::state::{ShellView, StreamStatus};
use super::super::workspace_summary::SessionSummaryItem;

pub(super) struct SessionsPaneView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) sessions: &'a [SessionSummaryItem],
    pub(super) selected_session: Option<usize>,
    pub(super) stream_status: &'a StreamStatus,
    pub(super) resyncing: bool,
}

impl<'a> SessionsPaneView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let list = if self.sessions.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No sessions available for this run.")
        } else {
            self.sessions
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_2(), |list, (index, session)| {
                    let is_selected = self.selected_session == Some(index);
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
                            .border_color(item_border)
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
                })
        };

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
        if self.resyncing {
            stream_block = stream_block.child("Resyncing session data...");
        }

        div()
            .id("sessions-pane-body")
            .flex()
            .flex_col()
            .gap_2()
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
            .child(div().h(px(6.0)))
            .child(list)
            .child(div().h(px(6.0)))
            .child(stream_block)
    }
}
