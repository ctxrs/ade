use gpui::{
    ClickEvent, Context, ElementId, MouseButton, MouseDownEvent, Rgba, div, prelude::*, px,
};

use crate::{automation_tree, theme::{ThemeColors, ThemeMetrics}};

use super::diagnostics::DiagnosticsPanelView;
use super::session::SessionView;
use super::sidebar::SidebarView;
use super::super::icons::{Icon, IconName};
use super::super::state::{DataLoadState, ShellRoute, ShellView, SidebarResizeState};

pub(crate) struct RouterView<'a> {
    pub(crate) shell: &'a ShellView,
    pub(crate) resyncing: bool,
}

impl<'a> RouterView<'a> {
    pub(crate) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let sidebar = SidebarView { shell: self.shell }.render(cx).into_any_element();

        let main = match self.shell.route {
            ShellRoute::Workbench => self.render_workbench(cx).into_any_element(),
            ShellRoute::Settings => self.render_settings(cx).into_any_element(),
            ShellRoute::Providers => self.render_settings(cx).into_any_element(),
            ShellRoute::Diagnostics => self.render_diagnostics(cx).into_any_element(),
            ShellRoute::AppSettings => self.render_app_settings(cx).into_any_element(),
            ShellRoute::Workspaces => self.render_workspaces(cx).into_any_element(),
        };

        let resizer = if self.shell.route == ShellRoute::Workbench && !self.shell.sidebar_collapsed {
            self.render_sidebar_resizer(cx).into_any_element()
        } else {
            div().into_any_element()
        };
        let sidebar_expand = if self.shell.route == ShellRoute::Workbench && self.shell.sidebar_collapsed {
            self.render_sidebar_expand(cx).into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .flex()
            .flex_row()
            .flex_1()
            .relative()
            .child(sidebar)
            .child(main)
            .child(resizer)
            .child(sidebar_expand)
    }

    fn render_sidebar_resizer(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let left = (self.shell.sidebar_width - 3.0).max(0.0);
        let highlight = rgba(78, 163, 255, 0.35);
        let bar_color = if self.shell.sidebar_resizing || self.shell.sidebar_resizer_hovered {
            highlight
        } else {
            rgba(255, 255, 255, 0.0)
        };

        div()
            .id("sidebar-resizer")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(left))
            .w(px(6.0))
            .cursor_col_resize()
            .on_hover(cx.listener(|view, hovered, _window, cx| {
                view.sidebar_resizer_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|view, event: &MouseDownEvent, window, cx| {
                window.prevent_default();
                if event.click_count >= 2 {
                    view.set_sidebar_width(260.0, window, cx);
                    view.sidebar_resizing = false;
                    view.sidebar_resize_state = None;
                    cx.notify();
                    return;
                }
                view.sidebar_resizing = true;
                view.sidebar_resize_state = Some(SidebarResizeState {
                    start_x: f32::from(event.position.x),
                    start_width: view.sidebar_width,
                });
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(2.0))
                    .w(px(2.0))
                    .rounded_full()
                    .bg(bar_color),
            )
    }

    fn render_sidebar_expand(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let expand_button = div()
            .id("sidebar-expand")
            .absolute()
            .top(px(metrics.spacing.xl))
            .left_0()
            .w(px(24.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_color(self.shell.colors.border)
            .bg(self.shell.colors.panel_2)
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_sidebar_collapsed(false, cx);
            }))
            .child(Icon::new(
                IconName::ChevronRight,
                14.0,
                self.shell.colors.text,
            ));
        div()
            .on_children_prepainted(automation_tree::track_children_bounds(
                "sidebar-expand",
                "button",
                Some("Expand"),
                Some("app-shell"),
            ))
            .child(expand_button)
    }

    fn render_workbench(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        SessionView {
            shell: self.shell,
            resyncing: self.resyncing,
        }
        .render(cx)
    }

