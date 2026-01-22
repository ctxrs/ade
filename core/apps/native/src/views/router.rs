use gpui::{
    ClickEvent, Context, ElementId, FontWeight, MouseButton, MouseDownEvent, Rgba, div, prelude::*,
    px, relative,
};
use gpui_component::ElementExt;
use gpui_component::input::Input;
use ctx_providers::adapters::ProviderHealth;

use crate::{automation_tree, theme::{ThemeColors, ThemeMetrics}};

use super::diagnostics::DiagnosticsPanelView;
use super::session::SessionView;
use super::sidebar::SidebarView;
use super::super::icons::{Icon, IconName};
use super::super::state::{DataLoadState, ShellRoute, ShellView, SidebarResizeState};

const SIDEBAR_TAB_SIZE: f32 = 28.0;
const SIDEBAR_TAB_TOP: f32 = 12.0;
const TERMINAL_RESIZER_HEIGHT: f32 = 6.0;
const TERMINAL_PANEL_HEIGHT: f32 = 260.0;

pub(crate) struct RouterView<'a> {
    pub(crate) shell: &'a ShellView,
    pub(crate) resyncing: bool,
}

impl<'a> RouterView<'a> {
    pub(crate) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let sidebar = if self.shell.route == ShellRoute::Workbench {
            SidebarView { shell: self.shell }.render(cx).into_any_element()
        } else {
            div().into_any_element()
        };

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
        let sidebar_collapse = if self.shell.route == ShellRoute::Workbench && !self.shell.sidebar_collapsed {
            self.render_sidebar_collapse_tab(cx).into_any_element()
        } else {
            div().into_any_element()
        };

        let workbench_row = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .relative()
            .child(sidebar)
            .child(main)
            .child(resizer)
            .child(sidebar_expand)
            .child(sidebar_collapse);

        let terminal_shell = if self.shell.route == ShellRoute::Workbench && self.shell.show_terminal_panel {
            self.render_terminal_shell(cx).into_any_element()
        } else {
            automation_tree::register_hidden(
                "terminal-panel",
                "pane",
                Some("Terminal"),
                Some("app-shell"),
            );
            div().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(workbench_row)
            .child(terminal_shell)
    }

    fn render_terminal_shell(&self, _cx: &mut Context<ShellView>) -> impl IntoElement {
        // Match web `terminalHeight=260` and `wb-terminal-resizer` (6px).
        div()
            .id("terminal-shell")
            .flex()
            .flex_col()
            .flex_none()
            .min_h(px(0.0))
            .child(
                div()
                    .id("terminal-resizer")
                    .h(px(TERMINAL_RESIZER_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_row_resize()
                    .bg(rgba(0, 0, 0, 0.0))
                    .child(div().w_full().h(px(1.0)).bg(self.shell.colors.border)),
            )
            .child(
                div()
                    .id("terminal-pane")
                    .h(px(TERMINAL_PANEL_HEIGHT))
                    .w_full()
                    .overflow_hidden()
                    .bg(self.shell.colors.panel)
                    .child(self.shell.terminal_panel_state.clone()),
            )
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
        let tab_bg = rgba(255, 255, 255, 0.03);
        let tab_top = SIDEBAR_TAB_TOP;
        div()
            .id("sidebar-expand")
            .on_prepaint(automation_tree::track_bounds(
                "sidebar-expand",
                "button",
                Some("Expand"),
                Some("app-shell"),
            ))
            .absolute()
            .top(px(tab_top))
            .left(px(0.0))
            .w(px(SIDEBAR_TAB_SIZE))
            .h(px(SIDEBAR_TAB_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_tr(px(8.0))
            .rounded_br(px(8.0))
            .rounded_tl(px(0.0))
            .rounded_bl(px(0.0))
            .border_1()
            .border_l_0()
            .border_color(self.shell.colors.border)
            .bg(tab_bg)
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_sidebar_collapsed(false, cx);
            }))
            .child(Icon::new(
                IconName::ChevronsRight,
                16.0,
                self.shell.colors.text,
            ))
    }

    fn render_sidebar_collapse_tab(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let tab_bg = rgba(255, 255, 255, 0.03);
        let tab_top = SIDEBAR_TAB_TOP;
        let left = (self.shell.sidebar_width - SIDEBAR_TAB_SIZE).max(0.0);
        div()
            .id("sidebar-collapse")
            .on_prepaint(automation_tree::track_bounds(
                "sidebar-collapse",
                "button",
                Some("Collapse"),
                Some("app-shell"),
            ))
            .absolute()
            .top(px(tab_top))
            .left(px(left))
            .w(px(SIDEBAR_TAB_SIZE))
            .h(px(SIDEBAR_TAB_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_tl(px(8.0))
            .rounded_bl(px(8.0))
            .rounded_tr(px(0.0))
            .rounded_br(px(0.0))
            .border_1()
            .border_r_0()
            .border_color(self.shell.colors.border)
            .bg(tab_bg)
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_sidebar_collapsed(true, cx);
            }))
            .child(Icon::new(
                IconName::ChevronsLeft,
                16.0,
                self.shell.colors.text,
            ))
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
            .bg(self.shell.colors.bg)
            .child(self.shell.settings_state.clone())
    }

