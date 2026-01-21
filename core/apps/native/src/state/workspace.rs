use std::collections::HashMap;

use gpui::{AppContext as _, AsyncApp, ClickEvent, Context, WeakEntity, Window};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    MessageRole, SessionHeadSnapshot, SessionSnapshot, SessionSnapshotSummary, Task,
    WorkspaceActiveSnapshot, WorkspaceActiveTaskSummary, WorkspaceTaskSummary,
};
use ctx_providers::adapters::ProviderStatus;

use super::{
    AnchorRect, ArchiveConfirmState, ArtifactPreviewState, RightPaneMode, ShellView, StreamStatus,
    TaskArchiveAction, TaskMenuState,
};
use super::session::SessionThreadCache;
use super::super::models::{
    session_info_from_summary, MessageItem,
    SessionInfo,
};
use super::super::workspace_summary::{
    session_snapshot_from_summary, task_session_summaries, TaskSummaryItem,
};

#[derive(Clone)]
pub(crate) struct WorkspaceItem {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
    pub(crate) root_path: String,
}

pub(crate) type ProviderItem = ProviderStatus;

struct InitialLoadResult {
    workspaces: Vec<WorkspaceItem>,
    providers: Vec<ProviderItem>,
}

struct WorkspaceLoadResult {
    active_snapshot: WorkspaceActiveSnapshot,
}

pub(crate) enum DataLoadState {
    Loading,
    Loaded,
    Error(String),
}

impl ShellView {
    fn ensure_workspace_tab(&mut self, workspace_id: WorkspaceId) {
        if self.workspace_tabs.iter().any(|id| *id == workspace_id) {
            return;
        }
        self.workspace_tabs.push(workspace_id);
    }

    pub(crate) fn open_workspace_tab(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        self.ensure_workspace_tab(workspace_id);
        self.activate_workspace_tab(workspace_id, cx);
    }

    pub(crate) fn activate_workspace_tab(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        if self.active_workspace_tab == Some(workspace_id) && self.selected_workspace == Some(workspace_id) {
            return;
        }
        self.active_workspace_tab = Some(workspace_id);
        self.load_workspace(workspace_id, cx);
    }

    pub(crate) fn close_workspace_tab(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        let idx = self.workspace_tabs.iter().position(|id| *id == workspace_id);
        if idx.is_none() {
            return;
        }
        let idx = idx.unwrap();
        self.workspace_tabs.retain(|id| *id != workspace_id);

        if self.active_workspace_tab != Some(workspace_id) {
            cx.notify();
            return;
        }

        let next = self.workspace_tabs.get(idx).copied().or_else(|| self.workspace_tabs.last().copied());
        if let Some(next_id) = next {
            self.activate_workspace_tab(next_id, cx);
            return;
        }

        self.active_workspace_tab = None;
        self.selected_workspace = None;
        self.stop_workspace_stream();
        self.reset_workspace_view("No workspace selected.", cx);
        self.data_state = DataLoadState::Loaded;
        cx.notify();
    }

    pub(crate) fn focus_new_task(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_task_focus("Select a task to begin.", cx);
        cx.notify();
    }

    pub(crate) fn focus_new_task_shortcut(&mut self, cx: &mut Context<Self>) {
        self.clear_task_focus("Select a task to begin.", cx);
        cx.notify();
    }

