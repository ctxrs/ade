use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::Utc;
use gpui::{
    App, Application, Bounds, Context, ListAlignment, ListState, ScrollStrategy, Window,
    WindowBounds, WindowOptions, div,
    InteractiveElement as _, StatefulInteractiveElement as _, prelude::*, px, size,
};
use gpui_component::{VirtualListScrollHandle, input::{InputEvent, InputState}};

use crate::automation;
use crate::automation_tree;
use crate::app_identity;
use crate::theme::{ThemeColors, ThemeTokens};
use self::ui_state::UiStateStore;

#[path = "icons.rs"]
mod icons;
#[path = "harness_catalog.rs"]
mod harness_catalog;
#[path = "model_effort.rs"]
mod model_effort;
#[path = "workspace_summary.rs"]
mod workspace_summary;
#[path = "models.rs"]
mod models;
#[path = "relative_time.rs"]
mod relative_time;
#[path = "ui_state.rs"]
mod ui_state;
#[path = "state/mod.rs"]
mod state;
#[path = "views/mod.rs"]
mod views;

use self::icons::{Icon, IconAssets, IconName};
use self::models::{MessageItem, SessionInfo};
use self::state::{
    ArtifactPreviewState, ComposerAutocompleteState, ComposerDraft, ComposerVerbosity, DataLoadState,
    DiffReviewState, SettingsState, ShellRoute, StreamStatus, TaskFetchState, TerminalPanelState,
    WorkbenchModeId,
};
use ctx_client::EnvTarget;
use self::views::RouterView;

pub(crate) use self::state::{ComposerMenuId, ShellView};

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

fn load_theme(is_dark: bool) -> (ThemeTokens, ThemeColors) {
    let tokens = if is_dark {
        ThemeTokens::dark_from_web().unwrap_or_else(|err| {
            eprintln!("ctx-native: theme load failed: {err}");
            ThemeTokens::dark_fallback()
        })
    } else {
        ThemeTokens::light()
    };
    let colors = ThemeColors::from_tokens(&tokens).unwrap_or_else(|err| {
        eprintln!("ctx-native: theme parse failed: {err}");
        ThemeColors::fallback_dark()
    });
    (tokens, colors)
}