    fn render_settings(&self, _cx: &mut Context<ShellView>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .bg(self.shell.colors.panel)
            .child(self.shell.settings_state.clone())
    }

    fn render_workspaces(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let colors = self.shell.colors;
        let metrics = ThemeMetrics::default();
        automation_tree::clear_prefix("workspace-item-");
        let refresh_button = self
            .action_button(colors, "Refresh")
            .h(px(metrics.controls.h_sm))
            .flex()
            .items_center()
            .cursor_pointer()
            .id("workspaces-refresh")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.start_data_load(cx);
            }));

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_lg().child("Workspaces"))
            .child(refresh_button);

        let list = if self.shell.workspaces.is_empty() {
            div()
                .text_sm()
                .text_color(colors.muted)
                .child("No workspaces yet.")
        } else {
            self.shell
                .workspaces
                .iter()
                .enumerate()
                .fold(div().flex().flex_col().gap_2(), |list, (index, workspace)| {
                    let is_selected = self.shell.selected_workspace == Some(workspace.id);
                    let item_bg = if is_selected {
                        colors.panel
                    } else {
                        colors.panel_2
                    };
                    let item_border = if is_selected {
                        colors.border_strong
                    } else {
                        colors.border
                    };
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_workspace(index, cx);
                        view.set_route(ShellRoute::Workbench, cx);
                    });
                    let item_id = format!("workspace-item-{}", index);
                    let item_label = workspace.name.clone();
                    let item = div()
                        .flex()
                        .items_center()
                        .px_3()
                        .py_2()
                        .border_1()
                        .border_color(item_border)
                        .rounded_sm()
                        .bg(item_bg)
                        .text_sm()
                        .child(item_label.clone())
                        .cursor_pointer()
                        .id(ElementId::named_usize("workspace", index))
                        .on_click(on_click);
                    let tracked = div()
                        .on_children_prepainted(automation_tree::track_children_bounds_dynamic(
                            item_id,
                            "button".to_string(),
                            Some(item_label),
                            Some("workspaces-list".to_string()),
                        ))
                        .child(item);
                    list.child(tracked)
                })
                .on_children_prepainted(automation_tree::track_children_bounds(
                    "workspaces-list",
                    "list",
                    Some("Workspaces"),
                    Some("app-shell"),
                ))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_3()
            .p(px(metrics.spacing.gutter))
            .bg(colors.panel)
            .child(header)
            .child(list)
    }

    fn render_diagnostics(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let colors = self.shell.colors;
        let metrics = ThemeMetrics::default();
        let panel = DiagnosticsPanelView {
            colors,
            frame_time_ms: None,
            message_count: self.shell.messages.len(),
            event_count: self.shell.session_events.len(),
            memory_mb: None,
        }
        .render();

        let base_url = if self.shell.base_url == "unknown" {
            "Not connected".to_string()
        } else {
            format!("Connected to {}", self.shell.base_url)
        };
        let stream_label = self.shell.stream_status.label();
        let stream_detail = self.shell.stream_status.detail();
        let stream_text = stream_detail
            .map(|detail| format!("{stream_label} ({detail})"))
            .unwrap_or_else(|| stream_label.to_string());
        let data_state = match &self.shell.data_state {
            DataLoadState::Loading => "Loading",
            DataLoadState::Loaded => "Loaded",
            DataLoadState::Error(_) => "Error",
        };
        let selected_workspace = self
            .shell
            .selected_workspace
            .and_then(|id| self.shell.workspaces.iter().find(|ws| ws.id == id));
        let workspace_name = selected_workspace
            .map(|ws| ws.name.clone())
            .unwrap_or_else(|| "No workspace selected".to_string());
        let workspace_path = selected_workspace
            .map(|ws| ws.root_path.clone())
            .unwrap_or_else(|| "—".to_string());
        let tasks_active = self.shell.task_active_order.len();
        let tasks_archived = self.shell.task_archived_order.len();
        let sessions = self.shell.sessions.len();
        let artifacts = self.shell.artifacts.len();
        let providers = self.shell.providers.len();

        let refresh_button = self
            .action_button(colors, "Refresh data")
            .h(px(metrics.controls.h_sm))
            .flex()
            .items_center()
            .cursor_pointer()
            .id("diagnostics-refresh")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.start_data_load(cx);
            }));

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_lg().child("Diagnostics"))
            .child(refresh_button);

        let connection_card = self.section_card(
            colors,
            "Connection",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(colors.muted).child(base_url))
                .child(div().text_sm().child(format!("Stream: {stream_text}")))
                .child(div().text_sm().child(format!("Data: {data_state}"))),
        );

        let workspace_card = self.section_card(
            colors,
            "Workspace",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(colors.muted).child(workspace_name))
                .child(div().text_sm().text_color(colors.muted).child(workspace_path))
                .child(
                    div()
                        .text_sm()
                        .child(format!("Tasks: {tasks_active} active / {tasks_archived} archived")),
                )
                .child(div().text_sm().child(format!("Sessions: {sessions}")))
                .child(div().text_sm().child(format!("Artifacts: {artifacts}")))
                .child(div().text_sm().child(format!("Providers: {providers}"))),
        );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_3()
            .p(px(metrics.spacing.gutter))
            .bg(colors.panel)
            .child(header)
            .child(connection_card)
            .child(workspace_card)
            .child(panel)
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child("Full diagnostics and logs are available in the web UI."),
            )
    }

    fn render_app_settings(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let colors = self.shell.colors;
        let metrics = ThemeMetrics::default();
        let base_url = if self.shell.base_url == "unknown" {
            "Not connected".to_string()
        } else {
            format!("Connected to {}", self.shell.base_url)
        };

        let workspaces_button = self
            .action_button(colors, "Go to workspaces")
            .cursor_pointer()
            .id("app-settings-go-workspaces")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_route(ShellRoute::Workspaces, cx);
            }));

        let settings_button = self
            .action_button(colors, "Daemon settings")
            .cursor_pointer()
            .id("app-settings-daemon-settings")
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_route(ShellRoute::Settings, cx);
            }));

        let connection_card = self.section_card(
            colors,
            "Connection",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(colors.muted).child(base_url))
                .child(div().flex().items_center().gap_2().child(workspaces_button)),
        );

        let recents = self.shell.ui_state.recent_workspaces();
        let recents_count = recents.len();
        let recents_label = format!("{recents_count} saved entries");
        let mut clear_button = self
            .action_button(colors, "Clear recents")
            .id("app-settings-clear-recents");
        if recents_count > 0 {
            clear_button = clear_button
                .cursor_pointer()
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.ui_state.clear_recent_workspaces();
                    cx.notify();
                }));
        } else {
            clear_button = clear_button.opacity(0.6);
        }

        let recents_card = self.section_card(
            colors,
            "Recents",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted)
                        .child(recents_label),
                )
                .child(div().flex().items_center().gap_2().child(clear_button)),
        );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_3()
            .p(px(metrics.spacing.gutter))
            .bg(colors.panel)
            .child(div().text_lg().child("App Settings"))
            .child(connection_card)
            .child(recents_card)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child("Daemon-specific settings live in Settings."),
                    )
                    .child(div().flex().items_center().gap_2().child(settings_button)),
            )
    }

    fn section_card<E: IntoElement>(
        &self,
        colors: ThemeColors,
        title: &str,
        body: E,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .bg(colors.panel_2)
            .p_3()
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child(title.to_string()),
            )
            .child(body)
    }

    fn action_button(&self, colors: ThemeColors, label: &str) -> gpui::Div {
        div()
            .px_3()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(colors.border)
            .rounded_sm()
            .bg(colors.panel)
            .child(label.to_string())
    }
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}
