pub(super) mod artifacts;
pub(super) mod composer;
pub(super) mod diff_review;
pub(super) mod session;
pub(super) mod settings;
pub(super) mod stream;
pub(super) mod terminal;
pub(super) mod turn_tools;
pub(super) mod workspace;

use std::collections::HashMap;

use gpui::AppContext as _;
use gpui::{ClickEvent, Context, FocusHandle, ListState, Window, Entity};
use tokio::sync::watch;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    Artifact, MessageAttachment, SessionCatchupSummary, SessionEvent,
    WorkspaceCatchupClientMessage,
};

use crate::theme::ThemeColors;

use super::models::{MessageItem, SessionInfo};
use super::workspace_summary::{SessionSummaryItem, TaskSummaryItem};

pub(crate) use artifacts::ArtifactPreviewState;
pub(crate) use composer::ComposerState;
pub(crate) use diff_review::DiffReviewState;
pub(crate) use settings::SettingsState;
pub(crate) use stream::StreamStatus;
pub(crate) use terminal::{TerminalContext, TerminalPanelState};
pub(crate) use workspace::{DataLoadState, ProviderItem, WorkspaceItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellRoute {
    Workbench,
    Workspaces,
    Settings,
    Providers,
    Diagnostics,
    AppSettings,
}

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    pub(crate) is_dark: bool,
    pub(crate) route: ShellRoute,
    pub(crate) workspaces: Vec<WorkspaceItem>,
    pub(crate) selected_workspace: Option<WorkspaceId>,
    pub(crate) providers: Vec<ProviderItem>,
    pub(crate) composer_provider_id: Option<String>,
    pub(crate) composer_model_id: Option<String>,
    pub(crate) composer_provider_menu_open: bool,
    pub(crate) composer_model_menu_open: bool,
    pub(crate) catchup_active_total: Option<i64>,
    pub(crate) catchup_archived_total: Option<i64>,
    pub(crate) tasks: Vec<TaskSummaryItem>,
    pub(crate) selected_task: Option<usize>,
    pub(crate) sessions: Vec<SessionSummaryItem>,
    pub(crate) selected_session: Option<usize>,
    pub(crate) messages: Vec<MessageItem>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) selected_artifact: Option<usize>,
    pub(crate) artifact_preview: ArtifactPreviewState,
    pub(crate) session_events: Vec<SessionEvent>,
    pub(crate) session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    pub(crate) session: SessionInfo,
    pub(crate) data_state: DataLoadState,
    pub(crate) composer: ComposerState,
    pub(crate) composer_focus: FocusHandle,
    pub(crate) composer_attachments: Vec<MessageAttachment>,
    pub(crate) composer_attachment_input: ComposerState,
    pub(crate) composer_attachment_focus: FocusHandle,
    pub(crate) composer_notice: Option<String>,
    pub(crate) message_list_state: ListState,
    pub(crate) message_list_len: usize,
    pub(crate) message_auto_follow: bool,
    pub(crate) new_message_count: usize,
    pub(crate) stream_status: StreamStatus,
    pub(crate) resyncing_session: Option<SessionId>,
    pub(crate) stream_subscribe_tx: Option<watch::Sender<WorkspaceCatchupClientMessage>>,
    pub(crate) stream_stop_tx: Option<watch::Sender<bool>>,
    pub(crate) session_last_event_seq: HashMap<SessionId, i64>,
    pub(crate) message_list_handler_set: bool,
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
    pub(crate) fn toggle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dark = !self.is_dark;
        self.colors = super::load_theme_colors(self.is_dark);
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

    pub(crate) fn toggle_sessions_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_sessions_pane = !self.show_sessions_pane;
        cx.notify();
    }

    pub(crate) fn toggle_diff_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_diff_pane = !self.show_diff_pane;
        cx.notify();
    }

    pub(crate) fn toggle_artifacts_pane(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_artifacts_pane = !self.show_artifacts_pane;
        cx.notify();
    }

    pub(crate) fn toggle_terminal_panel(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_terminal_panel = !self.show_terminal_panel;
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

        let (task_id, track_id, worktree_id) = selected_session_id
            .and_then(|session_id| self.session_summary_map.get(&session_id))
            .map(|summary| {
                (
                    Some(summary.session.task_id),
                    Some(summary.session.track_id),
                    Some(summary.session.worktree_id),
                )
            })
            .unwrap_or((None, None, None));

        let terminal_context = TerminalContext {
            workspace_id: self.selected_workspace,
            task_id,
            track_id,
            session_id: selected_session_id,
            worktree_id,
        };

        cx.update_entity(&self.terminal_panel_state, |state, cx| {
            state.set_context(terminal_context, cx);
        });
        cx.update_entity(&self.diff_review_state, |state, cx| {
            state.set_track_id(track_id, cx);
        });
    }
}