    pub(crate) fn select_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        self.open_workspace_tab(workspace.id, cx);
    }

    pub(crate) fn create_workspace(&mut self, cx: &mut Context<Self>) {
        let root_path = self
            .workspace_root_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if root_path.is_empty() {
            return;
        }
        let name = self
            .workspace_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let request = ctx_client::CreateWorkspaceRequest {
            root_path,
            name: if name.is_empty() { None } else { Some(name) },
        };

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.create_workspace(&request).await
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(_) => {
                            view.start_data_load(cx);
                        }
                        Err(err) => {
                            eprintln!("ctx-native: create workspace failed: {err}");
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn start_data_load(&mut self, cx: &mut Context<Self>) {
        self.ensure_thread_list_handler(cx);
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...", cx);
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
                    root_path: workspace.root_path,
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
                            view.workspace_tabs.retain(|id| view.workspaces.iter().any(|ws| ws.id == *id));
                            if let Some(active) = view.active_workspace_tab {
                                if !view.workspace_tabs.iter().any(|id| *id == active) {
                                    view.active_workspace_tab = None;
                                }
                            }
                            if let Some(selected) = view.selected_workspace {
                                if !view.workspaces.iter().any(|ws| ws.id == selected) {
                                    view.selected_workspace = None;
                                }
                            }

                            let workspace_id = view
                                .active_workspace_tab
                                .or(view.selected_workspace)
                                .or_else(|| view.workspace_tabs.first().copied())
                                .or_else(|| view.workspaces.first().map(|workspace| workspace.id));
                            if let Some(workspace_id) = workspace_id {
                                view.open_workspace_tab(workspace_id, cx);
                            } else {
                                view.reset_workspace_view("No workspaces yet.", cx);
                                view.data_state = DataLoadState::Loaded;
                                cx.notify();
                            }
                        }
                        Err(err) => {
                            view.workspaces.clear();
                            view.providers.clear();
                            view.selected_workspace = None;
                            view.reset_workspace_view("Unable to load workspace list.", cx);
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
        self.ensure_workspace_tab(workspace_id);
        self.active_workspace_tab = Some(workspace_id);
        self.apply_workspace_ui_state(workspace_id);
        self.selected_workspace = Some(workspace_id);
        if let Some(workspace) = self.workspaces.iter().find(|ws| ws.id == workspace_id) {
            self.ui_state
                .record_recent_workspace(&workspace.name, &workspace.root_path);
        }
        self.stop_workspace_stream();
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...", cx);
        self.hydrate_cached_active_snapshot(workspace_id, cx);
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceActiveSnapshotParams {
                limit: Some(50),
            };
            let active_snapshot = client
                .get_workspace_active_snapshot(workspace_id, &params)
                .await?;
            Ok(WorkspaceLoadResult {
                active_snapshot,
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
                        let preserve_session = view.task_store_initialized;
                        view.apply_active_snapshot(&data.active_snapshot, preserve_session, cx);
                        view.persist_active_snapshot(&data.active_snapshot, cx);
                        if !preserve_session {
                            view.hydrate_loaded_snapshot_state(cx);
                        }
                        view.data_state = DataLoadState::Loaded;
                        view.start_workspace_stream(workspace_id, cx);
                        view.prefetch_archived_head_window(cx);
                    }
                    Err(err) => {
                        if view.task_store_initialized {
                            view.data_state = DataLoadState::Error(err.to_string());
                        } else {
                            view.reset_workspace_view("Unable to load workspace data.", cx);
                            view.data_state = DataLoadState::Error(err.to_string());
                        }
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

    fn hydrate_cached_active_snapshot(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        let cache = self.ats_cache.clone();
        let task = Tokio::spawn_result(cx, async move {
            cache.load_active_snapshot(workspace_id).await
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.selected_workspace != Some(workspace_id) {
                        return;
                    }
                    if view.task_store_initialized {
                        return;
                    }
                    let Ok(Some(snapshot)) = result else {
                        return;
                    };
                    view.apply_active_snapshot(&snapshot, false, cx);
                    view.hydrate_loaded_snapshot_state(cx);
                    view.hydrate_cached_archived_head(workspace_id, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn hydrate_loaded_snapshot_state(&mut self, cx: &mut Context<Self>) {
        let keep_new_task = self.new_task_mode_locked;
        let selected_session_id = if keep_new_task {
            None
        } else {
            self.task_active_order
                .first()
                .and_then(|task_id| self.tasks_by_id.get(task_id))
                .and_then(|task| self.preferred_session_for_task(task))
        };
        self.selected_session = selected_session_id.and_then(|id| {
            self.sessions
                .iter()
                .position(|summary| summary.session_id == id)
        });
        self.selected_task = if keep_new_task {
            None
        } else {
            self.selected_session
                .and_then(|index| self.sessions.get(index))
                .and_then(|summary| {
                    self.session_summary_map
                        .get(&summary.session_id)
                        .map(|summary| summary.session.task_id)
                })
                .or_else(|| self.task_active_order.first().copied())
        };
        self.new_task_mode = keep_new_task || self.selected_session.is_none();
        self.composer_focus_pending = self.new_task_mode;
        self.hydrate_pane_state();
        self.composer_needs_apply = true;
        self.session = if keep_new_task {
            SessionInfo::placeholder()
        } else {
            selected_session_id
                .and_then(|id| self.session_summary_map.get(&id))
                .map(session_info_from_summary)
                .unwrap_or_else(SessionInfo::placeholder)
        };
        self.sync_composer_defaults();
        self.session_events.clear();
        self.session_turns.clear();
        self.session_turn_tools.clear();
        self.session_history_cursor = None;
        self.session_history_has_more = false;
        self.session_history_loading = false;
        self.replace_messages(Vec::new(), cx);
        if let Some(session_id) = self.selected_session_id() {
            if !self.apply_cached_thread_data(session_id, cx) {
                self.replace_messages(
                    vec![MessageItem::new(
                        MessageRole::Assistant,
                        "No messages yet. Create one to begin.",
                    )],
                    cx,
                );
            }
            if matches!(self.right_pane, Some(RightPaneMode::Artifacts)) {
                self.load_session_artifacts(session_id, cx);
            }
        } else {
            self.replace_messages(
                vec![MessageItem::new(
                    MessageRole::Assistant,
                    "Select a task to begin.",
                )],
                cx,
            );
        }
    }

    fn persist_active_snapshot(&self, snapshot: &WorkspaceActiveSnapshot, cx: &mut Context<Self>) {
        let snapshot = snapshot.clone();
        let ats_cache = self.ats_cache.clone();
        Tokio::spawn(cx, async move {
            let _ = ats_cache.save_active_snapshot(&snapshot).await;
        })
        .detach();
    }

    fn persist_archived_head_window(
        &self,
        page: &ctx_core::models::WorkspaceArchivedPage,
        cx: &mut Context<Self>,
    ) {
        let page = page.clone();
        let ats_cache = self.ats_cache.clone();
        Tokio::spawn(cx, async move {
            let _ = ats_cache
                .save_archived_head_window(page.workspace_id, &page)
                .await;
        })
        .detach();
    }

    fn hydrate_cached_archived_head(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        if self.task_archived_loaded {
            return;
        }
        let cache = self.ats_cache.clone();
        let task = Tokio::spawn_result(cx, async move {
            cache.load_archived_head_window(workspace_id).await
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.selected_workspace != Some(workspace_id) {
                        return;
                    }
                    if view.task_archived_loaded {
                        return;
                    }
                    let Ok(Some(page)) = result else {
                        return;
                    };
                    view.apply_archived_head_window(&page);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn prefetch_archived_head_window(&mut self, cx: &mut Context<Self>) {
        if self.task_archived_loaded {
            return;
        }
        if self.task_fetch_archived == super::TaskFetchState::Loading {
            return;
        }
        self.load_more_archived_tasks(true, cx);
    }

    fn reset_workspace_view(&mut self, message: &str, cx: &mut Context<Self>) {
        self.task_store_initialized = false;
        self.workspace_snapshot_rev = 0;
        self.archived_snapshot_rev = 0;
        self.task_fetch_active = super::TaskFetchState::Idle;
        self.task_fetch_archived = super::TaskFetchState::Idle;
        self.task_has_more_active = true;
        self.task_has_more_archived = false;
        self.task_archived_loaded = false;
        self.tasks_by_id.clear();
        self.task_active_order.clear();
        self.task_archived_order.clear();
        self.task_archived_cursor = None;
        self.task_query.clear();
        self.right_pane = None;
        self.selected_task = None;
        self.sessions.clear();
        self.selected_session = None;
        self.replace_messages(vec![MessageItem::new(MessageRole::Assistant, message)], cx);
        self.artifacts.clear();
        self.artifacts_session_id = None;
        self.artifact_preview = ArtifactPreviewState::None;
        self.clear_artifact_prefetch_cache();
        self.session_events.clear();
        self.selected_artifact = None;
        self.session_history_cursor = None;
        self.session_history_has_more = false;
        self.session_history_loading = false;
        self.composer_attachments.clear();
        self.composer_notice = None;
        self.composer_provider_menu_open = false;
        self.composer_model_menu_open = false;
        self.session_summary_map.clear();
        self.session_thread_cache.clear();
        self.session_thread_view_state.clear();
        self.thread_render_cache.clear();
        self.thread_render_inflight.clear();
        self.session_last_event_seq.clear();
        self.resyncing_session = None;
        self.active_snapshot_rev = None;
        self.session = SessionInfo::placeholder();
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

    fn apply_active_snapshot(
        &mut self,
        snapshot: &WorkspaceActiveSnapshot,
        preserve_session: bool,
        cx: &mut Context<Self>,
    ) {
        let archived_items = if preserve_session {
            self.task_archived_order
                .iter()
                .filter_map(|id| self.tasks_by_id.get(id).cloned())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        self.task_store_initialized = true;
        self.workspace_snapshot_rev = self.workspace_snapshot_rev.max(snapshot.snapshot_rev);
        self.archived_snapshot_rev = self.archived_snapshot_rev.max(snapshot.archived_rev);
        self.active_snapshot_rev = Some(snapshot.snapshot_rev);
        self.task_fetch_active = super::TaskFetchState::Idle;
        self.task_has_more_active = false;

        if !preserve_session {
            self.task_fetch_archived = super::TaskFetchState::Idle;
            self.task_has_more_archived = false;
            self.task_archived_loaded = false;
            self.task_archived_order.clear();
            self.task_archived_cursor = None;
        }

        self.tasks_by_id.clear();
        self.task_active_order.clear();
        if !preserve_session {
            self.session_thread_cache.clear();
            self.session_thread_view_state.clear();
            self.thread_render_cache.clear();
            self.thread_render_inflight.clear();
            self.session_head_meta.clear();
            self.session_last_event_seq.clear();
        }

        for summary in &snapshot.active.tasks {
            let item = TaskSummaryItem::from_active(&summary);
            self.tasks_by_id.insert(item.id, item);
            self.cache_session_snapshot(&summary.primary_session, &summary.primary_session_head, cx);
        }

        if preserve_session {
            for item in archived_items {
                self.tasks_by_id.entry(item.id).or_insert(item);
            }
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
        self.task_fetch_active = super::TaskFetchState::Loading;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceActiveSnapshotParams {
                limit: Some(50),
            };
            let snapshot = client
                .get_workspace_active_snapshot(workspace_id, &params)
                .await?;
            Ok(snapshot)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| match result {
                    Ok(snapshot) => {
                        view.apply_active_snapshot(&snapshot, true, cx);
                        view.persist_active_snapshot(&snapshot, cx);
                        view.send_stream_subscribe();
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

    pub(crate) fn refresh_active_snapshot(&mut self, cx: &mut Context<Self>) {
        if self.task_fetch_active == super::TaskFetchState::Loading {
            return;
        }
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        self.task_fetch_active = super::TaskFetchState::Loading;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceActiveSnapshotParams {
                limit: Some(50),
            };
            let snapshot = client
                .get_workspace_active_snapshot(workspace_id, &params)
                .await?;
            Ok(snapshot)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| match result {
                    Ok(snapshot) => {
                        view.apply_active_snapshot(&snapshot, true, cx);
                        view.persist_active_snapshot(&snapshot, cx);
                        view.send_stream_subscribe();
                        view.task_fetch_active = super::TaskFetchState::Idle;
                        view.prefetch_archived_head_window(cx);
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
        let persist_head = reset;
        self.task_fetch_archived = super::TaskFetchState::Loading;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceArchivedPageParams {
                limit: Some(50),
                cursor,
            };
            client
                .list_workspace_archived_task_summaries(workspace_id, &params)
                .await
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(page) => {
                            if reset {
                                view.clear_archived_tasks();
                                view.task_archived_cursor = None;
                            }
                            view.archived_snapshot_rev =
                                view.archived_snapshot_rev.max(page.archived_rev);
                            for summary in page.tasks.iter() {
                                let fallback_worktree_id = summary.task.primary_worktree_id;
                                let sessions = summary
                                    .sessions
                                    .iter()
                                    .map(|session| {
                                        session_snapshot_from_summary(session, fallback_worktree_id)
                                    })
                                    .collect::<Vec<_>>();
                                let item = TaskSummaryItem::from_archived(summary.task.clone(), sessions);
                                view.tasks_by_id.insert(item.id, item);
                            }
                            view.task_archived_loaded = true;
                            view.task_has_more_archived = page.next_cursor.is_some();
                            view.task_archived_cursor = page.next_cursor.clone();
                            if persist_head {
                                view.persist_archived_head_window(&page, cx);
                            }
                            view.rebuild_task_orders();
                            view.rebuild_session_state();
                            view.task_fetch_archived = super::TaskFetchState::Idle;
                        }
                        Err(_) => {
                            view.task_fetch_archived = super::TaskFetchState::Error;
                        }
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

    fn clear_archived_tasks(&mut self) {
        for task_id in self.task_archived_order.drain(..) {
            self.tasks_by_id.remove(&task_id);
        }
    }

    fn apply_archived_head_window(&mut self, page: &ctx_core::models::WorkspaceArchivedPage) {
        self.clear_archived_tasks();
        self.task_archived_cursor = page.next_cursor.clone();
        self.task_archived_loaded = true;
        self.task_has_more_archived = page.next_cursor.is_some();
        self.archived_snapshot_rev = self.archived_snapshot_rev.max(page.archived_rev);

        for summary in &page.tasks {
            let fallback_worktree_id = summary.task.primary_worktree_id;
            let sessions = summary
                .sessions
                .iter()
                .map(|session| session_snapshot_from_summary(session, fallback_worktree_id))
                .collect::<Vec<_>>();
            let item = TaskSummaryItem::from_archived(summary.task.clone(), sessions);
            self.tasks_by_id.insert(item.id, item);
        }

        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn upsert_active_task_summary(
        &mut self,
        summary: WorkspaceActiveTaskSummary,
        cx: &mut Context<Self>,
    ) {
        let item = TaskSummaryItem::from_active(&summary);
        self.tasks_by_id.insert(item.id, item);
        self.cache_session_snapshot(&summary.primary_session, &summary.primary_session_head, cx);
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn upsert_archived_task_summary(
        &mut self,
        summary: WorkspaceTaskSummary,
        snapshot: Option<SessionSnapshot>,
        cx: &mut Context<Self>,
    ) {
        let fallback_worktree_id = summary.task.primary_worktree_id;
        let sessions = summary
            .sessions
            .iter()
            .map(|session| session_snapshot_from_summary(session, fallback_worktree_id))
            .collect::<Vec<_>>();
        let item = TaskSummaryItem::from_archived(summary.task, sessions);
        self.tasks_by_id.insert(item.id, item);
        if let Some(snapshot) = snapshot.as_ref() {
            self.cache_session_snapshot(&snapshot.summary, &snapshot.head, cx);
        }
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn apply_task_update(&mut self, task: Task) {
        let Some(existing) = self.tasks_by_id.get(&task.id).cloned() else {
            return;
        };
        let updated = existing.with_task(task);
        self.tasks_by_id.insert(updated.id, updated);
        self.rebuild_task_orders();
        self.rebuild_session_state();
    }

    pub(crate) fn remove_task(&mut self, task_id: TaskId) {
        if self.tasks_by_id.remove(&task_id).is_none() {
            return;
        };
        self.task_active_order.retain(|id| *id != task_id);
        self.task_archived_order.retain(|id| *id != task_id);
        if self.selected_task == Some(task_id) {
            self.selected_task = None;
        }
        self.rebuild_session_state();
    }

    pub(crate) fn apply_session_summary(&mut self, summary: SessionSnapshotSummary) {
        let task_id = summary.session.task_id;
        let Some(task) = self.tasks_by_id.get(&task_id).cloned() else {
            return;
        };
        let mut primary_session = task.primary_session.clone();
        let mut sessions = task.sessions.clone();

        if primary_session
            .as_ref()
            .map(|session| session.session.id == summary.session.id)
            .unwrap_or(false)
        {
            primary_session = Some(summary);
        } else {
            let idx = sessions
                .iter()
                .position(|session| session.session.id == summary.session.id);
            if let Some(idx) = idx {
                sessions[idx] = summary;
            } else {
                sessions.push(summary);
            }
            sessions.sort_by_key(|session| session.session.created_at);
        }

        let updated = TaskSummaryItem {
            primary_session,
            sessions,
            ..task
        };
        self.tasks_by_id.insert(task_id, updated);
        self.rebuild_session_state();
    }

    fn cache_session_snapshot(
        &mut self,
        summary: &SessionSnapshotSummary,
        head: &SessionHeadSnapshot,
        cx: &mut Context<Self>,
    ) {
        let cache = SessionThreadCache::from_snapshot(head);
        self.session_thread_cache
            .insert(summary.session.id, cache);
        self.update_session_head_meta(summary.session.id, head);
        let mut seq = summary.last_event_seq.unwrap_or(head.last_event_seq);
        if let Some(prev) = self.session_last_event_seq.get(&summary.session.id) {
            seq = seq.max(*prev);
        }
        self.session_last_event_seq.insert(summary.session.id, seq);
        self.persist_cached_session_head(summary.session.id, cx);
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
                MessageRole::Assistant,
                "No messages yet. Create one to begin.",
            )], cx);
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

        let summaries = task
            .primary_session
            .iter()
            .chain(task.sessions.iter());
        for session in summaries {
            if session.activity.is_working {
                working = true;
            }
            if session.unread.unwrap_or(false) {
                session_unread = true;
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
        if let Some(primary) = task.primary_session.as_ref() {
            return Some(primary.session.id);
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
        let prev_last_event_seq = std::mem::take(&mut self.session_last_event_seq);

        let mut task_ids = self.task_active_order.clone();
        if let Some(task_id) = self.selected_task {
            if self
                .task_archived_order
                .iter()
                .any(|archived_id| *archived_id == task_id)
            {
                task_ids.push(task_id);
            }
        }
        for task_id in &task_ids {
            let Some(task) = self.tasks_by_id.get(task_id) else {
                continue;
            };
            let summaries = task
                .primary_session
                .iter()
                .chain(task.sessions.iter());
            for summary in summaries {
                session_summary_map.insert(summary.session.id, summary.clone());
                let mut seq = summary.last_event_seq;
                if let Some(prev) = prev_last_event_seq.get(&summary.session.id) {
                    seq = Some(seq.map_or(*prev, |current| current.max(*prev)));
                }
                if let Some(seq) = seq {
                    session_last_event_seq.insert(summary.session.id, seq);
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

    fn clear_task_focus(&mut self, message: &str, cx: &mut Context<Self>) {
        self.selected_task = None;
        self.selected_session = None;
        self.new_task_mode = true;
        self.new_task_mode_locked = true;
        self.composer_focus_pending = true;
        self.composer_needs_apply = true;
        self.composer_context_window = None;
        self.right_pane = None;
        self.session = SessionInfo::placeholder();
        self.replace_messages(
            vec![MessageItem::new(MessageRole::Assistant, message)],
            cx,
        );
        self.artifacts.clear();
        self.artifacts_session_id = None;
        self.artifact_preview = ArtifactPreviewState::None;
        self.clear_artifact_prefetch_cache();
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
                        if next_archived {
                            view.artifact_prefetch_cache.purge_task(task_id);
                            if was_selected {
                                view.clear_task_focus("Select a task to begin.", cx);
                            }
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
                            view.clear_task_focus("Select a task to begin.", cx);
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
