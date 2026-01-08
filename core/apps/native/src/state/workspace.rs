use std::collections::HashMap;

use gpui::Context;
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    Artifact, SessionCatchupSummary, SessionEvent, SessionHead, SessionHistoryPage,
};

use super::{ArtifactPreviewState, ShellView, StreamStatus};
use super::super::models::{build_message_items, session_info_from_head, session_info_from_summary, MessageItem, SessionInfo};
use super::super::workspace_summary::{
    catchup_counts,
    session_summaries as build_session_summaries,
    task_summaries as build_task_summaries,
    SessionSummaryItem,
    TaskSummaryItem,
};

#[derive(Clone)]
pub(crate) struct WorkspaceItem {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
}

#[derive(Clone)]
pub(crate) struct ProviderItem {
    pub(crate) name: String,
    pub(crate) status: String,
}

struct InitialLoadResult {
    workspaces: Vec<WorkspaceItem>,
    providers: Vec<ProviderItem>,
}

struct WorkspaceLoadResult {
    catchup_active_total: Option<i64>,
    catchup_archived_total: Option<i64>,
    catchup_tasks: Vec<TaskSummaryItem>,
    catchup_sessions: Vec<SessionSummaryItem>,
    artifacts: Vec<Artifact>,
    session_summaries: Vec<SessionCatchupSummary>,
    session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    session_events: Vec<SessionEvent>,
}

pub(crate) enum DataLoadState {
    Loading,
    Loaded,
    Error(String),
}

