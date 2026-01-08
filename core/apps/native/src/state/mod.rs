mod artifacts;
mod composer;
mod session;
mod stream;
mod workspace;

use std::collections::HashMap;

use gpui::{ClickEvent, Context, FocusHandle, ListState, Window};
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
pub(crate) use stream::StreamStatus;
pub(crate) use workspace::{DataLoadState, ProviderItem, WorkspaceItem};

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    pub(crate) is_dark: bool,
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
}

impl ShellView {
    pub(crate) fn toggle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dark = !self.is_dark;
        self.colors = super::load_theme_colors(self.is_dark);
        cx.notify();
    }
}
