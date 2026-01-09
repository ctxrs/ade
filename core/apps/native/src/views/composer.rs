use gpui::{ClickEvent, Context, CursorStyle, ElementId, FocusHandle, div, prelude::*, px};

use ctx_core::models::MessageAttachment;

use crate::{
    automation_tree,
    theme::{ThemeColors, ThemeMetrics},
};

use super::super::icons::{Icon, IconName};
use super::super::state::ShellView;

fn attachment_label(att: &MessageAttachment) -> String {
    match att {
        MessageAttachment::Image { name, mime_type, .. }
        | MessageAttachment::ImageRef { name, mime_type, .. } => name
            .as_deref()
            .map(|value| value.to_string())
            .unwrap_or_else(|| mime_type.clone()),
    }
}

pub(super) struct ComposerView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) composer_text: &'a str,
    pub(super) composer_cursor: usize,
    pub(super) can_send: bool,
    pub(super) focus_handle: &'a FocusHandle,
    pub(super) composer_attachment_text: &'a str,
    pub(super) composer_attachment_cursor: usize,
    pub(super) composer_attachments: &'a [MessageAttachment],
    pub(super) provider_options: &'a [String],
    pub(super) model_options: &'a [String],
    pub(super) selected_provider: Option<&'a str>,
    pub(super) selected_model: Option<&'a str>,
    pub(super) provider_menu_open: bool,
    pub(super) model_menu_open: bool,
    pub(super) composer_notice: Option<&'a str>,
    pub(super) attachment_focus: &'a FocusHandle,
}

