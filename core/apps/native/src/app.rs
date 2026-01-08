use std::collections::HashMap;

use gpui::{
    App, Application, Bounds, Context, ListAlignment, ListState, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use crate::theme::{ThemeColors, ThemeTokens};

#[path = "icons.rs"]
mod icons;
#[path = "workspace_summary.rs"]
mod workspace_summary;
#[path = "models.rs"]
mod models;
#[path = "state/mod.rs"]
mod state;
#[path = "views/mod.rs"]
mod views;

use self::icons::{Icon, IconAssets, IconName};
use self::models::{MessageItem, SessionInfo};
use self::state::{ArtifactPreviewState, ComposerState, DataLoadState, ShellView, StreamStatus};
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
    Application::new()
        .with_assets(IconAssets::new())
        .run(|cx: &mut App| {
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
                        composer_provider_id: None,
                        composer_model_id: None,
                        composer_provider_menu_open: false,
                        composer_model_menu_open: false,
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
                        session_last_event_seq: HashMap::new(),
                        session: SessionInfo::placeholder(),
                        data_state: DataLoadState::Loading,
                        composer: ComposerState::new(),
                        composer_focus: cx.focus_handle(),
                        composer_attachments: Vec::new(),
                        composer_attachment_input: ComposerState::new(),
                        composer_attachment_focus: cx.focus_handle(),
                        composer_notice: None,
                        message_list_state: ListState::new(1, ListAlignment::Bottom, px(160.0)),
                        message_list_len: 1,
                        message_auto_follow: true,
                        new_message_count: 0,
                        stream_status: StreamStatus::Idle,
                        resyncing_session: None,
                        stream_subscribe_tx: None,
                        stream_stop_tx: None,
                        message_list_handler_set: false,
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
        let toggle_text = if self.is_dark { "Light" } else { "Dark" };
        let toggle_label = div()
            .flex()
            .items_center()
            .gap_1()
            .child(Icon::new(
                IconName::Settings,
                12.0,
                self.colors.muted,
            ))
            .child(toggle_text);
        let resyncing = self
            .resyncing_session
            .and_then(|resync_id| {
                self.selected_session
                    .and_then(|index| self.sessions.get(index))
                    .map(|summary| summary.session_id == resync_id)
            })
            .unwrap_or(false);
        let provider_options = self.composer_provider_options();
        let model_options = self.composer_model_options();
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
                            message_list_state: &self.message_list_state,
                            new_message_count: self.new_message_count,
                            session_events: &self.session_events,
                            artifacts: &self.artifacts,
                            selected_artifact: self.selected_artifact,
                            artifact_preview: &self.artifact_preview,
                            data_state: &self.data_state,
                            composer_text: self.composer.text(),
                            composer_cursor: self.composer.cursor(),
                            composer_attachment_text: self.composer_attachment_input.text(),
                            composer_attachment_cursor: self.composer_attachment_input.cursor(),
                            composer_attachments: &self.composer_attachments,
                            provider_options,
                            model_options,
                            selected_provider: self.composer_provider_id.clone(),
                            selected_model: self.composer_model_id.clone(),
                            provider_menu_open: self.composer_provider_menu_open,
                            model_menu_open: self.composer_model_menu_open,
                            composer_notice: self.composer_notice.clone(),
                            composer_attachment_focus: &self.composer_attachment_focus,
                            composer_focus: &self.composer_focus,
                            stream_status: &self.stream_status,
                            resyncing,
                        }
                        .render(cx),
                    ),
            )
    }
}
