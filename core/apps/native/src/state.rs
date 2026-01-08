use std::collections::HashMap;

use gpui::{ClickEvent, Context, FocusHandle, KeyDownEvent, Window};
use gpui_tokio::Tokio;

use ctx_core::ids::SessionId;
use ctx_core::models::{SessionCatchupSummary, SessionHead, SessionHistoryPage};

use crate::theme::ThemeColors;

use super::models::{
    artifact_label, build_message_items, session_info_from_head, session_info_from_summary,
    MessageItem, SessionInfo,
};
use super::workspace_summary::{
    catchup_counts,
    session_summaries as build_session_summaries,
    task_summaries as build_task_summaries,
    SessionSummaryItem,
    TaskSummaryItem,
};

pub(crate) struct AppData {
    pub(crate) workspace_count: usize,
    pub(crate) workspace_names: Vec<String>,
    pub(crate) selected_workspace: Option<String>,
    pub(crate) catchup_active_total: Option<i64>,
    pub(crate) catchup_archived_total: Option<i64>,
    pub(crate) catchup_tasks: Vec<TaskSummaryItem>,
    pub(crate) catchup_sessions: Vec<SessionSummaryItem>,
    pub(crate) artifact_names: Vec<String>,
}

struct LoadedData {
    app_data: AppData,
    session_summaries: Vec<SessionCatchupSummary>,
    session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
}

struct SessionLoadResult {
    session_id: SessionId,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    artifacts: Vec<String>,
}

pub(crate) enum DataLoadState {
    Loading,
    Loaded(AppData),
    Error(String),
}

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    pub(crate) is_dark: bool,
    pub(crate) tasks: Vec<TaskSummaryItem>,
    pub(crate) selected_task: Option<usize>,
    pub(crate) sessions: Vec<SessionSummaryItem>,
    pub(crate) selected_session: Option<usize>,
    pub(crate) messages: Vec<MessageItem>,
    pub(crate) artifacts: Vec<String>,
    pub(crate) session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    pub(crate) session: SessionInfo,
    pub(crate) data_state: DataLoadState,
    pub(crate) composer_text: String,
    pub(crate) composer_focus: FocusHandle,
}