impl ShellView {
    pub(crate) fn select_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if self.selected_workspace == Some(workspace.id) {
            return;
        }
        self.load_workspace(workspace.id, cx);
    }

    pub(crate) fn start_data_load(&mut self, cx: &mut Context<Self>) {
        self.ensure_message_list_handler(cx);
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...");
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let workspaces = client.list_workspaces().await?;
            let providers = client.list_providers().await.unwrap_or_default();
            let workspaces = workspaces
                .into_iter()
                .map(|workspace| WorkspaceItem {
                    id: workspace.id,
                    name: workspace.name,
                })
                .collect::<Vec<_>>();
            let providers = providers
                .into_iter()
                .map(|provider| ProviderItem {
                    name: provider.provider_id,
                    status: format!("{:?}", provider.health),
                })
                .collect::<Vec<_>>();
            Ok(InitialLoadResult {
                workspaces,
                providers,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(data) => {
                        view.workspaces = data.workspaces;
                        view.providers = data.providers;
                        view.sync_composer_defaults();
                        if let Some(selected) = view.selected_workspace {
                            if !view.workspaces.iter().any(|ws| ws.id == selected) {
                                view.selected_workspace = None;
                            }
                        }
                        if view.selected_workspace.is_none() {
                            view.selected_workspace =
                                view.workspaces.first().map(|workspace| workspace.id);
                        }
                        if let Some(workspace_id) = view.selected_workspace {
                            view.load_workspace(workspace_id, cx);
                        } else {
                            view.reset_workspace_view("No workspaces yet.");
                            view.data_state = DataLoadState::Loaded;
                            cx.notify();
                        }
                    }
                    Err(err) => {
                        view.workspaces.clear();
                        view.providers.clear();
                        view.selected_workspace = None;
                        view.reset_workspace_view("Unable to load workspace list.");
                        view.data_state = DataLoadState::Error(err.to_string());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn load_workspace(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        self.selected_workspace = Some(workspace_id);
        self.stop_workspace_stream();
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...");
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceCatchupParams {
                limit: Some(20),
                ..Default::default()
            };
            let snapshot = client.get_workspace_catchup(workspace_id, &params).await?;
            let counts = catchup_counts(&snapshot);
            let catchup_active_total = Some(counts.active_total);
            let catchup_archived_total = counts.archived_total;
            let catchup_tasks = build_task_summaries(&snapshot);
            let catchup_sessions = build_session_summaries(&snapshot);
            let mut session_summaries = Vec::new();
            let mut session_summary_map = HashMap::new();
            let mut first_session_id = None;
            for task in &snapshot.active.tasks {
                for track in &task.tracks {
                    for session in &track.sessions {
                        if first_session_id.is_none() {
                            first_session_id = Some(session.session.id);
                        }
                        session_summaries.push(session.clone());
                        session_summary_map.insert(session.session.id, session.clone());
                    }
                }
            }
            let mut session_head = None;
            let mut session_history = None;
            let mut session_events = Vec::new();
            let mut artifacts = Vec::new();
            if let Some(session_id) = first_session_id {
                session_head = client
                    .get_session_head(session_id, Some(40), Some(false))
                    .await
                    .ok();
                session_history = client
                    .get_session_history(session_id, None, Some(60))
                    .await
                    .ok();
                session_events = client
                    .get_session_events(session_id, None, None, Some(40))
                    .await
                    .ok()
                    .map(|page| page.events)
                    .unwrap_or_default();
                artifacts = client
                    .list_session_artifacts(session_id)
                    .await
                    .ok()
                    .map(|items| items.into_iter().collect::<Vec<_>>())
                    .unwrap_or_default();
            }

            Ok(WorkspaceLoadResult {
                catchup_active_total,
                catchup_archived_total,
                catchup_tasks,
                catchup_sessions,
                artifacts,
                session_summaries,
                session_summary_map,
                session_head,
                session_history,
                session_events,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.selected_workspace != Some(workspace_id) {
                    return;
                }
                match result {
                    Ok(data) => {
                        view.tasks = data.catchup_tasks;
                        view.selected_task = if view.tasks.is_empty() { None } else { Some(0) };
                        view.sessions = data.catchup_sessions;
                        view.selected_session =
                            if view.sessions.is_empty() { None } else { Some(0) };
                        view.session_summary_map = data.session_summary_map;
                        view.session_last_event_seq.clear();
                        for summary in &data.session_summaries {
                            if let Some(seq) = summary.last_event_seq {
                                view.update_session_last_event_seq(summary.session.id, seq);
                            }
                        }
                        view.catchup_active_total = data.catchup_active_total;
                        view.catchup_archived_total = data.catchup_archived_total;
                        view.session = data
                            .session_head
                            .as_ref()
                            .map(session_info_from_head)
                            .or_else(|| data.session_summaries.first().map(session_info_from_summary))
                            .unwrap_or_else(SessionInfo::placeholder);
                        view.sync_composer_defaults();
                        let mut messages = build_message_items(
                            data.session_head.as_ref(),
                            data.session_history.as_ref(),
                        );
                        if messages.is_empty() {
                            messages.push(MessageItem::new(
                                "assistant",
                                "No messages yet. Create one to begin.",
                            ));
                        }
                        view.replace_messages(messages);
                        view.session_events = data.session_events;
                        if let Some(head) = data.session_head.as_ref() {
                            view.update_session_last_event_seq(head.session.id, head.last_event_seq);
                        } else if let Some(event) = view.session_events.last() {
                            view.update_session_last_event_seq(event.session_id, event.seq);
                        }
                        view.artifacts = data.artifacts;
                        view.selected_artifact = if view.artifacts.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                        view.load_artifact_preview(cx);
                        view.data_state = DataLoadState::Loaded;
                        view.start_workspace_stream(workspace_id, cx);
                    }
                    Err(err) => {
                        view.reset_workspace_view("Unable to load workspace data.");
                        view.data_state = DataLoadState::Error(err.to_string());
                        view.stream_status = StreamStatus::Idle;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reset_workspace_view(&mut self, message: &str) {
        self.tasks.clear();
        self.selected_task = None;
        self.sessions.clear();
        self.selected_session = None;
        self.replace_messages(vec![MessageItem::new("assistant", message)]);
        self.artifacts.clear();
        self.artifact_preview = ArtifactPreviewState::None;
        self.session_events.clear();
        self.selected_artifact = None;
        self.composer_attachments.clear();
        self.composer_attachment_input.clear();
        self.composer_notice = None;
        self.composer_provider_menu_open = false;
        self.composer_model_menu_open = false;
        self.session_summary_map.clear();
        self.session_last_event_seq.clear();
        self.resyncing_session = None;
        self.session = SessionInfo::placeholder();
        self.catchup_active_total = None;
        self.catchup_archived_total = None;
    }
}
