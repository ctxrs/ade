use gpui::{Context, CursorStyle, FocusHandle, div, prelude::*, px};

use crate::theme::ThemeColors;

use super::super::icons::{Icon, IconName};
use super::super::state::ShellView;

pub(super) struct ComposerView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) composer_text: &'a str,
    pub(super) composer_cursor: usize,
    pub(super) can_send: bool,
    pub(super) focus_handle: &'a FocusHandle,
}

impl<'a> ComposerView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let placeholder_text = "Type a message...";
        let (input_text, is_placeholder) = if self.composer_text.is_empty() {
            (format!("|{placeholder_text}"), true)
        } else {
            let mut cursor = self.composer_cursor.min(self.composer_text.len());
            while cursor > 0 && !self.composer_text.is_char_boundary(cursor) {
                cursor -= 1;
            }
            let mut display = String::with_capacity(self.composer_text.len() + 1);
            display.push_str(&self.composer_text[..cursor]);
            display.push('|');
            display.push_str(&self.composer_text[cursor..]);
            (display, false)
        };
        let input_color = if is_placeholder {
            self.colors.muted
        } else {
            self.colors.text
        };

        let input = div()
            .flex_1()
            .text_sm()
            .text_color(input_color)
            .cursor(CursorStyle::IBeam)
            .track_focus(self.focus_handle)
            .on_click(cx.listener(ShellView::focus_composer))
            .on_key_down(cx.listener(ShellView::on_composer_key_down))
            .child(input_text);

        let send_color = if self.can_send {
            self.colors.text
        } else {
            self.colors.muted
        };
        let send_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::Send, 14.0, send_color))
            .child("Send");

        let mut send_button = div()
            .px_3()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .child(send_label);

        if self.can_send {
            send_button = send_button
                .bg(self.colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(ShellView::on_send_click));
        } else {
            send_button = send_button
                .bg(self.colors.panel_2)
                .text_color(self.colors.muted);
        }

        div()
            .id("composer")
            .flex()
            .flex_row()
            .items_end()
            .gap_2()
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .px_2()
            .py_2()
            .bg(self.colors.panel_2)
            .child(input)
            .child(send_button)
    }
}
