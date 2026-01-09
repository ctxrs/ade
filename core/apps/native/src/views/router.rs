use gpui::{ClickEvent, Context, ElementId, div, prelude::*, px};

use crate::theme::{ThemeColors, ThemeMetrics};

use super::diagnostics::DiagnosticsPanelView;
use super::session::SessionView;
use super::sidebar::SidebarView;
use super::super::state::{ShellRoute, ShellView};

pub(crate) struct RouterView<'a> {
    pub(crate) shell: &'a ShellView,
    pub(crate) provider_options: Vec<String>,
    pub(crate) model_options: Vec<String>,
    pub(crate) resyncing: bool,
}

impl<'a> RouterView<'a> {
    pub(crate) fn render(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        let sidebar = SidebarView {
            colors: self.shell.colors,
            current_route: self.shell.route,
            workspaces: &self.shell.workspaces,
            selected_workspace: self.shell.selected_workspace,
            providers: &self.shell.providers,
            tasks: &self.shell.tasks,
            selected_task: self.shell.selected_task,
        }
        .render(cx)
        .into_any_element();

        let main = match self.shell.route {
            ShellRoute::Workbench => self.render_workbench(cx).into_any_element(),
            ShellRoute::Settings => self.render_settings(cx).into_any_element(),
            ShellRoute::Providers => self.render_settings(cx).into_any_element(),
            ShellRoute::Diagnostics => self.render_diagnostics(cx).into_any_element(),
            ShellRoute::AppSettings => self.render_app_settings(cx).into_any_element(),
            ShellRoute::Workspaces => self.render_workspaces(cx).into_any_element(),
        };

        div()
            .flex()
            .flex_row()
            .flex_1()
            .child(sidebar)
            .child(main)
    }

    fn render_workbench(&self, cx: &mut Context<ShellView>) -> impl IntoElement {
        SessionView {
            colors: self.shell.colors,
            session: &self.shell.session,
            sessions: &self.shell.sessions,
            selected_session: self.shell.selected_session,
            messages: &self.shell.messages,
            message_list_state: &self.shell.message_list_state,
            new_message_count: self.shell.new_message_count,
            session_events: &self.shell.session_events,
            artifacts: &self.shell.artifacts,
            selected_artifact: self.shell.selected_artifact,
            artifact_preview: &self.shell.artifact_preview,
            data_state: &self.shell.data_state,
            composer_text: self.shell.composer.text(),
            composer_cursor: self.shell.composer.cursor(),
            composer_attachment_text: self.shell.composer_attachment_input.text(),
            composer_attachment_cursor: self.shell.composer_attachment_input.cursor(),
            composer_attachments: &self.shell.composer_attachments,
            provider_options: self.provider_options.clone(),
            model_options: self.model_options.clone(),
            selected_provider: self.shell.composer_provider_id.clone(),
            selected_model: self.shell.composer_model_id.clone(),
            provider_menu_open: self.shell.composer_provider_menu_open,
            model_menu_open: self.shell.composer_model_menu_open,
            composer_notice: self.shell.composer_notice.clone(),
            composer_attachment_focus: &self.shell.composer_attachment_focus,
            composer_focus: &self.shell.composer_focus,
            stream_status: &self.shell.stream_status,
            resyncing: self.resyncing,
            show_sessions_pane: self.shell.show_sessions_pane,
            show_diff_pane: self.shell.show_diff_pane,
            show_artifacts_pane: self.shell.show_artifacts_pane,
            show_terminal_panel: self.shell.show_terminal_panel,
            diff_review_state: self.shell.diff_review_state.clone(),
            terminal_panel_state: self.shell.terminal_panel_state.clone(),
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
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .px_3()
                            .py_2()
                            .border_1()
                            .border_color(item_border)
                            .rounded_sm()
                            .bg(item_bg)
                            .text_sm()
                            .child(workspace.name.clone())
                            .cursor_pointer()
                            .id(ElementId::named_usize("workspace", index))
                            .on_click(on_click),
                    )
                })
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

    fn render_diagnostics(&self, _cx: &mut Context<ShellView>) -> impl IntoElement {
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

        div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_3()
            .p(px(metrics.spacing.gutter))
            .bg(colors.panel)
            .child(div().text_lg().child("Diagnostics"))
            .child(panel)
            .child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child("Detailed diagnostics are available in the web UI."),
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
                        .child("Recents are managed in the web launcher."),
                )
                .child(div().flex().items_center().gap_2().child(settings_button)),
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