    fn render_workspaces(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let colors = self.shell.colors;
        let metrics = ThemeMetrics::default();
        let ui_font = "DejaVu Sans".to_string();
        let blend_tint = |base: Rgba, tint: Rgba, alpha: f32| -> Rgba {
            base.blend(Rgba {
                r: tint.r,
                g: tint.g,
                b: tint.b,
                a: alpha,
            })
        };
        automation_tree::clear_prefix("workspace-item-");
        let max_width = 960.0;
        let banner_bg = blend_tint(colors.panel, colors.warning, 0.18);
        let banner_border = blend_tint(colors.border, colors.warning, 0.30);
        let list_divider = rgba(240, 240, 240, 1.0);
        let show_harness_warning = self
            .shell
            .providers
            .iter()
            .any(|provider| !matches!(provider.health, ProviderHealth::Ok));

        let mut warning_banner = div().into_any_element();
        if show_harness_warning {
            let link = div()
                .id("workspaces-harness-warning-link")
                .text_size(px(metrics.type_scale.md))
                .text_color(colors.accent)
                .font_family(ui_font.clone())
                .line_height(relative(1.4))
                .border_b_1()
                .border_color(colors.accent)
                .pb(px(1.0))
                    .child("Install or update harnesses.")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                        view.set_route(ShellRoute::Providers, cx);
                    }));
            warning_banner = div()
                .id("workspaces-harness-warning")
                .flex()
                .items_center()
                .gap(px(metrics.spacing.xs))
                .px(px(metrics.spacing.md))
                .py(px(7.0))
                .w_full()
                .border_1()
                .border_color(banner_border)
                .rounded(px(6.0))
                .bg(banner_bg)
                .child(
                    div()
                        .text_size(px(metrics.type_scale.md))
                        .font_family(ui_font.clone())
                        .line_height(relative(1.4))
                        .child("Some harnesses are not ready."),
                )
                .child(link)
                .into_any_element();
        }

        let header_row = div()
            .flex()
            .items_center()
            .w_full()
            .mt(px(14.0))
            .mb(px(3.0))
            .child(
                div()
                    .text_size(px(26.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text)
                    .font_family(ui_font.clone())
                    .child("Workspaces"),
            );

        let root_input = Input::new(&self.shell.workspace_root_input)
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .px(px(8.0))
            .py(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(colors.text)
            .w_full();
        let name_input = Input::new(&self.shell.workspace_name_input)
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .px(px(8.0))
            .py(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .text_size(px(13.0))
            .line_height(relative(1.4))
            .text_color(colors.text)
            .w_full();

        let add_button = div()
            .flex()
            .items_center()
            .justify_center()
            .px(px(12.0))
            .py(px(8.0))
            .border_1()
            .border_color(colors.border)
            .rounded(px(6.0))
            .bg(colors.panel_2)
            .text_size(px(metrics.type_scale.md))
            .font_family(ui_font.clone())
            .id("workspaces-add")
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                view.create_workspace(window, cx);
            }))
            .child("Add workspace");

        let root_label = div()
            .grid()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(metrics.type_scale.md))
                    .text_color(colors.text)
                    .font_family(ui_font.clone())
                    .line_height(relative(1.4))
                    .child("Root path"),
            )
            .child(root_input)
            .child(
                div()
                    .mt(px(6.0))
                    .text_size(px(metrics.type_scale.md))
                    .text_color(colors.muted)
                    .font_family(ui_font.clone())
                    .line_height(relative(1.4))
                    .child("Must be a git repo root (contains .git). Tilde (~) is supported."),
            );
        let name_label = div()
            .grid()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(metrics.type_scale.md))
                    .text_color(colors.text)
                    .font_family(ui_font.clone())
                    .line_height(relative(1.4))
                    .child("Name (optional)"),
            )
            .child(name_input);
        let form = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .w_full()
            .border_1()
            .border_color(colors.border)
            .rounded(px(8.0))
            .bg(colors.panel)
            .child(root_label)
            .child(name_label)
            .child(add_button);

        let list = if self.shell.workspaces.is_empty() {
            let empty = div()
                .text_size(px(metrics.type_scale.md))
                .text_color(colors.muted)
                .font_family(ui_font.clone())
                .line_height(relative(1.4))
                .child("No workspaces yet.");
            div()
                .on_children_prepainted(automation_tree::track_children_bounds(
                    "workspaces-list",
                    "list",
                    Some("Workspaces"),
                    Some("app-shell"),
                ))
                .child(empty)
        } else {
            self.shell
                .workspaces
                .iter()
                .enumerate()
                .fold(div().flex().flex_col(), |list, (index, workspace)| {
                    let on_click = cx.listener(move |view, _: &ClickEvent, _window, cx| {
                        view.select_workspace(index, cx);
                        view.set_route(ShellRoute::Workbench, cx);
                    });
                    let item_id = format!("workspace-item-{}", index);
                    let item_label = workspace.name.clone();
                    let item = div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .w_full()
                        .py(px(metrics.spacing.md))
                        .border_b_1()
                        .border_color(list_divider)
                        .cursor_pointer()
                        .id(ElementId::named_usize("workspace", index))
                        .on_click(on_click)
                        .child(
                            div()
                                .text_size(px(metrics.type_scale.md))
                                .text_color(colors.accent)
                                .font_family(ui_font.clone())
                                .line_height(relative(1.4))
                                .border_b_1()
                                .border_color(colors.accent)
                                .pb(px(1.0))
                                .flex_none()
                                .child(item_label.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(metrics.type_scale.md))
                                .text_color(colors.muted)
                                .font_family(ui_font.clone())
                                .line_height(relative(1.4))
                                .child(workspace.root_path.clone()),
                        );
                    let tracked = div()
                        .on_children_prepainted(automation_tree::track_children_bounds_dynamic(
                            item_id,
                            "button".to_string(),
                            Some(item_label),
                            Some("workspaces-list".to_string()),
                        ))
                        .w_full()
                        .child(item);
                    list.child(tracked)
                })
                .on_children_prepainted(automation_tree::track_children_bounds(
                    "workspaces-list",
                    "list",
                    Some("Workspaces"),
                    Some("app-shell"),
                ))
        }
        .w_full();

        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .max_w(px(max_width))
            .p(px(16.0))
            .font_family(metrics.type_scale.ui.clone())
            .child(header_row);
        if show_harness_warning {
            content = content.child(warning_banner);
        }
        content = content.child(form).child(list);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .bg(colors.bg)
            .child(content)
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
