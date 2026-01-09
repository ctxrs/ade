use std::collections::HashMap;
use std::str::FromStr;

use gpui::{
    App, Application, Bounds, Context, ListAlignment, ListState, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use crate::automation;
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
use self::state::{
    ArtifactPreviewState, ComposerState, DataLoadState, DiffReviewState, SettingsState,
    ShellRoute, StreamStatus, TerminalPanelState,
};
use self::views::RouterView;

pub(crate) use self::state::ShellView;

#[derive(Clone, Debug)]
pub struct AppOptions {
    pub window_size: Option<WindowSize>,
    pub automation: automation::AutomationConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
        }
    }
}

impl FromStr for WindowSize {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (width, height) = value
            .split_once('x')
            .or_else(|| value.split_once('X'))
            .ok_or_else(|| "window size must be WIDTHxHEIGHT".to_string())?;
        let width = width
            .parse::<f32>()
            .map_err(|_| "window width must be a number".to_string())?;
        let height = height
            .parse::<f32>()
            .map_err(|_| "window height must be a number".to_string())?;
        if width <= 0.0 || height <= 0.0 {
            return Err("window size must be positive".to_string());
        }
        Ok(Self { width, height })
    }
}

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

pub fn run(options: AppOptions) {
    let is_dark = true;
    let colors = load_theme_colors(is_dark);
    let base_url = ctx_client::resolve_daemon_config()
        .map(|cfg| cfg.base_url)
        .unwrap_or_else(|err| {
            eprintln!("ctx-native: daemon config failed: {err}");
            "unknown".to_string()
        });
    let app = Application::new()
        .with_assets(IconAssets::new());
    app
        .run(move |cx: &mut App| {
        gpui_tokio::init(cx);
        let window_size = options.window_size.unwrap_or_default();
        let bounds = Bounds::centered(
            None,
            size(px(window_size.width), px(window_size.height)),
            cx,
        );
        let window = cx
            .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let base_url = base_url.clone();
                cx.new(|cx| {
                    let diff_review_state = cx.new(|_| DiffReviewState::new());
                    let terminal_panel_state =
                        cx.new(|cx| TerminalPanelState::new(colors, cx.focus_handle()));
                    let settings_state = cx.new(|_| SettingsState::new(colors));
                    let mut view = ShellView {
                        colors,
                        base_url,
                        is_dark,
                        route: ShellRoute::Workbench,
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
                        show_sessions_pane: false,
                        show_diff_pane: false,
                        show_artifacts_pane: false,
                        show_terminal_panel: false,
                        diff_review_state,
                        terminal_panel_state,
                        settings_state,
                        aux_workspace_id: None,
                        aux_session_id: None,
                    };
                    view.start_data_load(cx);
                    view
                })
            },
        )
        .unwrap();
        automation::start(cx, window, options.automation.clone());
        cx.activate(true);
    });
}

impl Render for ShellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_auxiliary_panes(cx);
        let toggle_label = Icon::new(IconName::Settings, 12.0, self.colors.muted);
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
        let workspace_label = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
            .map(|ws| ws.name.clone())
            .unwrap_or_else(|| "No workspace".to_string());
        let task_label = self
            .selected_task
            .and_then(|index| self.tasks.get(index))
            .map(|task| task.title.clone());
        let mut title_label = div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .child(workspace_label);
        if let Some(task_label) = task_label {
            title_label = title_label.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(task_label),
            );
        }
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
                            .child(title_label)
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
                        RouterView {
                            shell: self,
                            provider_options,
                            model_options,
                            resyncing,
                        }
                        .render(cx),
                    ),
            )
    }
}
