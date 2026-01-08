use gpui::{div, prelude::*, px, Window};

use ctx_core::models::TerminalStatus;

use super::super::state::terminal::{
    TerminalLoadState, TerminalPanelState, TerminalScope, TerminalStreamState,
};

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
        let scope_terminals = self.scope_terminals();
        let selected_terminal = self
            .selected_terminal_id
            .and_then(|id| scope_terminals.iter().find(|terminal| terminal.id == id))
            .copied();
        let task_scope_disabled = self.context.task_id.is_none();
        let can_create = self.context.workspace_id.is_some();
        let load_text = match &self.load_state {
            TerminalLoadState::Idle => "Idle".to_string(),
            TerminalLoadState::Loading => "Loading terminals...".to_string(),
            TerminalLoadState::Loaded => format!("Loaded {} terminals.", self.terminals.len()),
            TerminalLoadState::Error(message) => message.clone(),
        };
        let stream_text = match &self.stream_state {
            TerminalStreamState::Idle => "Stream: Idle".to_string(),
            TerminalStreamState::Connecting => "Stream: Connecting".to_string(),
            TerminalStreamState::Connected => "Stream: Connected".to_string(),
            TerminalStreamState::Reconnecting { .. } => "Stream: Reconnecting".to_string(),
            TerminalStreamState::Error(message) => format!("Stream error: {message}"),
        };
        let stream_detail = self
            .stream_url
            .as_ref()
            .map(|url| format!("Stream URL: {url}"));
        let stream_hint = self
            .stream_state
            .detail()
            .map(|detail| format!("Stream detail: {detail}"));

        let mut refresh_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Refresh");
        if can_create {
            refresh_button = refresh_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_refresh_click));
        } else {
            refresh_button = refresh_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let mut create_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("New");
        if can_create {
            create_button = create_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_create_terminal_click));
        } else {
            create_button = create_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let mut task_scope_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child(TerminalScope::Task.label());
        if task_scope_disabled {
            task_scope_button = task_scope_button.bg(colors.panel_2).text_color(colors.muted);
        } else if self.scope == TerminalScope::Task {
            task_scope_button = task_scope_button
                .bg(colors.panel)
                .text_color(colors.text)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_task_click));
        } else {
            task_scope_button = task_scope_button
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_task_click));
        }

        let mut workspace_scope_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child(TerminalScope::Workspace.label());
        if self.scope == TerminalScope::Workspace {
            workspace_scope_button = workspace_scope_button
                .bg(colors.panel)
                .text_color(colors.text)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_workspace_click));
        } else {
            workspace_scope_button = workspace_scope_button
                .bg(colors.panel_2)
                .text_color(colors.muted)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_workspace_click));
        }

        let list = if scope_terminals.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No terminals in this scope.")
        } else {
            scope_terminals.iter().fold(div().flex().flex_col().gap_1(), |list, terminal| {
                let is_selected = Some(terminal.id) == self.selected_terminal_id;
                let status = terminal_status_text(terminal);
                let on_click = cx.listener(move |view, _, _, cx| {
                    view.select_terminal(terminal.id, cx);
                });
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(colors.border)
                        .rounded_sm()
                        .bg(if is_selected { colors.panel_2 } else { colors.panel })
                        .cursor_pointer()
                        .active(|this| this.opacity(0.85))
                        .on_click(on_click)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(terminal.title.as_str())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted)
                                        .child(terminal.cwd.as_str()),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted)
                                .child(status),
                        ),
                )
            })
        };

        let mut delete_button = div()
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .child("Delete");
        if let Some(terminal) = selected_terminal {
            let terminal_id = terminal.id;
            delete_button = delete_button
                .bg(colors.panel)
                .cursor_pointer()
                .active(|this| this.opacity(0.85))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.delete_terminal(terminal_id, cx);
                }));
        } else {
            delete_button = delete_button
                .bg(colors.panel_2)
                .text_color(colors.muted);
        }

        let details = if let Some(terminal) = selected_terminal {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(format!("Title: {}", terminal.title)))
                .child(div().text_sm().child(format!("Shell: {}", terminal.shell)))
                .child(div().text_sm().child(format!("Cwd: {}", terminal.cwd)))
                .child(div().text_sm().child(format!("Status: {}", terminal_status_text(terminal))))
                .child(
                    div()
                        .text_sm()
                        .child(format!(
                            "Exit code: {}",
                            terminal
                                .exit_code
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "--".to_string())
                        )),
                )
        } else {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("Select a terminal to view details.")
        };

        div()
            .id("terminal-panel")
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .bg(colors.panel_2)
            .p_2()
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
                            .child("Terminals")
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors.muted)
                                    .child(format!("{}", scope_terminals.len())),
                            ),
                    )
                    .child(div().flex().items_center().gap_2().child(refresh_button).child(create_button)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(task_scope_button)
                    .child(workspace_scope_button),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child(load_text),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child(stream_text),
            )
            .child(if let Some(detail) = stream_detail {
                div().text_sm().text_color(colors.muted).child(detail)
            } else {
                div()
            })
            .child(if let Some(detail) = stream_hint {
                div().text_sm().text_color(colors.muted).child(detail)
            } else {
                div()
            })
            .child(
                if let Some(error) = &self.last_error {
                    div()
                        .text_sm()
                        .text_color(colors.error)
                        .child(error.as_str())
                } else {
                    div()
                },
            )
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child("Details"),
                    )
                    .child(delete_button),
            )
            .child(details)
            .child(div().h(px(8.0)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child("Output"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .border_1()
                            .border_color(colors.border)
                            .rounded_sm()
                            .bg(colors.panel)
                            .p_2()
                            .child(if self.stream_output.is_empty() {
                                "No output yet."
                            } else {
                                self.stream_output.as_str()
                            }),
                    ),
            )
    }
}
