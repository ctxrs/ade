use gpui::{Context, ListState, div, list, prelude::*, px};
use ctx_core::models::SessionEvent;

use crate::theme::ThemeColors;

use super::super::models::{session_event_type_label, MessageItem};
use super::super::state::ShellView;

pub(super) struct MessagesView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) messages: &'a [MessageItem],
    pub(super) message_list_state: &'a ListState,
    pub(super) new_message_count: usize,
}

impl<'a> MessagesView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let messages = self.messages.to_vec();
        let colors = self.colors;
        let list = list(self.message_list_state.clone(), move |index, _window, _cx| {
            let Some(message) = messages.get(index) else {
                return div().into_any_element();
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_2()
                .border_1()
                .border_color(colors.border)
                .rounded_sm()
                .bg(colors.panel)
                .text_sm()
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted)
                        .child(message.role.as_str()),
                )
                .child(message.content.as_str())
                .into_any_element()
        });

        let indicator = if self.new_message_count > 0 {
            let label = if self.new_message_count == 1 {
                "1 new message".to_string()
            } else {
                format!("{} new messages", self.new_message_count)
            };
            div()
                .px_2()
                .py_1()
                .text_sm()
                .border_1()
                .border_color(self.colors.border_strong)
                .rounded_sm()
                .bg(self.colors.panel)
                .text_color(self.colors.accent)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .child(label)
                .on_click(cx.listener(ShellView::on_new_messages_click))
        } else {
            div()
        };

        div()
            .id("message-list")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Messages")
                    .child(indicator),
            )
            .child(div().h(px(8.0)))
            .child(
                div()
                    .border_1()
                    .border_color(self.colors.border)
                    .rounded_sm()
                    .bg(self.colors.panel_2)
                    .p_2()
                    .child(list.h(px(220.0)).w_full()),
            )
    }
}

pub(super) struct EventsView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) events: &'a [SessionEvent],
}

impl<'a> EventsView<'a> {
    pub(super) fn render(&self) -> impl IntoElement {
        let list = if self.events.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No events yet.")
        } else {
            self.events
                .iter()
                .fold(div().flex().flex_col().gap_2(), |list, event| {
                    let left = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_color(self.colors.muted)
                                .child(format!("#{}", event.seq)),
                        )
                        .child(session_event_type_label(&event.event_type));
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
                            .bg(self.colors.panel_2)
                            .text_sm()
                            .child(left)
                            .child(
                                div()
                                    .text_color(self.colors.muted)
                                    .child(event.created_at.to_rfc3339()),
                            ),
                    )
                })
        };

        div()
            .id("events")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Events")
                    .child(format!("{}", self.events.len())),
            )
            .child(div().h(px(8.0)))
            .child(list)
    }
}