impl ShellView {
    pub(crate) fn toggle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dark = !self.is_dark;
        self.colors = super::load_theme_colors(self.is_dark);
        cx.notify();
    }

    pub(crate) fn focus_composer(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.composer_focus.focus(window);
    }

    pub(crate) fn on_composer_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "backspace" {
            self.composer_text.pop();
            cx.notify();
            return;
        }

        if event.keystroke.key == "enter" || event.keystroke.key == "tab" {
            return;
        }

        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }

        if let Some(text) = event.keystroke.key_char.as_ref() {
            if text != "\n" && text != "\r" && text != "\t" {
                self.composer_text.push_str(text);
                cx.notify();
            }
        }
    }

    pub(crate) fn on_send_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.send_composer_message(cx);
    }

    fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let content = self.composer_text.trim().to_string();
        if content.is_empty() {
            return;
        }

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let request = ctx_client::PostMessageRequest {
                content,
                delivery: None,
                attachments: Vec::new(),
            };
            client.post_message(session_id, &request).await?;
            Ok(session_id)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(session_id) => {
                        view.composer_text.clear();
                        view.load_session_details(session_id, cx);
                    }
                    Err(_) => {
                        view.messages.push(MessageItem::new(
                            "assistant",
                            "Unable to send message.",
                        ));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn selected_session_id(&self) -> Option<SessionId> {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
    }

    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(summary) = self.sessions.get(index) else {
            return;
        };
        let session_id = summary.session_id;
        self.selected_session = Some(index);
        self.session = self
            .session_summary_map
            .get(&session_id)
            .map(session_info_from_summary)
            .unwrap_or_else(SessionInfo::placeholder);
        self.messages = vec![MessageItem::new(
            "assistant",
            "Loading session messages...",
        )];
        self.artifacts.clear();
        cx.notify();
        self.load_session_details(session_id, cx);
    }

    fn load_session_details(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let session_head = client
                .get_session_head(session_id, Some(40), Some(false))
                .await
                .ok();
            let session_history = client
                .get_session_history(session_id, None, Some(60))
                .await
                .ok();
            let artifacts = client
                .list_session_artifacts(session_id)
                .await
                .ok()
                .map(|items| {
                    items
                        .into_iter()
                        .map(artifact_label)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            Ok(SessionLoadResult {
                session_id,
                session_head,
                session_history,
                artifacts,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if !view.is_session_selected(session_id) {
                    return;
                }
                match result {
                    Ok(data) => {
                        view.session = data
                            .session_head
                            .as_ref()
                            .map(session_info_from_head)
                            .or_else(|| {
                                view.session_summary_map
                                    .get(&data.session_id)
                                    .map(session_info_from_summary)
                            })
                            .unwrap_or_else(SessionInfo::placeholder);
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
                        view.messages = messages;
                        view.artifacts = data.artifacts;
                    }
                    Err(_) => {
                        view.messages = vec![MessageItem::new(
                            "assistant",
                            "Unable to load session details.",
                        )];
                        view.artifacts.clear();
                        if let Some(summary) = view.session_summary_map.get(&session_id) {
                            view.session = session_info_from_summary(summary);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn is_session_selected(&self, session_id: SessionId) -> bool {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
            == Some(session_id)
    }

    pub(crate) fn start_data_load(&mut self, cx: &mut Context<Self>) {
        self.data_state = DataLoadState::Loading;
        cx.notify();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let workspaces = client.list_workspaces().await?;
            let workspace_count = workspaces.len();
            let workspace_names = workspaces
                .iter()
                .map(|workspace| workspace.name.clone())
                .collect::<Vec<_>>();

            let mut selected_workspace = None;
            let mut catchup_active_total = None;
            let mut catchup_archived_total = None;
            let mut catchup_tasks = Vec::new();
            let mut catchup_sessions = Vec::new();
            let mut session_summaries = Vec::new();
            let mut session_summary_map = HashMap::new();
            let mut session_head = None;
            let mut session_history = None;
            let mut artifacts = Vec::new();

            if let Some(workspace) = workspaces.first() {
                selected_workspace = Some(workspace.name.clone());
                let params = ctx_client::WorkspaceCatchupParams {
                    limit: Some(20),
                    ..Default::default()
                };
                let snapshot = client
                    .get_workspace_catchup(workspace.id, &params)
                    .await?;
                let counts = catchup_counts(&snapshot);
                catchup_active_total = Some(counts.active_total);
                catchup_archived_total = counts.archived_total;
                catchup_tasks = build_task_summaries(&snapshot);
                catchup_sessions = build_session_summaries(&snapshot);
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
                if let Some(session_id) = first_session_id {
                    session_head = client
                        .get_session_head(session_id, Some(40), Some(false))
                        .await
                        .ok();
                    session_history = client
                        .get_session_history(session_id, None, Some(60))
                        .await
                        .ok();
                    artifacts = client
                        .list_session_artifacts(session_id)
                        .await
                        .ok()
                        .map(|items| {
                            items
                                .into_iter()
                                .map(artifact_label)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                }
            }

            Ok(LoadedData {
                app_data: AppData {
                    workspace_count,
                    workspace_names,
                    selected_workspace,
                    catchup_active_total,
                    catchup_archived_total,
                    catchup_tasks,
                    catchup_sessions,
                    artifact_names: artifacts,
                },
                session_summaries,
                session_summary_map,
                session_head,
                session_history,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(data) => {
                        view.tasks = data.app_data.catchup_tasks.clone();
                        view.selected_task = if view.tasks.is_empty() { None } else { Some(0) };
                        view.sessions = data.app_data.catchup_sessions.clone();
                        view.selected_session = if view.sessions.is_empty() { None } else { Some(0) };
                        view.session_summary_map = data.session_summary_map.clone();
                        view.session = data
                            .session_head
                            .as_ref()
                            .map(session_info_from_head)
                            .or_else(|| data.session_summaries.first().map(session_info_from_summary))
                            .unwrap_or_else(SessionInfo::placeholder);
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
                        view.messages = messages;
                        view.artifacts = data.app_data.artifact_names.clone();
                        view.data_state = DataLoadState::Loaded(data.app_data);
                    }
                    Err(err) => {
                        view.tasks.clear();
                        view.selected_task = None;
                        view.messages = vec![MessageItem::new(
                            "assistant",
                            "Unable to load workspace data.",
                        )];
                        view.artifacts.clear();
                        view.data_state = DataLoadState::Error(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
