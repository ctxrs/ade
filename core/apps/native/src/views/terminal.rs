use gpui::{CursorStyle, div, prelude::*, px, Window};
use gpui_component::scroll::ScrollableElement;

use ctx_core::models::TerminalStatus;

use crate::automation_tree;

use super::super::icons::{Icon, IconName};
use super::super::state::terminal::{
    TerminalLoadState, TerminalPanelState, TerminalScope, TerminalStreamState,
};

const MONO_FONT_FAMILY: &str =
    "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace";

fn terminal_status_text(terminal: &ctx_core::models::TerminalSession) -> String {
    match terminal.status {
        TerminalStatus::Running => "Running".to_string(),
        TerminalStatus::Exited => match terminal.exit_code {
            Some(code) => format!("Exited ({code})"),
            None => "Exited".to_string(),
        },
    }
}

impl Render for TerminalPanelState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let metrics = crate::theme::ThemeMetrics::default();
        let scope_terminals = self.scope_terminals();
        let selected_terminal = self
            .selected_terminal_id
            .and_then(|id| scope_terminals.iter().find(|terminal| terminal.id == id))
            .copied();
        let task_scope_disabled = self.context.task_id.is_none();
        let can_create = self.context.workspace_id.is_some();
        let can_reconnect = selected_terminal.is_some();
        let (load_message, load_color) = match &self.load_state {
            TerminalLoadState::Idle => (
                Some("Select a workspace to load terminals.".to_string()),
                colors.muted,
            ),
            TerminalLoadState::Loading => (Some("Loading terminals...".to_string()), colors.muted),
            TerminalLoadState::Loaded => (None, colors.muted),
            TerminalLoadState::Error(message) => (Some(message.clone()), colors.error),
        };
        let (stream_label, stream_color) = match &self.stream_state {
            TerminalStreamState::Idle => ("Stream: Idle".to_string(), colors.muted),
            TerminalStreamState::Connecting => ("Stream: Connecting".to_string(), colors.muted),
            TerminalStreamState::Connected => ("Stream: Connected".to_string(), colors.success),
            TerminalStreamState::Reconnecting { .. } => {
                ("Stream: Reconnecting".to_string(), colors.warning)
            }
            TerminalStreamState::Error(message) => (format!("Stream error: {message}"), colors.error),
        };
        let stream_detail = self
            .stream_state
            .detail()
            .map(|detail| format!("Detail: {detail}"));
        let terminal_status = selected_terminal.map(|terminal| {
            let status_text = terminal_status_text(terminal);
            let status_color = match terminal.status {
                TerminalStatus::Running => colors.success,
                TerminalStatus::Exited => colors.muted,
            };
            (status_text, status_color)
        });
        let status_pill = |label: String, color| {
            div()
                .px(px(metrics.spacing.md))
                .py(px(0.0))
                .text_sm()
                .border_1()
                .border_color(colors.border)
                .rounded_full()
                .bg(colors.panel)
                .text_color(color)
                .child(label)
        };

        let mut refresh_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Refresh")
            .id("terminal-refresh");
        if can_create {
            refresh_button = refresh_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_refresh_click));
        } else {
            refresh_button = refresh_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let mut create_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("New")
            .id("terminal-new");
        if can_create {
            create_button = create_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_create_terminal_click));
        } else {
            create_button = create_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let mut reconnect_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Reconnect")
            .id("terminal-reconnect");
        if can_reconnect {
            reconnect_button = reconnect_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_reconnect_click));
        } else {
            reconnect_button = reconnect_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let mut task_scope_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_full()
            .child(TerminalScope::Task.label())
            .id("terminal-scope-task");
        if task_scope_disabled {
            task_scope_button = task_scope_button.bg(colors.panel_2).text_color(colors.muted);
        } else if self.scope == TerminalScope::Task {
            task_scope_button = task_scope_button
                .bg(colors.panel)
                .text_color(colors.text)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_task_click));
        } else {
            task_scope_button = task_scope_button
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_task_click));
        }

        let mut workspace_scope_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_full()
            .child(TerminalScope::Workspace.label())
            .id("terminal-scope-workspace");
        if self.scope == TerminalScope::Workspace {
            workspace_scope_button = workspace_scope_button
                .bg(colors.panel)
                .text_color(colors.text)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_workspace_click));
        } else {
            workspace_scope_button = workspace_scope_button
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_workspace_click));
        }

        let list = if scope_terminals.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No terminals")
        } else {
            scope_terminals
                .iter()
                .fold(div().flex().flex_row().gap_1(), |list, terminal| {
                let is_selected = Some(terminal.id) == self.selected_terminal_id;
                let status_text = terminal_status_text(terminal);
                let status_color = match terminal.status {
                    TerminalStatus::Running => colors.success,
                    TerminalStatus::Exited => colors.muted,
                };
                let terminal_id = terminal.id;
                let terminal_key = format!("{terminal_id:?}");
                let select_click = cx.listener(move |view, _, _, cx| {
                    view.select_terminal(terminal_id, cx);
                });
                let delete_click = cx.listener(move |view, _, _, cx| {
                    view.delete_terminal(terminal_id, cx);
                });
                let indicator = div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(if is_selected { colors.accent } else { colors.border });
                let select_button = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .cursor_pointer()
                    .id(format!("terminal-select-{terminal_key}"))
                    .active(|style| style.opacity(0.85))
                    .on_click(select_click)
                    .child(indicator)
                    .child(div().text_sm().child(terminal.title.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(status_color)
                            .child(status_text),
                    );
                let delete_button = div()
                    .px(px(metrics.spacing.xs))
                    .py(px(0.0))
                    .text_sm()
                    .border_1()
                    .border_color(colors.border)
                    .rounded_sm()
                    .bg(colors.panel)
                    .text_color(colors.muted)
                    .cursor_pointer()
                    .id(format!("terminal-delete-{terminal_key}"))
                    .active(|style| style.opacity(0.85))
                    .on_click(delete_click)
                    .child("x");
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px(px(metrics.spacing.md))
                        .py(px(metrics.spacing.sm))
                        .border_1()
                        .border_color(if is_selected {
                            colors.border_strong
                        } else {
                            colors.border
                        })
                        .rounded_sm()
                        .bg(if is_selected { colors.panel } else { colors.panel_2 })
                        .child(select_button)
                        .child(delete_button),
                )
            })
        };

        let output_text = if selected_terminal.is_none() {
            "Select a terminal to view output.".to_string()
        } else if self.stream_output.is_empty() {
            "No output yet.".to_string()
        } else {
            self.stream_output.clone()
        };
        let output_is_empty = selected_terminal.is_none() || self.stream_output.is_empty();
        let output_color = if output_is_empty {
            colors.muted
        } else {
            colors.text
        };
        let can_clear_output = !self.stream_output.is_empty();
        let can_copy_output = !self.stream_output.trim().is_empty();
        let mut copy_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Copy")
            .id("terminal-copy");
        if can_copy_output {
            copy_button = copy_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_copy_output_click));
        } else {
            copy_button = copy_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }
        let mut clear_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Clear")
            .id("terminal-clear");
        if can_clear_output {
            clear_button = clear_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_clear_output_click));
        } else {
            clear_button = clear_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }
        let input_placeholder = if selected_terminal.is_some() {
            "Type input and press Enter..."
        } else {
            "Select a terminal to send input."
        };
        let (input_text, input_is_placeholder) = if self.input.text().is_empty() {
            (format!("|{input_placeholder}"), true)
        } else {
            let mut cursor = self.input.cursor().min(self.input.text().len());
            while cursor > 0 && !self.input.text().is_char_boundary(cursor) {
                cursor -= 1;
            }
            let mut display = String::with_capacity(self.input.text().len() + 1);
            display.push_str(&self.input.text()[..cursor]);
            display.push('|');
            display.push_str(&self.input.text()[cursor..]);
            (display, false)
        };
        let input_color = if input_is_placeholder {
            colors.muted
        } else {
            colors.text
        };
        let input_empty = self.input.text().is_empty();
        let can_send_input = selected_terminal.is_some() && !input_empty;
        let input_field = div()
            .flex_1()
            .text_sm()
            .font_family(MONO_FONT_FAMILY)
            .text_color(input_color)
            .whitespace_nowrap()
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.input_focus)
            .id("terminal-input")
            .on_click(cx.listener(TerminalPanelState::focus_input))
            .on_key_down(cx.listener(TerminalPanelState::on_input_key_down))
            .child(input_text);
        let send_icon_color = if can_send_input {
            colors.text
        } else {
            colors.muted
        };
        let send_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(IconName::Send, 12.0, send_icon_color))
            .child("Send");
        let mut send_button = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.sm))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_full()
            .child(send_label)
            .id("terminal-send");
        if can_send_input {
            send_button = send_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_send_input_click));
        } else {
            send_button = send_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let output_header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child("Output"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(copy_button)
                    .child(clear_button),
            );
        let output_view = div()
            .text_sm()
            .font_family(MONO_FONT_FAMILY)
            .text_color(output_color)
            .whitespace_nowrap()
            .overflow_scrollbar()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .bg(colors.panel)
            .p(px(metrics.spacing.md))
            .h(px(220.0))
            .child(output_text);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(metrics.spacing.xl))
            .py(px(metrics.spacing.lg))
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(Icon::new(IconName::Terminal, 14.0, colors.muted))
                    .child("Terminals")
                    .child(
                        div()
                            .px(px(metrics.spacing.md))
                            .py(px(0.0))
                            .text_sm()
                            .border_1()
                            .border_color(colors.border)
                            .rounded_full()
                            .bg(colors.panel)
                            .text_color(colors.text)
                            .child(format!("{}", scope_terminals.len())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(refresh_button)
                    .child(create_button),
            );

        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .bg(colors.panel_2)
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .p(px(metrics.spacing.xl))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(task_scope_button)
                    .child(workspace_scope_button),
            )
            .child(if let Some(message) = load_message {
                div().text_sm().text_color(load_color).child(message)
            } else {
                div()
            })
            .child(if let Some(error) = &self.last_error {
                div()
                    .text_sm()
                    .text_color(colors.error)
                    .child(error.clone())
            } else {
                div()
            })
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(status_pill(stream_label, stream_color))
                            .child(if let Some(detail) = stream_detail {
                                div().text_sm().text_color(colors.muted).child(detail)
                            } else {
                                div()
                            })
                            .child(if let Some((label, color)) = terminal_status {
                                status_pill(format!("Terminal: {label}"), color)
                            } else {
                                div()
                            }),
                    )
                    .child(reconnect_button),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(output_header)
                    .child(output_view),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child("Input"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .border_1()
                                    .border_color(colors.border)
                                    .rounded_sm()
                                    .bg(colors.panel)
                                    .p(px(metrics.spacing.md))
                                    .on_children_prepainted(automation_tree::track_children_bounds(
                                        "terminal-input",
                                        "textbox",
                                        Some("Terminal Input"),
                                        Some("terminal-panel"),
                                    ))
                                    .child(input_field),
                            )
                            .child(send_button),
                    ),
            );

        div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "terminal-panel",
                "pane",
                Some("Terminal"),
                Some("app-shell"),
            ))
            .id("terminal-panel")
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .child(body)
    }
}
