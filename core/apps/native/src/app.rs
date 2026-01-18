use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::Utc;
use gpui::{
    App, Application, Bounds, ClickEvent, Context, Entity, Global, KeyDownEvent, ListAlignment, ListState,
    ScrollStrategy, Window, WindowBounds, WindowHandle, WindowOptions, div,
    InteractiveElement as _, StatefulInteractiveElement as _, prelude::*, px, size,
};
use gpui_component::{Root, TitleBar, VirtualListScrollHandle, input::{InputEvent, InputState}};
use ctx_core::models::MessageRole;

use crate::automation;
use crate::automation_tree;
use crate::app_identity;
use crate::app_menus;
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

use self::icons::{AppAssets, Icon, IconName};
use self::models::{MessageItem, SessionInfo};
use self::state::{
    ArtifactContentCache, ArtifactPreviewState, AtsCache, ComposerAutocompleteState, ComposerDraft,
    ComposerVerbosity, DataLoadState, DiffReviewState, SessionViewVerbosity, SettingsState,
    StreamStatus, TaskFetchState, TerminalPanelState, WorkbenchModeId,
};
use self::views::RouterView;
pub(crate) use self::state::{ShellRoute, ShellView};
#[cfg(feature = "automation")]
pub(crate) use self::state::ComposerMenuId;

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

#[derive(Clone, Debug)]
pub(crate) struct AppWindowConfig {
    pub(crate) colors: ThemeColors,
    pub(crate) is_dark: bool,
    pub(crate) window_size: WindowSize,
}

impl Global for AppWindowConfig {}

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

fn resolve_base_url() -> String {
    ctx_client::resolve_daemon_config()
        .map(|cfg| cfg.base_url)
        .unwrap_or_else(|err| {
            eprintln!("ctx-native: daemon config failed: {err}");
            "unknown".to_string()
        })
}

fn create_shell_view(
    window: &mut Window,
    cx: &mut App,
    base_url: String,
    colors: ThemeColors,
    is_dark: bool,
    route: ShellRoute,
) -> Entity<ShellView> {
    cx.new(|cx| {
        let diff_review_state = cx.new(|_| DiffReviewState::new());
        let terminal_panel_state =
            cx.new(|cx| TerminalPanelState::new(colors, cx.focus_handle()));
        let settings_state = cx.new(|_| SettingsState::new(colors, is_dark));
        let task_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search Tasks"));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let ui_state = UiStateStore::load();
        let archive_confirm_dismissed = ui_state.archive_confirm_dismissed();
        let composer_new_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 22)
                .placeholder("@ for context, / for commands")
        });
        let composer_session_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 12)
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
            route,
            workspaces: Vec::new(),
            selected_workspace: None,
            providers: Vec::new(),
            composer_provider_id: None,
            composer_model_id: None,
            composer_provider_menu_open: false,
            composer_model_menu_open: false,
            task_store_initialized: false,
            task_fetch_active: TaskFetchState::Idle,
            task_fetch_archived: TaskFetchState::Idle,
            task_has_more_active: true,
            task_has_more_archived: false,
            task_archived_loaded: false,
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
                MessageRole::Assistant,
                "Loading workspace data...",
            )],
            session_turns: Vec::new(),
            session_history_cursor: None,
            session_history_has_more: false,
            session_history_loading: false,
            session_turn_tools: HashMap::new(),
            thread_items: Vec::new(),
            sticky_turn_header: None,
            sticky_turn_header_at_top: true,
            expanded_turn_headers: HashMap::new(),
            expanded_messages: HashMap::new(),
            expanded_turn_details: HashMap::new(),
            expanded_tools: HashMap::new(),
            turn_tools_loading: HashSet::new(),
            verbosity: SessionViewVerbosity::Default,
            verbosity_menu_open: false,
            artifacts: Vec::new(),
            artifacts_session_id: None,
            selected_artifact: None,
            artifact_preview: ArtifactPreviewState::None,
            artifact_prefetch_cache: ArtifactContentCache::default(),
            artifact_prefetch_session_id: None,
            artifact_prefetch_inflight: HashSet::new(),
            session_events: Vec::new(),
            session_thread_cache: HashMap::new(),
            session_head_meta: HashMap::new(),
            session_summary_map: HashMap::new(),
            session_last_event_seq: HashMap::new(),
            session: SessionInfo::placeholder(),
            data_state: DataLoadState::Loading,
            new_task_mode: true,
            new_task_mode_locked: false,
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
            composer_focus_pending: true,
            composer_has_focus: false,
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
            attachment_fetch_failed: HashSet::new(),
            ats_cache: AtsCache::new(),
            composer_subscriptions: Vec::new(),
            composer_subscriptions_set: false,
            thread_list_state: ListState::new(0, ListAlignment::Bottom, px(160.0)),
            thread_list_len: 0,
            thread_item_layout_hashes: HashMap::new(),
            thread_auto_follow: true,
            new_thread_item_count: 0,
            copied_flags: HashMap::new(),
            stream_status: StreamStatus::Idle,
            resyncing_session: None,
            stream_subscribe_tx: None,
            stream_stop_tx: None,
            thread_list_handler_set: false,
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
    })
}

