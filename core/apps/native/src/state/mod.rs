pub(super) mod artifacts;
pub(super) mod composer;
pub(super) mod diff_review;
pub(super) mod session;
pub(super) mod settings;
pub(super) mod stream;
pub(super) mod terminal;
#[allow(dead_code)]
pub(super) mod turn_tools;
pub(super) mod workspace;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gpui::AppContext as _;
use gpui::{
    AsyncApp, Bounds, ClickEvent, Context, Entity, Image, ListState, Pixels, Subscription, Task,
    WeakEntity, Window,
};
use gpui_component::{VirtualListScrollHandle, input::InputState};
use gpui_tokio::Tokio;
use ctx_client::{EnvTarget, ProviderOptions};
use tokio::sync::watch;

use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId};
use ctx_core::models::{
    Artifact, MessageAttachment, SessionEvent, SessionSnapshotSummary, SessionTurn,
    WorkspaceActiveSnapshotClientMessage,
};

use crate::theme::ThemeColors;
use super::ui_state::UiStateStore;

use super::models::{MessageItem, SessionInfo, ThreadListItem, TurnToolSnapshot, WorkbenchTurnHeader};
use super::workspace_summary::{SessionSummaryItem, TaskSummaryItem};
use session::SessionThreadCache;

pub(crate) use artifacts::ArtifactPreviewState;
pub(crate) use composer::{
    ComposerAutocompleteState, ComposerDraft, ComposerMenuId, ComposerState, ComposerVerbosity,
    ContextWindowInfo, DraftTrack, PopoverPlacement, ProviderInstallState, WorkbenchModeId,
};
pub(crate) use diff_review::DiffReviewState;
pub(crate) use settings::{
    LabeledOption, SettingsInputKind, SettingsSection, SettingsSectionGroup, SettingsSelectKind,
    SettingsState, SETTINGS_SECTIONS,
};
pub(crate) use stream::StreamStatus;
pub(crate) use terminal::{TerminalContext, TerminalPanelState};
pub(crate) use workspace::{DataLoadState, ProviderItem, WorkspaceItem};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellRoute {
    Workbench,
    Workspaces,
    Settings,
    Providers,
    Diagnostics,
    AppSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskFetchState {
    Idle,
    Loading,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskArchiveAction {
    Archive,
    Unarchive,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AnchorRect {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArchiveConfirmState {
    pub(crate) task_id: TaskId,
    pub(crate) anchor: AnchorRect,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskMenuState {
    pub(crate) task_id: TaskId,
    pub(crate) anchor: AnchorRect,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct SidebarResizeState {
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionViewVerbosity {
    Terse,
    Default,
    Verbose,
}

impl SessionViewVerbosity {
    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            SessionViewVerbosity::Terse => "Terse",
            SessionViewVerbosity::Default => "Default",
            SessionViewVerbosity::Verbose => "Verbose",
        }
    }
}

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    #[allow(dead_code)]
    pub(crate) is_dark: bool,
    pub(crate) route: ShellRoute,
    pub(crate) workspaces: Vec<WorkspaceItem>,
    pub(crate) selected_workspace: Option<WorkspaceId>,
    pub(crate) providers: Vec<ProviderItem>,
    pub(crate) composer_provider_id: Option<String>,
    pub(crate) composer_model_id: Option<String>,
    pub(crate) composer_provider_menu_open: bool,
    pub(crate) composer_model_menu_open: bool,
    pub(crate) task_store_initialized: bool,
    pub(crate) task_fetch_active: TaskFetchState,
    pub(crate) task_fetch_archived: TaskFetchState,
    pub(crate) task_has_more_active: bool,
    pub(crate) task_has_more_archived: bool,
    pub(crate) task_archived_loaded: bool,
    pub(crate) tasks_by_id: HashMap<TaskId, TaskSummaryItem>,
    pub(crate) task_active_order: Vec<TaskId>,
    pub(crate) task_archived_order: Vec<TaskId>,
    pub(crate) task_query: String,
    pub(crate) task_search_input: Entity<InputState>,
    pub(crate) task_list_scroll_handle: VirtualListScrollHandle,
    pub(crate) task_hovered: Option<TaskId>,
    pub(crate) task_menu: Option<TaskMenuState>,
    pub(crate) selected_task: Option<TaskId>,
    pub(crate) renaming_task_id: Option<TaskId>,
    pub(crate) rename_input: Entity<InputState>,
    pub(crate) rename_ignore_blur: bool,
    pub(crate) archive_pending: HashMap<TaskId, TaskArchiveAction>,
    pub(crate) task_mark_read_inflight: HashSet<TaskId>,
    pub(crate) archive_confirm: Option<ArchiveConfirmState>,
    pub(crate) archive_confirm_dont_remind: bool,
    pub(crate) archive_confirm_dismissed: bool,
    pub(crate) sidebar_width: f32,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) sidebar_anim_epoch: u64,
    pub(crate) sidebar_resizing: bool,
    pub(crate) sidebar_resize_state: Option<SidebarResizeState>,
    pub(crate) sidebar_resizer_hovered: bool,
    pub(crate) archived_collapsed: bool,
    pub(crate) relative_now: DateTime<Utc>,
    pub(crate) relative_time_task: Option<Task<()>>,
    pub(crate) ui_state: UiStateStore,
    pub(crate) sessions: Vec<SessionSummaryItem>,
    pub(crate) selected_session: Option<usize>,
    pub(crate) messages: Vec<MessageItem>,
    pub(crate) session_turns: Vec<SessionTurn>,
    pub(crate) session_history_cursor: Option<i64>,
    pub(crate) session_history_has_more: bool,
    pub(crate) session_history_loading: bool,
    pub(crate) session_turn_tools: HashMap<TurnId, Vec<TurnToolSnapshot>>,
    pub(crate) thread_items: Vec<ThreadListItem>,
    pub(crate) sticky_turn_header: Option<WorkbenchTurnHeader>,
    pub(crate) sticky_turn_header_at_top: bool,
    pub(crate) expanded_turn_headers: HashMap<String, bool>,
    pub(crate) expanded_messages: HashMap<String, bool>,
    pub(crate) expanded_turn_details: HashMap<String, bool>,
    pub(crate) expanded_tools: HashMap<String, bool>,
    pub(crate) turn_tools_loading: HashSet<TurnId>,
    pub(crate) verbosity: SessionViewVerbosity,
    #[allow(dead_code)]
    pub(crate) verbosity_menu_open: bool,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) artifacts_session_id: Option<SessionId>,
    pub(crate) selected_artifact: Option<usize>,
    pub(crate) artifact_preview: ArtifactPreviewState,
    pub(crate) session_events: Vec<SessionEvent>,
    pub(crate) session_thread_cache: HashMap<SessionId, SessionThreadCache>,
    pub(crate) session_summary_map: HashMap<SessionId, SessionSnapshotSummary>,
    pub(crate) session: SessionInfo,
    pub(crate) data_state: DataLoadState,
    pub(crate) new_task_mode: bool,
    pub(crate) new_task_mode_locked: bool,
    pub(crate) composer_new_input: Entity<InputState>,
    pub(crate) composer_session_input: Entity<InputState>,
    pub(crate) composer_attachments: Vec<MessageAttachment>,
    pub(crate) composer_new_attachments: Vec<MessageAttachment>,
    pub(crate) composer_session_attachments: HashMap<SessionId, Vec<MessageAttachment>>,
    pub(crate) composer_new_draft: ComposerDraft,
    pub(crate) composer_session_drafts: HashMap<SessionId, ComposerDraft>,
    pub(crate) composer_start_busy: bool,
    pub(crate) composer_start_error: Option<String>,
    pub(crate) composer_needs_apply: bool,
    pub(crate) composer_notice: Option<String>,
    pub(crate) composer_open_menu: Option<ComposerMenuId>,
    pub(crate) composer_menu_trigger_bounds: HashMap<ComposerMenuId, Bounds<Pixels>>,
    pub(crate) composer_menu_bounds: HashMap<ComposerMenuId, Bounds<Pixels>>,
    pub(crate) composer_menu_placements: HashMap<ComposerMenuId, PopoverPlacement>,
    pub(crate) composer_tooltip_open: Option<ComposerMenuId>,
    pub(crate) composer_tooltip_trigger_bounds: HashMap<ComposerMenuId, Bounds<Pixels>>,
    pub(crate) composer_tooltip_bounds: HashMap<ComposerMenuId, Bounds<Pixels>>,
    pub(crate) composer_tooltip_placements: HashMap<ComposerMenuId, PopoverPlacement>,
    pub(crate) composer_tooltip_close_id: u64,
    pub(crate) composer_mode_id: WorkbenchModeId,
    pub(crate) composer_verbosity: ComposerVerbosity,
    pub(crate) composer_context_window: Option<ContextWindowInfo>,
    pub(crate) composer_env_target: EnvTarget,
    pub(crate) composer_use_multiple_agents: bool,
    pub(crate) composer_draft_tracks: Vec<DraftTrack>,
    pub(crate) composer_provider_options: HashMap<String, ProviderOptions>,
    pub(crate) composer_provider_opts_busy: HashMap<String, bool>,
    pub(crate) composer_provider_auth_busy: HashMap<String, bool>,
    pub(crate) composer_provider_verify_busy: HashMap<String, bool>,
    pub(crate) composer_provider_installs: HashMap<String, ProviderInstallState>,
    pub(crate) composer_install_polling: HashSet<String>,
    pub(crate) composer_install_all_busy: bool,
    pub(crate) composer_provider_action_notice: Option<String>,
    pub(crate) composer_provider_action_error: Option<String>,
    pub(crate) composer_harness_expanded_provider: Option<String>,
    pub(crate) composer_harness_count_menu_provider: Option<String>,
    pub(crate) composer_harness_count_menu_placement: Option<PopoverPlacement>,
    pub(crate) composer_harness_count_trigger_bounds: HashMap<String, Bounds<Pixels>>,
    pub(crate) composer_harness_count_menu_bounds: Option<Bounds<Pixels>>,
    pub(crate) composer_recording: bool,
    pub(crate) composer_harness_search: Entity<InputState>,
    pub(crate) composer_model_search: Entity<InputState>,
    pub(crate) composer_model_search_placeholder: String,
    pub(crate) composer_model_manual: Entity<InputState>,
    pub(crate) composer_autocomplete: ComposerAutocompleteState,
    pub(crate) composer_track_model_inputs: HashMap<String, Entity<InputState>>,
    pub(crate) composer_track_model_placeholders: HashMap<String, String>,
    pub(crate) composer_track_input_subscriptions: HashMap<String, Subscription>,
    pub(crate) composer_autocomplete_input_bounds: Option<Bounds<Pixels>>,
    pub(crate) composer_autocomplete_anchor_bounds: Option<Bounds<Pixels>>,
    pub(crate) composer_autocomplete_menu_placement: Option<PopoverPlacement>,
    pub(crate) composer_autocomplete_menu_width: Option<Pixels>,
    pub(crate) composer_autocomplete_preview_placement: Option<PopoverPlacement>,
    pub(crate) composer_attachment_images: HashMap<String, Arc<Image>>,
    pub(crate) composer_attachment_loading: HashSet<String>,
    pub(crate) attachment_fetch_failed: HashSet<String>,
    pub(crate) composer_subscriptions: Vec<Subscription>,
    pub(crate) composer_subscriptions_set: bool,
    pub(crate) thread_list_state: ListState,
    pub(crate) thread_list_len: usize,
    pub(crate) thread_item_layout_hashes: HashMap<String, u64>,
    pub(crate) thread_auto_follow: bool,
    pub(crate) new_thread_item_count: usize,
    pub(crate) copied_flags: HashMap<String, Instant>,
    pub(crate) stream_status: StreamStatus,
    pub(crate) resyncing_session: Option<SessionId>,
    pub(crate) stream_subscribe_tx: Option<watch::Sender<WorkspaceActiveSnapshotClientMessage>>,
    pub(crate) stream_stop_tx: Option<watch::Sender<bool>>,
    pub(crate) session_last_event_seq: HashMap<SessionId, i64>,
    pub(crate) thread_list_handler_set: bool,
    pub(crate) show_sessions_pane: bool,
    pub(crate) show_diff_pane: bool,
    pub(crate) show_artifacts_pane: bool,
    pub(crate) show_terminal_panel: bool,
    pub(crate) diff_review_state: Entity<DiffReviewState>,
    pub(crate) terminal_panel_state: Entity<TerminalPanelState>,
    pub(crate) settings_state: Entity<SettingsState>,
    pub(crate) aux_workspace_id: Option<WorkspaceId>,
    pub(crate) aux_session_id: Option<SessionId>,
}

impl ShellView {
    #[allow(dead_code)]
    pub(crate) fn toggle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dark = !self.is_dark;
        let (tokens, colors) = super::load_theme(self.is_dark);
        self.colors = colors;
        crate::theme::apply_gpui_component_theme(&tokens, self.is_dark, cx);
        let colors = self.colors;
        cx.update_entity(&self.terminal_panel_state, |state, cx| {
            state.colors = colors;
            cx.notify();
        });
        cx.update_entity(&self.settings_state, |state, cx| {
            state.colors = colors;
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn set_route(&mut self, route: ShellRoute, cx: &mut Context<Self>) {
        if self.route == route {
            return;
        }
        self.route = route;
        if matches!(route, ShellRoute::Settings | ShellRoute::Providers) {
            cx.update_entity(&self.settings_state, |state, cx| {
                state.start_load(cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.sidebar_collapsed == collapsed {
            return;
        }
        self.sidebar_collapsed = collapsed;
        self.sidebar_anim_epoch = self.sidebar_anim_epoch.wrapping_add(1);
        if collapsed {
            self.sidebar_resizing = false;
            self.sidebar_resize_state = None;
            self.sidebar_resizer_hovered = false;
        }
        if let Some(workspace_id) = self.selected_workspace {
            self.ui_state
                .set_sidebar_collapsed(workspace_id, collapsed);
        }
        cx.notify();
    }

    pub(crate) fn set_archived_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.archived_collapsed == collapsed {
            return;
        }
        self.archived_collapsed = collapsed;
        if let Some(workspace_id) = self.selected_workspace {
            self.ui_state
                .set_archived_collapsed(workspace_id, collapsed);
        }
        cx.notify();
    }

    fn sessions_pane_scope(&self) -> Option<String> {
        self.selected_session_id().map(|id| id.0.to_string())
    }

    fn artifacts_pane_scope(&self) -> Option<String> {
        self.selected_session_id().map(|id| id.0.to_string())
    }

    fn diff_pane_scope(&self) -> Option<String> {
        if let Some(session_id) = self.selected_session_id() {
            return Some(format!("session:{}", session_id.0));
        }
        let worktree_id = self.selected_task.and_then(|task_id| {
            self.tasks_by_id.get(&task_id).and_then(|task| {
                task.task
                    .primary_worktree_id
                    .or_else(|| task.primary_session.as_ref().map(|session| session.session.worktree_id))
            })
        })?;
        Some(format!("worktree:{}", worktree_id.0))
    }

    pub(crate) fn hydrate_pane_state(&mut self) {
        let Some(workspace_id) = self.selected_workspace else {
            self.show_sessions_pane = false;
            self.show_diff_pane = false;
            self.show_artifacts_pane = false;
            self.show_terminal_panel = false;
            return;
        };

        self.show_sessions_pane = self
            .sessions_pane_scope()
            .and_then(|scope| self.ui_state.sessions_pane_open(workspace_id, &scope))
            .unwrap_or(false);
        self.show_diff_pane = self
            .diff_pane_scope()
            .and_then(|scope| self.ui_state.diff_pane_open(workspace_id, &scope))
            .unwrap_or(false);
        self.show_artifacts_pane = self
            .artifacts_pane_scope()
            .and_then(|scope| self.ui_state.artifacts_pane_open(workspace_id, &scope))
            .unwrap_or(false);
        self.show_terminal_panel = self
            .ui_state
            .terminal_panel_open(workspace_id)
            .unwrap_or(false);
    }

    fn persist_pane_state(&mut self) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if let Some(scope) = self.sessions_pane_scope() {
            self.ui_state
                .set_sessions_pane_open(workspace_id, &scope, self.show_sessions_pane);
        }
        if let Some(scope) = self.diff_pane_scope() {
            self.ui_state
                .set_diff_pane_open(workspace_id, &scope, self.show_diff_pane);
        }
        if let Some(scope) = self.artifacts_pane_scope() {
            self.ui_state
                .set_artifacts_pane_open(workspace_id, &scope, self.show_artifacts_pane);
        }
        self.ui_state
            .set_terminal_panel_open(workspace_id, self.show_terminal_panel);
    }

    #[cfg(feature = "automation")]
    #[allow(dead_code)]
    pub(crate) fn archived_ready(&self) -> bool {
        self.task_archived_loaded && self.task_fetch_archived != TaskFetchState::Loading
    }

    pub(crate) fn set_sidebar_width(
        &mut self,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let clamped = self.clamp_sidebar_width(width, window);
        if (self.sidebar_width - clamped).abs() < f32::EPSILON {
            return;
        }
        self.sidebar_width = clamped;
        if let Some(workspace_id) = self.selected_workspace {
            self.ui_state.set_sidebar_width(workspace_id, clamped);
        }
        cx.notify();
    }

    pub(crate) fn clamp_sidebar_width(&self, width: f32, window: &Window) -> f32 {
        let viewport = f32::from(window.bounds().size.width);
        let max = (viewport - 240.0).max(170.0);
        width.round().clamp(170.0, max)
    }

    pub(crate) fn start_relative_time(&mut self, cx: &mut Context<Self>) {
        if self.relative_time_task.is_some() {
            return;
        }
        let (tick_tx, mut tick_rx) = watch::channel(Utc::now());
        let tick_task = Tokio::spawn(cx, async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if tick_tx.send(Utc::now()).is_err() {
                    break;
                }
            }
        });
        tick_task.detach();

        let update_task = cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while tick_rx.changed().await.is_ok() {
                    let now = *tick_rx.borrow();
                    if this
                        .update(&mut cx, |view, cx| {
                            view.relative_now = now;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
        self.relative_time_task = Some(update_task);
    }

    pub(crate) fn toggle_sessions_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.show_sessions_pane;
        self.show_sessions_pane = next;
        if next {
            self.show_diff_pane = false;
            self.show_artifacts_pane = false;
        }
        self.persist_pane_state();
        cx.notify();
    }

    pub(crate) fn toggle_diff_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.show_diff_pane;
        self.show_diff_pane = next;
        if next {
            self.show_sessions_pane = false;
        }
        self.persist_pane_state();
        cx.notify();
    }

    pub(crate) fn toggle_artifacts_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.show_artifacts_pane;
        self.show_artifacts_pane = next;
        if next {
            self.show_sessions_pane = false;
            if let Some(session_id) = self.selected_session_id() {
                self.load_session_artifacts(session_id, cx);
            }
        }
        self.persist_pane_state();
        cx.notify();
    }

    pub(crate) fn toggle_terminal_panel(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_terminal_panel = !self.show_terminal_panel;
        self.persist_pane_state();
        cx.notify();
    }
    pub(crate) fn sync_auxiliary_panes(&mut self, cx: &mut Context<Self>) {
        let selected_session_id = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id);
        if self.aux_workspace_id == self.selected_workspace
            && self.aux_session_id == selected_session_id
        {
            return;
        }

        self.aux_workspace_id = self.selected_workspace;
        self.aux_session_id = selected_session_id;

        let (task_id, worktree_id) = selected_session_id
            .and_then(|session_id| self.session_summary_map.get(&session_id))
            .map(|summary| {
                (
                    Some(summary.session.task_id),
                    Some(summary.session.worktree_id),
                )
            })
            .unwrap_or((None, None));
        let worktree_id = worktree_id.or_else(|| {
            self.selected_task.and_then(|task_id| {
                self.tasks_by_id.get(&task_id).and_then(|task| {
                    task.task.primary_worktree_id.or_else(|| {
                        task.primary_session
                            .as_ref()
                            .map(|session| session.session.worktree_id)
                    })
                })
            })
        });

        let terminal_context = TerminalContext {
            workspace_id: self.selected_workspace,
            task_id,
            session_id: selected_session_id,
            worktree_id,
        };

        cx.update_entity(&self.terminal_panel_state, |state, cx| {
            state.set_context(terminal_context, cx);
        });
        cx.update_entity(&self.diff_review_state, |state, cx| {
            state.set_session_id(selected_session_id, cx);
        });
    }
}
