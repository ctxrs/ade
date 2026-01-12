use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::Utc;
use gpui::{
    App, Application, Bounds, ClickEvent, Context, ListAlignment, ListState, MouseMoveEvent,
    MouseUpEvent, Rgba, ScrollStrategy, Window, WindowBounds, WindowOptions, div,
    InteractiveElement as _, StatefulInteractiveElement as _, prelude::*, px, size,
};
use gpui_component::{Root, VirtualListScrollHandle, input::{InputEvent, InputState}};

use crate::automation;
use crate::automation_tree;
use crate::app_identity;
use crate::theme::{ThemeColors, ThemeTokens, apply_gpui_component_theme};
use self::ui_state::UiStateStore;

#[path = "icons.rs"]
mod icons;
#[path = "harness_catalog.rs"]
mod harness_catalog;
#[path = "workspace_summary.rs"]
mod workspace_summary;
#[path = "models.rs"]
mod models;
#[path = "model_effort.rs"]
mod model_effort;
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
    WorkbenchModeId, SessionViewVerbosity,
};
use self::views::{RouterView, SidebarOverlays};


use ctx_client::EnvTarget;
use ctx_core::models::MessageRole;
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

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
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

fn settings_section_from_fixture(value: &str) -> Option<state::SettingsSection> {
    match value {
        "general" => Some(state::SettingsSection::General),
        "agent_harnesses" => Some(state::SettingsSection::AgentHarnesses),
        "models_routing" => Some(state::SettingsSection::ModelsRouting),
        "sandboxing" => Some(state::SettingsSection::Sandboxing),
        "worktree_bootstrap" => Some(state::SettingsSection::WorktreeBootstrap),
        "workspace_attachments" => Some(state::SettingsSection::WorkspaceAttachments),
        "context_pack" => Some(state::SettingsSection::ContextPack),
        "resource_governance" => Some(state::SettingsSection::ResourceGovernance),
        "mobile_access" => Some(state::SettingsSection::MobileAccess),
        "resource_utilization" => Some(state::SettingsSection::ResourceUtilization),
        "dictation" => Some(state::SettingsSection::Dictation),
        "title_generation" => Some(state::SettingsSection::TitleGeneration),
        "billing" => Some(state::SettingsSection::Billing),
        "team_enterprise" => Some(state::SettingsSection::TeamEnterprise),
        "usage_analytics" => Some(state::SettingsSection::UsageAnalytics),
        _ => None,
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
        app_identity::apply_app_identity();
        gpui_tokio::init(cx);
        gpui_component::init(cx);
        apply_gpui_component_theme(&theme_tokens, is_dark, cx);
        let window_size = options.window_size.unwrap_or_default();
        let bounds = Bounds::centered(
            None,
            size(px(window_size.width), px(window_size.height)),
            cx,
        );
        let (initial_route, initial_settings_section) = match options.automation.fixture.as_deref() {
            Some("settings") => (Some(ShellRoute::Settings), None),
            Some("providers") => (Some(ShellRoute::Providers), None),
            Some(fixture) if fixture.starts_with("settings:") => {
                let section = fixture
                    .strip_prefix("settings:")
                    .and_then(settings_section_from_fixture);
                (Some(ShellRoute::Settings), section)
            }
            _ => (None, None),
        };

        let window = cx
            .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("ctx".to_string()),
                ..Default::default()
            },
            |window, cx| {
                let base_url = base_url.clone();
                let initial_route = initial_route;
                let initial_settings_section = initial_settings_section;
                let view = cx.new(|cx| {
                    let diff_review_state = cx.new(|_| DiffReviewState::new());
                    let terminal_panel_state =
                        cx.new(|cx| TerminalPanelState::new(colors, cx.focus_handle()));
                    let settings_state = cx.new(|_| SettingsState::new(colors, is_dark));
                    if let Some(section) = initial_settings_section {
                        settings_state.update(cx, |state, cx| {
                            state.set_active_section(section, cx);
                        });
                    }
                    let task_search_input =
                        cx.new(|cx| InputState::new(window, cx).placeholder("Search Tasks"));
                    let rename_input = cx.new(|cx| InputState::new(window, cx));
                    let ui_state = UiStateStore::load();
                    let archive_confirm_dismissed = ui_state.archive_confirm_dismissed();
                    let mut view = ShellView {
                        colors,
                        // Will set shell handle on settings state below after the ShellView entity exists.
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
                            MessageRole::Assistant,
                            "Loading session messages...",
                        )],
                        session_turns: Vec::new(),
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
                        selected_artifact: None,
                        artifact_preview: ArtifactPreviewState::None,
                        session_events: Vec::new(),
                        session_summary_map: HashMap::new(),
                        session: SessionInfo::placeholder(),
                        data_state: DataLoadState::Loading,
                        new_task_mode: true,
                        new_task_mode_locked: true,
                        composer_new_input: cx.new(|cx| InputState::new(window, cx)),
                        composer_session_input: cx.new(|cx| InputState::new(window, cx)),
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
                        composer_env_target: EnvTarget::Local,
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
                        composer_harness_search: cx.new(|cx| InputState::new(window, cx)),
                        composer_model_search: cx.new(|cx| InputState::new(window, cx)),
                        composer_model_search_placeholder: String::new(),
                        composer_model_manual: cx.new(|cx| InputState::new(window, cx)),
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
                        thread_list_state: ListState::new(0, ListAlignment::Bottom, px(160.0)),
                        thread_list_len: 0,
                        thread_auto_follow: true,
                        new_thread_item_count: 0,
                        copied_flags: HashMap::new(),
                        stream_status: StreamStatus::Idle,
                        resyncing_session: None,
                        stream_subscribe_tx: None,
                        stream_stop_tx: None,
                        session_last_event_seq: HashMap::new(),
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
                    if let Some(route) = initial_route {
                        view.set_route(route, cx);
                    }
                    // Give the settings state a handle to this ShellView for navigation (e.g., sidebar backlink).
                    let shell_handle = cx.entity();
                    view.settings_state.update(cx, |state, _cx| {
                        state.set_shell_handle(shell_handle);
                    });
                    view
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .unwrap();
        automation::start(cx, window, options.automation.clone());
        cx.activate(true);
    });
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Wire composer state: subscriptions, provider options, track inputs, and placeholders.
        self.init_composer(window, cx);
        self.ensure_composer_provider_options(cx);
        self.ensure_track_model_inputs(window, cx);
        self.update_composer_placeholders(window, cx);
        self.sync_auxiliary_panes(cx);
        if self.sidebar_resizing {
            let view_handle = cx.entity();
            window.on_mouse_event({
                let view_handle = view_handle.clone();
                move |event: &MouseMoveEvent, _, window, cx| {
                    let _ = view_handle.update(cx, |view, cx| {
                        if let Some(state) = view.sidebar_resize_state {
                            let delta = f32::from(event.position.x) - state.start_x;
                            let next_width = state.start_width + delta;
                            view.set_sidebar_width(next_width, window, cx);
                        }
                    });
                }
            });
            window.on_mouse_event({
                let view_handle = view_handle.clone();
                move |_: &MouseUpEvent, _, _window, cx| {
                    let _ = view_handle.update(cx, |view, cx| {
                        if view.sidebar_resizing {
                            view.sidebar_resizing = false;
                            view.sidebar_resize_state = None;
                            view.sidebar_resizer_hovered = false;
                            cx.notify();
                        }
                    });
                }
            });
        }
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
        let mut title_group = div().flex().items_center().gap(px(10.0));
        if self.route == ShellRoute::Workbench && self.sidebar_collapsed {
            let expand = div()
                .w(px(26.0))
                .h(px(26.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(self.colors.border)
                .bg(rgba(255, 255, 255, 0.03))
                .text_size(px(16.0))
                .text_color(self.colors.text)
                .flex()
                .items_center()
                .justify_center()
                .child("›")
                .cursor_pointer()
                .id("sidebar-expand")
                .active(|style| style.opacity(0.85))
                .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                    view.set_sidebar_collapsed(false, cx);
                }));
            title_group = title_group.child(expand);
        }
        title_group = title_group.child(title_label);
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
            .relative()
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
                            .child(title_group)
                            .child(
                                div()
                                    .id("settings-button")
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
                                    .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                                        view.set_route(ShellRoute::Settings, cx);
                                    })),
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
            .when(self.route == ShellRoute::Workbench, |this| {
                this.child(
                    SidebarOverlays {
                        shell: self,
                        viewport: window.bounds().size,
                    }
                    .render(cx),
                )
            })
    }
}