pub(crate) fn open_shell_window(
    cx: &mut App,
    route: ShellRoute,
) -> anyhow::Result<WindowHandle<Root>> {
    let config = cx.global::<AppWindowConfig>();
    let window_size = config.window_size;
    let bounds = Bounds::centered(
        None,
        size(px(window_size.width), px(window_size.height)),
        cx,
    );
    let base_url = resolve_base_url();
    let colors = config.colors;
    let is_dark = config.is_dark;

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("ctx".to_string()),
            titlebar: Some(TitleBar::title_bar_options()),
            ..Default::default()
        },
        |window, cx| {
            let view = create_shell_view(window, cx, base_url, colors, is_dark, route);
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
}

pub fn run(options: AppOptions) {
    let is_dark = true;
    let (theme_tokens, colors) = load_theme(is_dark);
    let app = Application::new().with_assets(AppAssets::new());
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        crate::theme::apply_gpui_component_theme(&theme_tokens, is_dark, cx);
        app_identity::apply_app_identity();
        gpui_tokio::init(cx);
        let window_size = options.window_size.unwrap_or_default();
        cx.set_global(AppWindowConfig {
            colors,
            is_dark,
            window_size,
        });
        app_menus::init(cx);
        let window = open_shell_window(cx, ShellRoute::Workbench).unwrap();
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
        let settings_icon = Icon::new(IconName::Settings, 14.0, self.colors.text);
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
        let settings_button = div()
            .id("settings-button")
            .w(px(26.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(self.colors.panel_2)
            .border_1()
            .border_color(self.colors.border)
            .cursor_pointer()
            .active(|style| style.opacity(0.85))
            .child(settings_icon)
            .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                view.set_route(ShellRoute::Settings, cx);
            }));
        let title_bar = TitleBar::new()
            .bg(self.colors.panel)
            .border_color(self.colors.border_strong)
            .child(
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .child(div().w(px(26.0)).flex_none())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .child(workspace_label),
                    )
                    .child(settings_button),
            );
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
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                let key = event.keystroke.key.to_lowercase();
                let has_modifier = modifiers.platform || modifiers.control;
                if !has_modifier || modifiers.alt || modifiers.shift {
                    return;
                }
                if key == "b" {
                    if view.route == ShellRoute::Workbench {
                        let collapsed = view.sidebar_collapsed;
                        view.set_sidebar_collapsed(!collapsed, cx);
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                if key == "n" {
                    if view.route != ShellRoute::Workbench {
                        view.set_route(ShellRoute::Workbench, cx);
                    }
                    view.focus_new_task_shortcut(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(
                title_bar,
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