impl<'a> ComposerView<'a> {
    pub(super) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
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
            let mut c = self.colors.text;
            c.a = 0.35;
            c
        } else {
            self.colors.text
        };

        let input = div()
            .flex_1()
            .text_sm()
            .text_color(input_color)
            .cursor(CursorStyle::IBeam)
            .track_focus(self.focus_handle)
            .on_children_prepainted(automation_tree::track_children_bounds(
                "composer-input",
                "textbox",
                Some("Composer"),
                Some("app-shell"),
            ))
            .id("composer-input")
            .on_click(cx.listener(ShellView::focus_composer))
            .on_key_down(cx.listener(ShellView::on_composer_key_down))
            .child(input_text);
        // Send button: 24x24 circular with accent border and subtle accent tint, icon-only
        let send_button = {
            let mut send_bg = self.colors.accent;
            send_bg.a = 0.20; // subtle tint
            let send_icon_color = if self.can_send { self.colors.text } else { self.colors.muted };
            let base = div()
                .w(px(24.0))
                .h(px(24.0))
                .rounded_full()
                .border_1()
                .border_color(if self.can_send { self.colors.accent } else { self.colors.border })
                .bg(if self.can_send { send_bg } else { self.colors.panel_2 })
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Send, 14.0, send_icon_color))
                .id("composer-send");
            if self.can_send {
                base
                    .cursor_pointer()
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(ShellView::on_send_click))
            } else {
                base
            }
        };

        let attachment_placeholder = "Attachment path (image)";
        let (attachment_text, attachment_is_placeholder) =
            if self.composer_attachment_text.is_empty() {
                (format!("|{attachment_placeholder}"), true)
            } else {
                let mut cursor = self
                    .composer_attachment_cursor
                    .min(self.composer_attachment_text.len());
                while cursor > 0 && !self.composer_attachment_text.is_char_boundary(cursor) {
                    cursor -= 1;
                }
                let mut display =
                    String::with_capacity(self.composer_attachment_text.len() + 1);
                display.push_str(&self.composer_attachment_text[..cursor]);
                display.push('|');
                display.push_str(&self.composer_attachment_text[cursor..]);
                (display, false)
            };
        let attachment_color = if attachment_is_placeholder {
            let mut c = self.colors.text;
            c.a = 0.35;
            c
        } else {
            self.colors.text
        };
        let attachment_input = div()
            .flex_1()
            .text_sm()
            .text_color(attachment_color)
            .cursor(CursorStyle::IBeam)
            .track_focus(self.attachment_focus)
            .id("composer-attachment-input")
            .on_click(cx.listener(ShellView::focus_composer_attachment))
            .on_key_down(cx.listener(ShellView::on_composer_attachment_key_down))
            .child(attachment_text);

        let mut add_attachment_button = div()
            .w(px(metrics.controls.h_sm))
            .h(px(metrics.controls.h_sm))
            .rounded_sm()
            .text_sm()
            .flex()
            .items_center()
            .justify_center()
            .text_color(self.colors.muted)
            .child(Icon::new(IconName::Image, 14.0, self.colors.muted))
            .id("composer-add-attachment");
        add_attachment_button = add_attachment_button
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(ShellView::on_add_attachment_click));

        let provider_label = self.selected_provider.unwrap_or("Provider");
        let model_label = self.selected_model.unwrap_or("Model");

        let provider_menu = if self.provider_menu_open {
            let list = if self.provider_options.is_empty() {
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("No providers")
            } else {
                self.provider_options.iter().enumerate().fold(
                    div().flex().flex_col().gap(px(metrics.spacing.md)),
                    |list, (index, provider)| {
                        let is_selected = self.selected_provider == Some(provider.as_str());
                        let provider_id = provider.clone();
                        let on_click =
                            cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                view.select_composer_provider(provider_id.clone(), cx);
                            });
                        list.child(
                            div()
                                .px(px(metrics.spacing.md))
                                .py(px(metrics.spacing.sm))
                                .border_1()
                                .border_color(self.colors.border)
                                .rounded_sm()
                                .bg(if is_selected {
                                    self.colors.panel
                                } else {
                                    self.colors.panel_2
                                })
                                .text_sm()
                                .child(provider.clone())
                                .cursor_pointer()
                                .id(ElementId::named_usize("composer-provider", index))
                                .on_click(on_click),
                        )
                    },
                )
            };
            div()
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.md))
                .p(px(metrics.spacing.sm))
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .bg(self.colors.panel_2)
                .child(list)
        } else {
            div()
        };

        let model_menu = if self.model_menu_open {
            let list = if self.model_options.is_empty() {
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("No models")
            } else {
                self.model_options.iter().enumerate().fold(
                    div().flex().flex_col().gap(px(metrics.spacing.md)),
                    |list, (index, model)| {
                        let is_selected = self.selected_model == Some(model.as_str());
                        let model_id = model.clone();
                        let on_click =
                            cx.listener(move |view, _: &ClickEvent, _window, cx| {
                                view.select_composer_model(model_id.clone(), cx);
                            });
                        list.child(
                            div()
                                .px(px(metrics.spacing.md))
                                .py(px(metrics.spacing.sm))
                                .border_1()
                                .border_color(self.colors.border)
                                .rounded_sm()
                                .bg(if is_selected {
                                    self.colors.panel
                                } else {
                                    self.colors.panel_2
                                })
                                .text_sm()
                                .child(model.clone())
                                .cursor_pointer()
                                .id(ElementId::named_usize("composer-model", index))
                                .on_click(on_click),
                        )
                    },
                )
            };
            div()
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.md))
                .p(px(metrics.spacing.sm))
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .bg(self.colors.panel_2)
                .child(list)
        } else {
            div()
        };

        let provider_control = div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.md))
            .child(
                div()
                    .h(px(metrics.controls.h_sm))
                    .px(px(6.0))
                    .text_sm()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap(px(metrics.spacing.sm))
                    .text_color(self.colors.muted)
                    .child(format!("{provider_label} ▾"))
                    .cursor_pointer()
                    .id("composer-provider-toggle")
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(ShellView::toggle_composer_provider_menu))
            )
            .child(provider_menu);

        let model_control = div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.md))
            .child(
                div()
                    .h(px(metrics.controls.h_sm))
                    .px(px(6.0))
                    .text_sm()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap(px(metrics.spacing.sm))
                    .text_color(self.colors.muted)
                    .child(format!("{model_label} ▾"))
                    .cursor_pointer()
                    .id("composer-model-toggle")
                    .active(|style| style.opacity(0.85))
                    .on_click(cx.listener(ShellView::toggle_composer_model_menu))
            )
            .child(model_menu);

        let mut attachments_block = div().flex().flex_col().gap(px(metrics.spacing.md));
        if let Some(notice) = self.composer_notice {
            let notice = notice.to_string();
            attachments_block = attachments_block.child(
                div()
                    .text_sm()
                    .text_color(self.colors.warning)
                    .child(notice.to_string()),
            );
        }
        if !self.composer_attachments.is_empty() {
            // Render attachments as compact pill chips with inline remove
            let list = self
                .composer_attachments
                .iter()
                .enumerate()
                .fold(div().flex().gap(px(metrics.spacing.sm)).flex_wrap(), |list, (index, attachment)| {
                    let label = attachment_label(attachment);
                    let on_remove = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.remove_composer_attachment(index, cx);
                    });
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(metrics.spacing.sm))
                            .px(px(metrics.spacing.md))
                            .py(px(metrics.spacing.xs))
                            .rounded_full()
                            .border_1()
                            .border_color(self.colors.border)
                            .bg(self.colors.panel_2)
                            .text_sm()
                            .text_color(self.colors.muted)
                            .child(label)
                            .child(
                                div()
                                    .px(px(metrics.spacing.xs))
                                    .py(px(0.0))
                                    .text_sm()
                                    .text_color(self.colors.muted)
                                    .child("×")
                                    .cursor_pointer()
                                    .id(ElementId::named_usize("composer-attachment-remove", index))
                                    .on_click(on_remove),
                            ),
                    )
                });
            attachments_block = attachments_block.child(list);
        }
        attachments_block = attachments_block.child(
            div()
                .flex()
                .items_center()
                .gap(px(metrics.spacing.md))
                .child(attachment_input)
                .child(add_attachment_button),
        );

        let composer_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(metrics.spacing.xl))
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .px(px(metrics.spacing.xl))
            .py(px(metrics.spacing.lg))
            .bg(self.colors.panel_2)
            .child(input)
            .child(send_button);

        div()
            .id("composer")
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.lg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(metrics.spacing.md))
                    .child(provider_control)
                    .child(model_control),
            )
            .child(attachments_block)
            .child(composer_row)
    }
}
