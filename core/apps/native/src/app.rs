use std::collections::HashMap;

use gpui::{
    App, Application, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    size,
};

use crate::theme::{ThemeColors, ThemeTokens};

#[path = "workspace_summary.rs"]
mod workspace_summary;
#[path = "models.rs"]
mod models;
#[path = "state.rs"]
mod state;
#[path = "views/mod.rs"]
mod views;

use self::models::{MessageItem, SessionInfo};
use self::state::{ArtifactPreviewState, DataLoadState, ShellView};
use self::views::{SessionView, SidebarView};

fn load_theme_colors(is_dark: bool) -> ThemeColors {
    let tokens = if is_dark {
        ThemeTokens::dark_from_web().unwrap_or_else(|err| {
            eprintln!("ctx-native: theme load failed: {err}");
            ThemeTokens::dark_fallback()
        })
    } else {
        ThemeTokens::light()
    };
    ThemeColors::from_tokens(&tokens).unwrap_or_else(|err| {
        eprintln!("ctx-native: theme parse failed: {err}");
        ThemeColors::fallback_dark()
    })
}

pub fn run() {
    let is_dark = true;
    let colors = load_theme_colors(is_dark);
    let base_url = ctx_client::resolve_daemon_config()
        .map(|cfg| cfg.base_url)
        .unwrap_or_else(|err| {
            eprintln!("ctx-native: daemon config failed: {err}");
            "unknown".to_string()
        });
    Application::new().run(|cx: &mut App| {
        gpui_tokio::init(cx);
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let base_url = base_url.clone();
                cx.new(|cx| {
                    let mut view = ShellView {
                        colors,
                        base_url,
                        is_dark,
                        workspaces: Vec::new(),
                        selected_workspace: None,
                        providers: Vec::new(),
                        catchup_active_total: None,
                        catchup_archived_total: None,
                        tasks: Vec::new(),
                        selected_task: None,
                        sessions: Vec::new(),
                        selected_session: None,
                        messages: vec![MessageItem::new(
                            "assistant",
                            "Loading workspace data...",
                        )],
                        artifacts: Vec::new(),
                        session_events: Vec::new(),
                        selected_artifact: None,
                        artifact_preview: ArtifactPreviewState::None,
                        session_summary_map: HashMap::new(),
                        session: SessionInfo::placeholder(),
                        data_state: DataLoadState::Loading,
                        composer_text: String::new(),
                        composer_focus: cx.focus_handle(),
                    };
                    view.start_data_load(cx);
                    view
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

impl Render for ShellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle_label = if self.is_dark { "Light" } else { "Dark" };
        div()
            .id("app-shell")
            .size_full()
            .flex()
            .flex_col()
            .bg(self.colors.bg)
            .text_color(self.colors.text)
            .child(
                div()
                    .id("title-bar")
                    .flex()
                    .items_center()
                    .h(px(44.0))
                    .px_3()
                    .bg(self.colors.panel)
                    .border_b_1()
                    .border_color(self.colors.border_strong)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .child(format!("ctx-native · {base_url}", base_url = self.base_url))
                            .child(
                                div()
                                    .id("theme-toggle")
                                    .flex_none()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .bg(self.colors.panel_2)
                                    .border_1()
                                    .border_color(self.colors.border)
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .active(|this| this.opacity(0.85))
                                    .child(toggle_label)
                                    .on_click(cx.listener(Self::toggle_theme)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("content")
                    .flex()
                    .flex_row()
                    .flex_1()
                    .child(
                        SidebarView {
                            colors: self.colors,
                            workspaces: &self.workspaces,
                            selected_workspace: self.selected_workspace,
                            providers: &self.providers,
                            tasks: &self.tasks,
                            selected_task: self.selected_task,
                        }
                        .render(cx),
                    )
                    .child(
                        SessionView {
                            colors: self.colors,
                            workspaces: &self.workspaces,
                            selected_workspace: self.selected_workspace,
                            catchup_active_total: self.catchup_active_total,
                            catchup_archived_total: self.catchup_archived_total,
                            tasks: &self.tasks,
                            session: &self.session,
                            sessions: &self.sessions,
                            selected_session: self.selected_session,
                            messages: &self.messages,
                            session_events: &self.session_events,
                            artifacts: &self.artifacts,
                            selected_artifact: self.selected_artifact,
                            artifact_preview: &self.artifact_preview,
                            data_state: &self.data_state,
                            composer_text: self.composer_text.as_str(),
                            composer_focus: &self.composer_focus,
                        }
                        .render(cx),
                    ),
            )
    }
}
