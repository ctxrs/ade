use chrono::{DateTime, Utc};
use gpui::{
    div, prelude::*, px, AnyElement, ClickEvent, CursorStyle, MouseButton, Rgba, StyledText, Window,
};
use gpui_component::ElementExt;

use crate::automation_tree;
use super::super::relative_time::format_relative_age_short;

use super::super::icons::{Icon, IconName};
use super::super::state::terminal::{
    terminal_line_height, TerminalLoadState, TerminalPanelState, TerminalScope,
    TerminalStreamState, MONO_FONT_FAMILY, TERMINAL_FONT_SIZE,
};

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

fn parse_port_seen_at(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

impl Render for TerminalPanelState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let metrics = crate::theme::ThemeMetrics::default();
        let scope_terminals = self.scope_terminals();
        let scope_ports = self.scope_ports();
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
        let active_stream_state = self.active_stream_state();
        let stream_detail = active_stream_state.detail().map(ToOwned::to_owned);
        let stream_error = match &active_stream_state {
            TerminalStreamState::Error(message) => Some(message.clone()),
            _ => None,
        };

        let action_hover_bg = rgba(255, 255, 255, 0.06);
        let mut refresh_button = div()
            .w(px(22.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .id("terminal-refresh")
            .bg(rgba(0, 0, 0, 0.0))
            .child(Icon::new(IconName::ChevronDown, 14.0, colors.muted));
        if can_create {
            refresh_button = refresh_button
                .cursor_pointer()
                .hover(move |style| style.bg(action_hover_bg))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_refresh_click));
        } else {
            refresh_button = refresh_button.opacity(0.5);
        }

        let mut create_button = div()
            .w(px(22.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .id("terminal-new")
            .bg(rgba(0, 0, 0, 0.0))
            .child(Icon::new(IconName::LayersPlus, 14.0, colors.muted));
        if can_create {
            create_button = create_button
                .cursor_pointer()
                .hover(move |style| style.bg(action_hover_bg))
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_create_terminal_click));
        } else {
            create_button = create_button.opacity(0.5);
        }

        let reconnect_visible = matches!(
            active_stream_state,
            TerminalStreamState::Error(_)
                | TerminalStreamState::Idle
                | TerminalStreamState::Reconnecting { .. }
        );
        let mut reconnect_button = div()
            .px(px(metrics.spacing.sm))
            .py(px(metrics.spacing.xs))
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
            reconnect_button = reconnect_button.bg(colors.panel_2).text_color(colors.muted);
        }

        let scope_container_bg = rgba(255, 255, 255, 0.04);
        let scope_active_bg = rgba(255, 255, 255, 0.08);
        let mut task_scope_button = div()
            .text_size(px(11.0))
            .px(px(8.0))
            .py(px(2.0))
            .text_color(if self.scope == TerminalScope::Task {
                colors.text
            } else {
                colors.muted
            })
            .bg(if self.scope == TerminalScope::Task {
                scope_active_bg
            } else {
                rgba(0, 0, 0, 0.0)
            })
            .child(TerminalScope::Task.label())
            .id("terminal-scope-task");
        if task_scope_disabled {
            task_scope_button = task_scope_button.opacity(0.5);
        } else {
            task_scope_button = task_scope_button
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_task_click));
        }

        let mut workspace_scope_button = div()
            .text_size(px(11.0))
            .px(px(8.0))
            .py(px(2.0))
            .text_color(if self.scope == TerminalScope::Workspace {
                colors.text
            } else {
                colors.muted
            })
            .bg(if self.scope == TerminalScope::Workspace {
                scope_active_bg
            } else {
                rgba(0, 0, 0, 0.0)
            })
            .child(TerminalScope::Workspace.label())
            .id("terminal-scope-workspace");
        if self.context.workspace_id.is_none() {
            workspace_scope_button = workspace_scope_button.opacity(0.5);
        } else {
            workspace_scope_button = workspace_scope_button
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_scope_workspace_click));
        }

        let list = if scope_terminals.is_empty() {
            div().flex_1().min_h(px(0.0))
        } else {
            scope_terminals
                .iter()
                .fold(div().flex().flex_col(), |list, terminal| {
                    let is_selected = Some(terminal.id) == self.selected_terminal_id;
                    let terminal_id = terminal.id;
                    let terminal_key = terminal_id.0.to_string();
                    let select_click = cx.listener(move |view, _, _, cx| {
                        view.select_terminal(terminal_id, cx);
                    });
                    let delete_click = cx.listener(move |view, _, _, cx| {
                        view.delete_terminal(terminal_id, cx);
                    });
                    let accent_bar = div().w(px(2.0)).h_full().bg(if is_selected {
                        colors.accent
                    } else {
                        colors.panel_2
                    });
                    let select_button = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .cursor_pointer()
                        .id(format!("terminal-select-{terminal_key}"))
                        .active(|style| style.opacity(0.85))
                        .on_click(select_click)
                        .child(Icon::new(IconName::Terminal, 12.0, colors.muted))
                        .child(div().text_sm().child(terminal.title.clone()));
                    let delete_button = div()
                        .w(px(18.0))
                        .h(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .id(format!("terminal-delete-{terminal_key}"))
                        .active(|style| style.opacity(0.85))
                        .on_click(delete_click)
                        .child(Icon::new(IconName::Cancel, 12.0, colors.muted));
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(30.0))
                            .px(px(metrics.spacing.sm))
                            .bg(if is_selected {
                                colors.panel
                            } else {
                                colors.panel_2
                            })
                            .border_b_1()
                            .border_color(colors.border)
                            .child(accent_bar)
                            .child(select_button)
                            .child(delete_button),
                    )
                })
        };

        let has_terminals = !scope_terminals.is_empty();
        let auto_forward_label = if self.auto_forward_enabled { "On" } else { "Off" };
        let auto_forward_disabled = !self.auto_forward_loaded || self.auto_forward_busy;
        let mut auto_forward_toggle = div()
            .px(px(metrics.spacing.md))
            .py(px(metrics.spacing.xs))
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_full()
            .child(format!("Auto-forward {auto_forward_label}"))
            .id("terminal-preview-auto-forward");
        if auto_forward_disabled {
            auto_forward_toggle = auto_forward_toggle
                .bg(colors.panel_2)
                .text_color(colors.muted);
        } else {
            auto_forward_toggle = auto_forward_toggle
                .bg(colors.panel)
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(TerminalPanelState::on_toggle_auto_forward));
        }

        let preview_header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child("Previews"),
            )
            .child(auto_forward_toggle);

        let preview_list: AnyElement = if self.ports_loading && scope_ports.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("Loading previews...")
                .into_any_element()
        } else if scope_ports.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No previews yet")
                .into_any_element()
        } else {
            scope_ports.iter().fold(div().flex().flex_col().gap_1(), |list, entry| {
                let last_seen = format_relative_age_short(
                    parse_port_seen_at(&entry.last_seen_at),
                    Utc::now(),
                );
                let port_id = entry.id.clone();
                let open_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                    view.open_port_preview(port_id.clone(), cx);
                });
                let open_button = div()
                    .px(px(metrics.spacing.sm))
                    .py(px(metrics.spacing.xs))
                    .text_sm()
                    .border_1()
                    .border_color(colors.border)
                    .rounded_sm()
                    .bg(colors.panel)
                    .text_color(colors.text)
                    .cursor_pointer()
                    .id(format!("terminal-preview-open-{}", entry.id))
                    .on_click(open_click)
                    .child("Open");
                let last_seen = if last_seen.is_empty() {
                    "Now".to_string()
                } else {
                    last_seen
                };
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px(px(metrics.spacing.md))
                        .py(px(metrics.spacing.sm))
                        .border_1()
                        .border_color(colors.border)
                        .rounded_sm()
                        .bg(colors.panel)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.text)
                                        .child(format!("{}:{}", entry.host, entry.port)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted)
                                        .child(last_seen),
                                ),
                        )
                        .child(open_button),
                )
            })
            .into_any_element()
        };

        let preview_block = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(preview_header)
            .child(preview_list)
            .child(if let Some(err) = &self.port_error {
                div()
                    .text_sm()
                    .text_color(colors.error)
                    .child(format!("Preview error: {err}"))
            } else {
                div()
            })
            .child(if let Some(err) = &self.auto_forward_error {
                div()
                    .text_sm()
                    .text_color(colors.error)
                    .child(format!("Auto-forward unavailable: {err}"))
            } else {
                div()
            });

        let output_text = if !has_terminals {
            "No terminals yet.".to_string()
        } else if selected_terminal.is_none() {
            "Select a terminal to view output.".to_string()
        } else {
            "No output yet.".to_string()
        };
        let output_is_empty = !has_terminals || selected_terminal.is_none();
        let rendered_output = self.active_stream_rendered();
        let output_color = if output_is_empty || rendered_output.is_none() {
            colors.muted
        } else {
            colors.text
        };
        let output_body: AnyElement = if output_is_empty {
            div().child(output_text.clone()).into_any_element()
        } else if let Some(rendered) = rendered_output {
            StyledText::new(rendered.text.clone())
                .with_runs(rendered.runs.clone())
                .into_any_element()
        } else {
            div().child(output_text.clone()).into_any_element()
        };
        let output_padding = px(8.0);
        let output_line_height = terminal_line_height(window);
        let output_view = div()
            .text_sm()
            .text_size(px(TERMINAL_FONT_SIZE))
            .line_height(output_line_height)
            .font_family(MONO_FONT_FAMILY)
            .text_color(output_color)
            .whitespace_nowrap()
            .overflow_hidden()
            .bg(colors.panel)
            .px(px(8.0))
            .py(px(6.0))
            .flex_1()
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.input_focus)
            .id("terminal-input")
            .on_click(cx.listener(TerminalPanelState::focus_input))
            .on_key_down(cx.listener(TerminalPanelState::on_input_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(TerminalPanelState::on_output_mouse_down),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(TerminalPanelState::on_output_mouse_down),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(TerminalPanelState::on_output_mouse_down),
            )
            .on_mouse_move(cx.listener(TerminalPanelState::on_output_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(TerminalPanelState::on_output_mouse_up),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(TerminalPanelState::on_output_mouse_up),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(TerminalPanelState::on_output_mouse_up),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(TerminalPanelState::on_output_mouse_up_out),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(TerminalPanelState::on_output_mouse_up_out),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(TerminalPanelState::on_output_mouse_up_out),
            )
            .on_scroll_wheel(cx.listener(TerminalPanelState::on_output_scroll_wheel))
            .on_prepaint({
                let view = cx.entity();
                move |bounds, window, cx| {
                    view.update(cx, |view, cx| {
                        view.update_terminal_output_bounds(bounds, output_padding, window, cx);
                    });
                }
            })
            .child(output_body);

        let sidebar = div()
            .w(px(120.0))
            .flex_none()
            .flex()
            .flex_col()
            .bg(colors.panel)
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_start()
                    .px(px(6.0))
                    .pt(px(4.0))
                    .pb(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(create_button)
                            .child(refresh_button),
                    )
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .mx(px(6.0))
                    .mt(px(6.0))
                    .mb(px(4.0))
                    .rounded_full()
                    .border_1()
                    .border_color(colors.border)
                    .overflow_hidden()
                    .bg(scope_container_bg)
                    .child(task_scope_button)
                    .child(workspace_scope_button),
            )
            .child(if let Some(message) = load_message {
                div()
                    .px(px(metrics.spacing.sm))
                    .pb(px(metrics.spacing.sm))
                    .text_sm()
                    .text_color(load_color)
                    .child(message)
            } else {
                div()
            })
            .child(if let Some(error) = &self.last_error {
                div()
                    .px(px(metrics.spacing.sm))
                    .pb(px(metrics.spacing.sm))
                    .text_sm()
                    .text_color(colors.error)
                    .child(error.clone())
            } else {
                div()
            })
            .child(div().flex_1().overflow_hidden().child(list))
            .child(preview_block)
            .child(if reconnect_visible {
                div()
                    .px(px(metrics.spacing.sm))
                    .py(px(metrics.spacing.sm))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(reconnect_button)
                            .child(if let Some(detail) = stream_detail.clone().or(stream_error.clone()) {
                                div()
                                    .text_sm()
                                    .text_color(colors.muted)
                                    .child(detail)
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
            } else {
                div()
            });

        let viewport = div()
            .flex_1()
            .flex()
            .flex_col()
            .bg(colors.panel)
            .child(output_view);

        div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "terminal-panel",
                "pane",
                Some("Terminal"),
                Some("app-shell"),
            ))
            .id("terminal-panel")
            .flex()
            .flex_row()
            .bg(colors.panel)
            .child(sidebar)
            .child(viewport)
    }
}
