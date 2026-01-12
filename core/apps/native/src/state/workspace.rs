use std::collections::HashMap;

use gpui::{AppContext as _, AsyncApp, ClickEvent, Context, WeakEntity, Window};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    Artifact, SessionCatchupSummary, SessionEvent, SessionHead, SessionHistoryPage, Task,
    WorkspaceCatchupSnapshot, WorkspaceCatchupTaskSummary, WorkspaceCatchupTrackSummary,
};
use ctx_providers::adapters::ProviderStatus;

use super::{AnchorRect, ArchiveConfirmState, ArtifactPreviewState, ShellView, StreamStatus, TaskArchiveAction, TaskMenuState};
use super::super::models::{
    build_message_items, session_info_from_head, session_info_from_summary, MessageItem,
    SessionInfo,
};
use super::super::workspace_summary::{catchup_counts, task_session_summaries, TaskSummaryItem};

#[derive(Clone)]
pub(crate) struct WorkspaceItem {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
}

pub(crate) type ProviderItem = ProviderStatus;

struct InitialLoadResult {
    workspaces: Vec<WorkspaceItem>,
    providers: Vec<ProviderItem>,
}

struct WorkspaceLoadResult {
    snapshot: WorkspaceCatchupSnapshot,
    artifacts: Vec<Artifact>,
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
    pub(crate) fn focus_new_task(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_task_focus("Select a task to begin.");
        cx.notify();
    }

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
            Ok(InitialLoadResult {
                workspaces,
                providers,
            })
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
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
            }
        })
        .detach();
    }

    pub(crate) fn load_workspace(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        self.apply_workspace_ui_state(workspace_id);
        self.selected_workspace = Some(workspace_id);
        self.stop_workspace_stream();
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...");
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceCatchupParams {
                limit: Some(50),
                include_archived: Some(false),
                ..Default::default()
            };
            let snapshot = client.get_workspace_catchup(workspace_id, &params).await?;
            let mut first_session_id = None;
            for task in &snapshot.active.tasks {
                for track in &task.tracks {
                    if let Some(session) = track.sessions.first() {
                        first_session_id = Some(session.session.id);
                        break;
                    }
                }
                if first_session_id.is_some() {
                    break;
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
                snapshot,
                artifacts,
                session_head,
                session_history,
                session_events,
            })
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                if view.selected_workspace != Some(workspace_id) {
                    return;
                }
                match result {
                    Ok(data) => {
                        view.apply_workspace_snapshot(data.snapshot);
                        let keep_new_task = view.new_task_mode_locked;
                        let selected_session_id = if keep_new_task {
                            None
                        } else {
                            data.session_head
                                .as_ref()
                                .map(|head| head.session.id)
                                .or_else(|| view.sessions.first().map(|summary| summary.session_id))
                        };
                        view.selected_session = selected_session_id
                            .and_then(|id| view.sessions.iter().position(|summary| summary.session_id == id));
                        view.selected_task = if keep_new_task {
                            None
                        } else {
                            view.selected_session
                                .and_then(|index| view.sessions.get(index))
                                .and_then(|summary| view.session_summary_map.get(&summary.session_id))
                                .map(|summary| summary.session.task_id)
                                .or_else(|| view.task_active_order.first().copied())
                        };
                        view.new_task_mode = keep_new_task || view.selected_session.is_none();
                        view.composer_needs_apply = true;
                        view.session = if keep_new_task {
                            SessionInfo::placeholder()
                        } else {
                            data.session_head
                                .as_ref()
                                .map(session_info_from_head)
                                .or_else(|| {
                                    view.selected_session
                                        .and_then(|index| view.sessions.get(index))
                                        .and_then(|summary| {
                                            view.session_summary_map
                                                .get(&summary.session_id)
                                                .map(session_info_from_summary)
                                        })
                                })
                                .unwrap_or_else(SessionInfo::placeholder)
                        };
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
            }
        })
        .detach();
    }

    fn reset_workspace_view(&mut self, message: &str) {
        self.task_store_initialized = false;
        self.task_fetch_active = super::TaskFetchState::Idle;
        self.task_fetch_archived = super::TaskFetchState::Idle;
        self.task_has_more_active = true;
        self.task_has_more_archived = false;
        self.task_archived_loaded = false;
        self.task_active_cursor = None;
        self.task_archived_cursor = None;
        self.tasks_by_id.clear();
        self.task_active_order.clear();
        self.task_archived_order.clear();
        self.task_query.clear();
        self.selected_task = None;
        self.sessions.clear();
        self.selected_session = None;
        self.replace_messages(vec![MessageItem::new("assistant", message)]);
        self.artifacts.clear();
        self.artifact_preview = ArtifactPreviewState::None;
        self.session_events.clear();
        self.selected_artifact = None;
        self.composer_attachments.clear();
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

impl ShellView {
    fn apply_workspace_ui_state(&mut self, workspace_id: WorkspaceId) {
        if let Some(width) = self.ui_state.sidebar_width(workspace_id) {
            self.sidebar_width = width;
        } else {
            self.sidebar_width = 260.0;
        }
        if let Some(collapsed) = self.ui_state.sidebar_collapsed(workspace_id) {
            self.sidebar_collapsed = collapsed;
        } else {
            self.sidebar_collapsed = false;
        }
        if let Some(collapsed) = self.ui_state.archived_collapsed(workspace_id) {
            self.archived_collapsed = collapsed;
        } else {
            self.archived_collapsed = true;
        }
    }

    fn apply_workspace_snapshot(&mut self, snapshot: WorkspaceCatchupSnapshot) {
        let counts = catchup_counts(&snapshot);
        self.catchup_active_total = Some(counts.active_total);
        self.catchup_archived_total = counts.archived_total;

        self.task_store_initialized = true;
        self.task_fetch_active = super::TaskFetchState::Idle;
        self.task_fetch_archived = super::TaskFetchState::Idle;
        let next_cursor = snapshot.active.next_cursor;
        self.task_has_more_active = next_cursor.is_some();
        self.task_active_cursor = next_cursor;
        self.task_archived_cursor = None;
        self.task_has_more_archived = false;
        self.task_archived_loaded = false;
        self.tasks_by_id.clear();
        self.task_active_order.clear();
        self.task_archived_order.clear();

        for summary in snapshot.active.tasks {
            let item = TaskSummaryItem::from_summary(&summary);
            self.tasks_by_id.insert(item.id, item);
        }
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn load_more_active_tasks(&mut self, cx: &mut Context<Self>) {
        if self.task_fetch_active == super::TaskFetchState::Loading {
            return;
        }
        if !self.task_has_more_active {
            return;
        }
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        let cursor = self.task_active_cursor.clone();
        self.task_fetch_active = super::TaskFetchState::Loading;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceCatchupParams {
                limit: Some(50),
                active_cursor: cursor,
                include_archived: Some(false),
                ..Default::default()
            };
            let snapshot = client.get_workspace_catchup(workspace_id, &params).await?;
            Ok(snapshot)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, _cx| match result {
                    Ok(snapshot) => {
                        let next_cursor = snapshot.active.next_cursor;
                        view.task_has_more_active = next_cursor.is_some();
                        view.task_active_cursor = next_cursor;
                        view.catchup_active_total = Some(snapshot.active.total_count);
                        for summary in snapshot.active.tasks {
                            view.upsert_task_summary(summary);
                        }
                        view.task_fetch_active = super::TaskFetchState::Idle;
                    }
                    Err(_) => {
                        view.task_fetch_active = super::TaskFetchState::Error;
                    }
                })
                .ok();
                let _ = this.update(&mut cx, |_, cx| {
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn ensure_archived_loaded(&mut self, cx: &mut Context<Self>) {
        if self.task_archived_loaded {
            return;
        }
        if self.task_fetch_archived == super::TaskFetchState::Loading {
            return;
        }
        self.load_more_archived_tasks(true, cx);
    }

    pub(crate) fn load_more_archived_tasks(&mut self, reset: bool, cx: &mut Context<Self>) {
        if self.task_fetch_archived == super::TaskFetchState::Loading {
            return;
        }
        if !reset && !self.task_has_more_archived {
            return;
        }
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        let cursor = if reset {
            None
        } else {
            self.task_archived_cursor.clone()
        };
        self.task_fetch_archived = super::TaskFetchState::Loading;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceCatchupParams {
                limit: Some(50),
                archived_only: Some(true),
                archived_cursor: cursor,
                ..Default::default()
            };
            let snapshot = client.get_workspace_catchup(workspace_id, &params).await?;
            Ok(snapshot)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, _cx| match result {
                    Ok(snapshot) => {
                        if let Some(page) = snapshot.archived {
                            let next_cursor = page.next_cursor;
                            view.task_has_more_archived = next_cursor.is_some();
                            view.task_archived_cursor = next_cursor;
                            view.task_archived_loaded = true;
                            view.catchup_archived_total = Some(page.total_count);
                            for summary in page.tasks {
                                view.upsert_task_summary(summary);
                            }
                        }
                        view.task_fetch_archived = super::TaskFetchState::Idle;
                    }
                    Err(_) => {
                        view.task_fetch_archived = super::TaskFetchState::Error;
                    }
                })
                .ok();
                let _ = this.update(&mut cx, |_, cx| {
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn upsert_task_summary(&mut self, summary: WorkspaceCatchupTaskSummary) {
        let item = TaskSummaryItem::from_summary(&summary);
        let existing = self.tasks_by_id.insert(item.id, item.clone());
        if let Some(prev) = existing.as_ref() {
            self.update_task_counts_for_move(prev, &item);
        } else if item.is_archived() {
            if let Some(total) = self.catchup_archived_total.as_mut() {
                *total += 1;
            }
        } else if let Some(total) = self.catchup_active_total.as_mut() {
            *total += 1;
        }
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn apply_task_update(&mut self, task: Task) {
        let Some(existing) = self.tasks_by_id.get(&task.id).cloned() else {
            return;
        };
        let updated = existing.with_task(task);
        self.update_task_counts_for_move(&existing, &updated);
        self.tasks_by_id.insert(updated.id, updated);
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn remove_task(&mut self, task_id: TaskId) {
        let Some(existing) = self.tasks_by_id.remove(&task_id) else {
            return;
        };
        self.task_active_order.retain(|id| *id != task_id);
        self.task_archived_order.retain(|id| *id != task_id);
        if existing.is_archived() {
            if let Some(total) = self.catchup_archived_total.as_mut() {
                *total = total.saturating_sub(1);
            }
        } else if let Some(total) = self.catchup_active_total.as_mut() {
            *total = total.saturating_sub(1);
        }
        if self.selected_task == Some(task_id) {
            self.selected_task = None;
        }
        self.rebuild_session_state();
    }

    pub(crate) fn apply_track_summary(&mut self, summary: WorkspaceCatchupTrackSummary) {
        let task_id = summary.track.task_id;
        let Some(task) = self.tasks_by_id.get(&task_id).cloned() else {
            return;
        };
        let mut next_tracks = task.tracks.clone();
        let idx = next_tracks
            .iter()
            .position(|track| track.track.id == summary.track.id);
        if let Some(idx) = idx {
            next_tracks[idx] = summary;
        } else {
            next_tracks.push(summary);
        }
        next_tracks.sort_by_key(|track| track.track.created_at);
        let updated = TaskSummaryItem {
            tracks: next_tracks,
            ..task
        };
        self.tasks_by_id.insert(task_id, updated);
        self.rebuild_session_state();
    }

    pub(crate) fn apply_session_summary(&mut self, summary: SessionCatchupSummary) {
        let task_id = summary.session.task_id;
        let Some(task) = self.tasks_by_id.get(&task_id).cloned() else {
            return;
        };
        let track_id = summary.session.track_id;
        let mut next_tracks = task.tracks.clone();
        if let Some(track_idx) = next_tracks.iter().position(|track| track.track.id == track_id) {
            let mut track = next_tracks[track_idx].clone();
            let mut sessions = track.sessions.clone();
            let session_idx = sessions
                .iter()
                .position(|session| session.session.id == summary.session.id);
            if let Some(session_idx) = session_idx {
                sessions[session_idx] = summary;
            } else {
                sessions.push(summary);
            }
            sessions.sort_by_key(|session| session.session.created_at);
            track.sessions = sessions;
            next_tracks[track_idx] = track;
            let updated = TaskSummaryItem {
                tracks: next_tracks,
                ..task
            };
            self.tasks_by_id.insert(task_id, updated);
            self.rebuild_session_state();
        }
    }

    pub(crate) fn focus_task(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.tasks_by_id.get(&task_id).cloned() else {
            return;
        };
        self.new_task_mode_locked = false;
        self.selected_task = Some(task_id);
        if let Some(session_id) = self.preferred_session_for_task(&task) {
            self.select_session_by_id(session_id, window, cx);
        } else {
            self.selected_session = None;
            self.session = SessionInfo::placeholder();
            self.replace_messages(vec![MessageItem::new(
                "assistant",
                "No messages yet. Create one to begin.",
            )]);
        }
        self.maybe_mark_selected_task_read(cx);
        cx.notify();
    }

    pub(crate) fn maybe_mark_selected_task_read(&mut self, cx: &mut Context<Self>) {
        let Some(task_id) = self.selected_task else {
            return;
        };
        if !self.should_mark_task_read(task_id) {
            return;
        }
        self.mark_task_read(task_id, cx);
    }

    fn should_mark_task_read(&self, task_id: TaskId) -> bool {
        let Some(task) = self.tasks_by_id.get(&task_id) else {
            return false;
        };
        let mut working = false;
        let mut session_unread = false;

        for track in &task.tracks {
            for session in &track.sessions {
                if session.activity.is_working {
                    working = true;
                }
                if session.unread.unwrap_or(false) {
                    session_unread = true;
                }
            }
        }

        if working {
            return false;
        }

        let last_assistant = task.task.last_assistant_message_at;
        let seen = task.task.assistant_seen_at;
        let task_unread = match last_assistant {
            Some(last) => seen.map(|seen| last > seen).unwrap_or(true),
            None => false,
        };

        session_unread || task_unread
    }

    fn select_session_by_id(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .sessions
            .iter()
            .position(|summary| summary.session_id == session_id)
        {
            self.select_session(index, window, cx);
        }
    }

    fn preferred_session_for_task(&self, task: &TaskSummaryItem) -> Option<SessionId> {
        for track in &task.tracks {
            if let Some(primary) = track.primary_session_id {
                if track.sessions.iter().any(|session| session.session.id == primary) {
                    return Some(primary);
                }
            }
        }
        task_session_summaries(task).first().map(|summary| summary.session_id)
    }

    fn rebuild_task_orders(&mut self) {
        self.task_active_order.clear();
        self.task_archived_order.clear();
        let mut items = self.tasks_by_id.values().collect::<Vec<_>>();
        items.sort_by(|a, b| {
            let cmp = b.sort_at_ms.cmp(&a.sort_at_ms);
            if cmp == std::cmp::Ordering::Equal {
                b.id.0.cmp(&a.id.0)
            } else {
                cmp
            }
        });
        for item in items {
            if item.is_archived() {
                self.task_archived_order.push(item.id);
            } else {
                self.task_active_order.push(item.id);
            }
        }
    }

    fn rebuild_session_state(&mut self) {
        let selected_session_id = self.selected_session_id();
        let mut sessions = Vec::new();
        let mut session_summary_map = HashMap::new();
        let mut session_last_event_seq = HashMap::new();

        let task_ids = self.task_active_order.clone();
        for task_id in &task_ids {
            let Some(task) = self.tasks_by_id.get(task_id) else {
                continue;
            };
            for track in &task.tracks {
                for summary in &track.sessions {
                    session_summary_map.insert(summary.session.id, summary.clone());
                    if let Some(seq) = summary.last_event_seq {
                        session_last_event_seq.insert(summary.session.id, seq);
                    }
                }
            }
            sessions.extend(task_session_summaries(task));
        }

        self.sessions = sessions;
        self.session_summary_map = session_summary_map;
        self.session_last_event_seq = session_last_event_seq;

        if let Some(session_id) = selected_session_id {
            self.selected_session = self
                .sessions
                .iter()
                .position(|summary| summary.session_id == session_id);
        } else {
            self.selected_session = None;
        }
    }

    fn update_task_counts_for_move(&mut self, prev: &TaskSummaryItem, next: &TaskSummaryItem) {
        let prev_archived = prev.is_archived();
        let next_archived = next.is_archived();
        if prev_archived == next_archived {
            return;
        }
        if prev_archived {
            if let Some(total) = self.catchup_archived_total.as_mut() {
                *total = total.saturating_sub(1);
            }
            if let Some(total) = self.catchup_active_total.as_mut() {
                *total += 1;
            }
        } else {
            if let Some(total) = self.catchup_active_total.as_mut() {
                *total = total.saturating_sub(1);
            }
            if let Some(total) = self.catchup_archived_total.as_mut() {
                *total += 1;
            }
        }
    }

    fn clear_task_focus(&mut self, message: &str) {
        self.selected_task = None;
        self.selected_session = None;
        self.new_task_mode = true;
        self.new_task_mode_locked = true;
        self.composer_needs_apply = true;
        self.session = SessionInfo::placeholder();
        self.replace_messages(vec![MessageItem::new("assistant", message)]);
        self.artifacts.clear();
        self.artifact_preview = ArtifactPreviewState::None;
        self.session_events.clear();
        self.selected_artifact = None;
    }

    #[allow(dead_code)]
    pub(crate) fn begin_task_rename(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(summary) = self.tasks_by_id.get(&task_id) else {
            return;
        };
        self.renaming_task_id = Some(task_id);
        self.rename_ignore_blur = false;
        let title = summary.task.title.clone();
        cx.update_entity(&self.rename_input, |state, cx| {
            state.set_value(title, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn cancel_task_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming_task_id = None;
        self.rename_ignore_blur = false;
        cx.notify();
    }

    pub(crate) fn commit_task_rename(
        &mut self,
        task_id: TaskId,
        next_value: String,
        cx: &mut Context<Self>,
    ) {
        let next = next_value.trim().to_string();
        if next.is_empty() {
            return;
        }
        let current = self
            .tasks_by_id
            .get(&task_id)
            .map(|summary| summary.task.title.trim().to_string())
            .unwrap_or_default();
        if current == next {
            self.cancel_task_rename(cx);
            return;
        }

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let updated = client.update_task_title(task_id, &next).await?;
            Ok(updated)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if let Ok(updated) = result {
                        view.apply_task_update(updated);
                        view.renaming_task_id = None;
                        view.rename_ignore_blur = false;
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn toggle_task_menu(
        &mut self,
        task_id: TaskId,
        anchor: AnchorRect,
        cx: &mut Context<Self>,
    ) {
        if let Some(current) = self.task_menu {
            if current.task_id == task_id {
                self.task_menu = None;
                cx.notify();
                return;
            }
        }
        self.task_menu = Some(TaskMenuState { task_id, anchor });
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn close_task_menu(&mut self, cx: &mut Context<Self>) {
        if self.task_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn toggle_task_archive(
        &mut self,
        task_id: TaskId,
        next_archived: bool,
        anchor: AnchorRect,
        cx: &mut Context<Self>,
    ) {
        if self.archive_pending.contains_key(&task_id) {
            return;
        }
        if next_archived && !self.archive_confirm_dismissed {
            self.archive_confirm = Some(ArchiveConfirmState { task_id, anchor });
            self.archive_confirm_dont_remind = false;
            cx.notify();
            return;
        }
        self.archive_confirm = None;
        self.apply_archive_toggle(task_id, next_archived, cx);
    }

    fn apply_archive_toggle(&mut self, task_id: TaskId, next_archived: bool, cx: &mut Context<Self>) {
        self.archive_pending.insert(
            task_id,
            if next_archived {
                TaskArchiveAction::Archive
            } else {
                TaskArchiveAction::Unarchive
            },
        );
        let was_selected = self.selected_task == Some(task_id);
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let updated = if next_archived {
                client.archive_task(task_id).await?
            } else {
                client.unarchive_task(task_id).await?
            };
            Ok(updated)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if let Ok(updated) = result {
                        view.apply_task_update(updated);
                        if next_archived && was_selected {
                            view.clear_task_focus("Select a task to begin.");
                        }
                    }
                    view.archive_pending.remove(&task_id);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    #[allow(dead_code)]
    pub(crate) fn confirm_archive(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.archive_confirm else {
            return;
        };
        if self.archive_pending.contains_key(&confirm.task_id) {
            self.archive_confirm = None;
            cx.notify();
            return;
        }
        self.archive_confirm = None;
        if self.archive_confirm_dont_remind {
            self.archive_confirm_dismissed = true;
            self.ui_state.set_archive_confirm_dismissed(true);
        }
        self.apply_archive_toggle(confirm.task_id, true, cx);
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_archive_confirm(&mut self, cx: &mut Context<Self>) {
        self.archive_confirm = None;
        cx.notify();
    }

    pub(crate) fn mark_task_read(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        if self.task_mark_read_inflight.contains(&task_id) {
            return;
        }
        self.task_mark_read_inflight.insert(task_id);
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let updated = client.mark_task_read(task_id).await?;
            Ok(updated)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.task_mark_read_inflight.remove(&task_id);
                    if let Ok(updated) = result {
                        view.apply_task_update(updated);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    #[allow(dead_code)]
    pub(crate) fn mark_task_unread(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let updated = client.mark_task_unread(task_id).await?;
            Ok(updated)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if let Ok(updated) = result {
                        view.apply_task_update(updated);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    #[allow(dead_code)]
    pub(crate) fn delete_task(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let was_selected = self.selected_task == Some(task_id);
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.delete_task(task_id).await?;
            Ok(())
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if result.is_ok() {
                        view.remove_task(task_id);
                        if was_selected {
                            view.clear_task_focus("Select a task to begin.");
                        }
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }
}