pub fn run(options: AppOptions) {
    let is_dark = true;
    let (theme_tokens, colors) = load_theme(is_dark);
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
        gpui_component::init(cx);
        crate::theme::apply_gpui_component_theme(&theme_tokens, is_dark, cx);
        app_identity::apply_app_identity();
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
                app_id: Some("ctx".to_string()),
                ..Default::default()
            },
            |window, cx| {
                let base_url = base_url.clone();
                let view = cx.new(|cx| {
                    let diff_review_state = cx.new(|_| DiffReviewState::new());
                    let terminal_panel_state =
                        cx.new(|cx| TerminalPanelState::new(colors, cx.focus_handle()));
                    let settings_state = cx.new(|_| SettingsState::new(colors));
                    let task_search_input =
                        cx.new(|cx| InputState::new(window, cx).placeholder("Search Tasks"));
                    let rename_input = cx.new(|cx| InputState::new(window, cx));
                    let ui_state = UiStateStore::load();
                    let archive_confirm_dismissed = ui_state.archive_confirm_dismissed();
                    let composer_new_input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .multi_line(true)
                            .auto_grow(1, 22)
                            .placeholder("@ for context, / for commands")
                    });
                    let composer_session_input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .multi_line(true)
                            .auto_grow(1, 8)
                            .placeholder("@ for context, / for commands")
                    });
                    let composer_harness_search = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("Search agents")
                    });
                    let composer_model_search = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("Search models")
                    });
                    let composer_model_manual = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("model_id")
                    });
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
                        task_store_initialized: false,
                        task_fetch_active: TaskFetchState::Idle,
                        task_fetch_archived: TaskFetchState::Idle,
                        task_has_more_active: true,
                        task_has_more_archived: false,
                        task_archived_loaded: false,
                        task_active_cursor: None,
                        task_archived_cursor: None,
                        tasks_by_id: HashMap::new(),
                        task_active_order: Vec::new(),
                        task_archived_order: Vec::new(),
                        task_query: String::new(),
                        task_search_input: task_search_input.clone(),
                        task_list_scroll_handle: VirtualListScrollHandle::new(),
                        task_hovered: None,
                        task_menu: None,
                        selected_task: None,
                        renaming_task_id: None,
                        rename_input: rename_input.clone(),
                        rename_ignore_blur: false,
                        archive_pending: HashMap::new(),
                        task_mark_read_inflight: HashSet::new(),
                        archive_confirm: None,
                        archive_confirm_dont_remind: false,
                        archive_confirm_dismissed,
                        sidebar_width: 260.0,
                        sidebar_collapsed: false,
                        sidebar_anim_epoch: 0,
                        sidebar_resizing: false,
                        sidebar_resize_state: None,
                        sidebar_resizer_hovered: false,
                        archived_collapsed: true,
                        relative_now: Utc::now(),
                        relative_time_task: None,
                        ui_state,
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
                        new_task_mode: true,
                        composer_new_input,
                        composer_session_input,
                        composer_attachments: Vec::new(),
                        composer_new_attachments: Vec::new(),
                        composer_session_attachments: HashMap::new(),
                        composer_new_draft: ComposerDraft::default(),
                        composer_session_drafts: HashMap::new(),
                        composer_start_busy: false,
                        composer_start_error: None,
                        composer_needs_apply: false,
                        composer_notice: None,
                        composer_open_menu: None,
                        composer_menu_trigger_bounds: HashMap::new(),
                        composer_menu_bounds: HashMap::new(),
                        composer_menu_placements: HashMap::new(),
                        composer_tooltip_open: None,
                        composer_tooltip_trigger_bounds: HashMap::new(),
                        composer_tooltip_bounds: HashMap::new(),
                        composer_tooltip_placements: HashMap::new(),
                        composer_tooltip_close_id: 0,
                        composer_mode_id: WorkbenchModeId::Default,
                        composer_verbosity: ComposerVerbosity::Default,
                        composer_context_window: None,
                        composer_env_target: EnvTarget::Worktree,
                        composer_use_multiple_agents: false,
                        composer_draft_tracks: Vec::new(),
                        composer_provider_options: HashMap::new(),
                        composer_provider_opts_busy: HashMap::new(),
                        composer_provider_auth_busy: HashMap::new(),
                        composer_provider_verify_busy: HashMap::new(),
                        composer_provider_installs: HashMap::new(),
                        composer_install_polling: HashSet::new(),
                        composer_install_all_busy: false,
                        composer_provider_action_notice: None,
                        composer_provider_action_error: None,
                        composer_harness_expanded_provider: None,
                        composer_harness_count_menu_provider: None,
                        composer_harness_count_menu_placement: None,
                        composer_harness_count_trigger_bounds: HashMap::new(),
                        composer_harness_count_menu_bounds: None,
                        composer_recording: false,
                        composer_harness_search,
                        composer_model_search,
                        composer_model_search_placeholder: "Search models".to_string(),
                        composer_model_manual,
                        composer_autocomplete: ComposerAutocompleteState::new(),
                        composer_track_model_inputs: HashMap::new(),
                        composer_track_model_placeholders: HashMap::new(),
                        composer_track_input_subscriptions: HashMap::new(),
                        composer_autocomplete_input_bounds: None,
                        composer_autocomplete_anchor_bounds: None,
                        composer_autocomplete_menu_placement: None,
                        composer_autocomplete_menu_width: None,
                        composer_autocomplete_preview_placement: None,
                        composer_attachment_images: HashMap::new(),
                        composer_attachment_loading: HashSet::new(),
                        composer_subscriptions: Vec::new(),
                        composer_subscriptions_set: false,
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
                    let search_subscription = cx.subscribe_in(
                        &task_search_input,
                        window,
                        |view: &mut ShellView, state: &gpui::Entity<InputState>, event, _window, cx| {
                            if matches!(event, InputEvent::Change) {
                                let value = state.read(cx).value().to_string();
                                if view.task_query != value {
                                    view.task_query = value;
                                    view.task_list_scroll_handle
                                        .scroll_to_item(0, ScrollStrategy::Top);
                                    cx.notify();
                                }
                            }
                        },
                    );
                    search_subscription.detach();

                    let rename_subscription = cx.subscribe_in(
                        &rename_input,
                        window,
                        |view, state: &gpui::Entity<InputState>, event, _window, cx| {
                            match event {
                                InputEvent::PressEnter { .. } => {
                                    if let Some(task_id) = view.renaming_task_id {
                                        let value = state.read(cx).value().to_string();
                                        view.rename_ignore_blur = true;
                                        view.commit_task_rename(task_id, value, cx);
                                    }
                                }
                                InputEvent::Blur => {
                                    if let Some(task_id) = view.renaming_task_id {
                                        if view.rename_ignore_blur {
                                            view.rename_ignore_blur = false;
                                            return;
                                        }
                                        let value = state.read(cx).value().to_string();
                                        view.commit_task_rename(task_id, value, cx);
                                    }
                                }
                                _ => {}
                            }
                        },
                    );
                    rename_subscription.detach();

                    view.start_relative_time(cx);
                    view.start_data_load(cx);
                    view
                });
                cx.new(|cx| gpui_component::Root::new(view, window, cx))
            },
        )
        .unwrap();
        automation::start(cx, window, options.automation.clone());
        cx.activate(true);
    });
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.init_composer(window, cx);
        if self.composer_needs_apply {
            self.composer_needs_apply = false;
            self.apply_active_composer_state(window, cx);
        }
        self.ensure_composer_provider_options(cx);
        self.update_composer_placeholders(window, cx);
        self.ensure_track_model_inputs(window, cx);
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
        let workspace_label = self
            .selected_workspace
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
            .map(|ws| ws.name.clone())
            .unwrap_or_else(|| "No workspace".to_string());
        let task_label = self
            .selected_task
            .and_then(|task_id| self.tasks_by_id.get(&task_id))
            .map(|task| task.task.title.clone());
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
            .on_children_prepainted(automation_tree::track_children_bounds(
                "app-shell",
                "application",
                Some("ctx"),
                None,
            ))
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
                            resyncing,
                        }
                        .render(cx),
                    ),
            )
    }
}
