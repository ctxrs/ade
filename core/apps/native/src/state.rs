use std::collections::HashMap;
use std::ops::Range;

use gpui::{ClickEvent, ClipboardItem, Context, FocusHandle, KeyDownEvent, Window};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{Artifact, SessionCatchupSummary, SessionEvent, SessionHead, SessionHistoryPage};

use crate::theme::ThemeColors;

use super::models::{
    build_message_items, session_info_from_head, session_info_from_summary, MessageItem,
    SessionInfo,
};
use super::workspace_summary::{
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

struct SessionLoadResult {
    session_id: SessionId,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    session_events: Vec<SessionEvent>,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Copy)]
enum SessionControlAction {
    Interrupt,
    Cancel,
}

impl SessionControlAction {
    fn failure_message(self) -> &'static str {
        match self {
            SessionControlAction::Interrupt => "Unable to interrupt session.",
            SessionControlAction::Cancel => "Unable to cancel session.",
        }
    }
}

pub(crate) enum DataLoadState {
    Loading,
    Loaded,
    Error(String),
}

const COMPOSER_HISTORY_LIMIT: usize = 20;

pub(crate) struct ComposerState {
    text: String,
    cursor: usize,
    selection_anchor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<String>,
}

impl ComposerState {
    pub(crate) fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            selection_anchor: 0,
            history: Vec::new(),
            history_index: None,
            history_draft: None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        self.text.as_str()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = 0;
        self.history_index = None;
        self.history_draft = None;
    }

    pub(crate) fn selection_range(&self) -> Range<usize> {
        if self.selection_anchor <= self.cursor {
            self.selection_anchor..self.cursor
        } else {
            self.cursor..self.selection_anchor
        }
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.selection_anchor != self.cursor
    }

    pub(crate) fn select_all(&mut self) {
        self.selection_anchor = 0;
        self.cursor = self.text.len();
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        self.exit_history_on_edit();
        self.replace_selection(text);
    }

    pub(crate) fn delete_backward(&mut self) -> bool {
        self.exit_history_on_edit();
        let selection = self.selection_range();
        if !selection.is_empty() {
            self.replace_range(selection, "");
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let new_cursor = prev_char_boundary(&self.text, self.cursor);
        self.replace_range(new_cursor..self.cursor, "");
        true
    }

    pub(crate) fn delete_forward(&mut self) -> bool {
        self.exit_history_on_edit();
        let selection = self.selection_range();
        if !selection.is_empty() {
            self.replace_range(selection, "");
            return true;
        }
        if self.cursor >= self.text.len() {
            return false;
        }
        let new_cursor = next_char_boundary(&self.text, self.cursor);
        self.replace_range(self.cursor..new_cursor, "");
        true
    }

    pub(crate) fn move_left(&mut self, select: bool) {
        if !select && self.has_selection() {
            let selection = self.selection_range();
            self.cursor = selection.start;
            self.selection_anchor = self.cursor;
            return;
        }
        let new_cursor = prev_char_boundary(&self.text, self.cursor);
        self.cursor = new_cursor;
        if !select {
            self.selection_anchor = self.cursor;
        }
    }

    pub(crate) fn move_right(&mut self, select: bool) {
        if !select && self.has_selection() {
            let selection = self.selection_range();
            self.cursor = selection.end;
            self.selection_anchor = self.cursor;
            return;
        }
        let new_cursor = next_char_boundary(&self.text, self.cursor);
        self.cursor = new_cursor;
        if !select {
            self.selection_anchor = self.cursor;
        }
    }

    pub(crate) fn move_home(&mut self, select: bool) {
        self.cursor = 0;
        if !select {
            self.selection_anchor = 0;
        }
    }

    pub(crate) fn move_end(&mut self, select: bool) {
        self.cursor = self.text.len();
        if !select {
            self.selection_anchor = self.cursor;
        }
    }

    pub(crate) fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let next_index = match self.history_index {
            Some(index) => {
                if index == 0 {
                    return false;
                }
                index - 1
            }
            None => {
                self.history_draft = Some(self.text.clone());
                self.history.len() - 1
            }
        };
        self.history_index = Some(next_index);
        self.set_text(self.history[next_index].clone());
        true
    }

    pub(crate) fn history_next(&mut self) -> bool {
        let Some(index) = self.history_index else {
            return false;
        };
        let next_index = index + 1;
        if next_index < self.history.len() {
            self.history_index = Some(next_index);
            self.set_text(self.history[next_index].clone());
            return true;
        }
        self.history_index = None;
        let draft = self.history_draft.take().unwrap_or_default();
        self.set_text(draft);
        true
    }

    pub(crate) fn push_history(&mut self, entry: String) {
        if entry.trim().is_empty() {
            return;
        }
        self.history.push(entry);
        if self.history.len() > COMPOSER_HISTORY_LIMIT {
            let overflow = self.history.len() - COMPOSER_HISTORY_LIMIT;
            self.history.drain(0..overflow);
        }
    }

    fn replace_selection(&mut self, text: &str) {
        let selection = self.selection_range();
        if selection.is_empty() {
            self.text.insert_str(self.cursor, text);
            self.cursor += text.len();
        } else {
            self.replace_range(selection, text);
        }
        self.selection_anchor = self.cursor;
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.text.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.selection_anchor = self.cursor;
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.selection_anchor = self.cursor;
    }

    fn exit_history_on_edit(&mut self) {
        if self.history_index.is_some() {
            self.history_index = None;
            self.history_draft = None;
        }
    }
}

fn prev_char_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut index = offset.saturating_sub(1);
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    let mut index = (offset + 1).min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    pub(crate) is_dark: bool,
    pub(crate) workspaces: Vec<WorkspaceItem>,
    pub(crate) selected_workspace: Option<WorkspaceId>,
    pub(crate) providers: Vec<ProviderItem>,
    pub(crate) catchup_active_total: Option<i64>,
    pub(crate) catchup_archived_total: Option<i64>,
    pub(crate) tasks: Vec<TaskSummaryItem>,
    pub(crate) selected_task: Option<usize>,
    pub(crate) sessions: Vec<SessionSummaryItem>,
    pub(crate) selected_session: Option<usize>,
    pub(crate) messages: Vec<MessageItem>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) selected_artifact: Option<usize>,
    pub(crate) session_events: Vec<SessionEvent>,
    pub(crate) session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    pub(crate) session: SessionInfo,
    pub(crate) data_state: DataLoadState,
    pub(crate) composer: ComposerState,
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
        let modifiers = event.keystroke.modifiers;
        let has_command = modifiers.control || modifiers.platform;

        if has_command {
            match event.keystroke.key.as_str() {
                "enter" => {
                    self.send_composer_message(cx);
                    return;
                }
                "c" => {
                    self.copy_composer(cx);
                    return;
                }
                "x" => {
                    self.cut_composer(cx);
                    return;
                }
                "v" => {
                    self.paste_composer(cx);
                    return;
                }
                "a" => {
                    self.composer.select_all();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match event.keystroke.key.as_str() {
            "backspace" => {
                if self.composer.delete_backward() {
                    cx.notify();
                }
            }
            "delete" => {
                if self.composer.delete_forward() {
                    cx.notify();
                }
            }
            "enter" => {
                self.composer.insert_text("\n");
                cx.notify();
            }
            "tab" => {}
            "left" => {
                self.composer.move_left(modifiers.shift);
                cx.notify();
            }
            "right" => {
                self.composer.move_right(modifiers.shift);
                cx.notify();
            }
            "home" => {
                self.composer.move_home(modifiers.shift);
                cx.notify();
            }
            "end" => {
                self.composer.move_end(modifiers.shift);
                cx.notify();
            }
            "up" => {
                if !modifiers.shift
                    && !modifiers.alt
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.function
                    && self.composer.history_prev()
                {
                    cx.notify();
                }
            }
            "down" => {
                if !modifiers.shift
                    && !modifiers.alt
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.function
                    && self.composer.history_next()
                {
                    cx.notify();
                }
            }
            _ => {
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                if let Some(text) = event.keystroke.key_char.as_ref() {
                    if text != "\n" && text != "\r" && text != "\t" {
                        self.composer.insert_text(text);
                        cx.notify();
                    }
                }
            }
        }
    }

    pub(crate) fn on_send_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.send_composer_message(cx);
    }

    pub(crate) fn on_interrupt_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_control(SessionControlAction::Interrupt, cx);
    }

    pub(crate) fn on_cancel_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_control(SessionControlAction::Cancel, cx);
    }

    fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let content = self.composer.text().trim().to_string();
        if content.is_empty() {
            return;
        }
        let history_entry = content.clone();

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
                        view.composer.push_history(history_entry);
                        view.composer.clear();
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

    fn copy_composer(&mut self, cx: &mut Context<Self>) {
        let selection = self.composer.selection_range();
        let text = if selection.is_empty() {
            self.composer.text().to_string()
        } else {
            self.composer
                .text()
                .get(selection)
                .unwrap_or_default()
                .to_string()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut_composer(&mut self, cx: &mut Context<Self>) {
        let selection = self.composer.selection_range();
        let text = if selection.is_empty() {
            self.composer.text().to_string()
        } else {
            self.composer
                .text()
                .get(selection.clone())
                .unwrap_or_default()
                .to_string()
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if selection.is_empty() {
            self.composer.clear();
        } else {
            self.composer.delete_backward();
        }
        cx.notify();
    }

    fn paste_composer(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if !text.is_empty() {
                self.composer.insert_text(&text);
                cx.notify();
            }
        }
    }

    fn request_session_control(
        &mut self,
        action: SessionControlAction,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            match action {
                SessionControlAction::Interrupt => client.interrupt_session(session_id).await?,
                SessionControlAction::Cancel => client.cancel_session(session_id).await?,
            }
            Ok(session_id)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(session_id) => {
                        view.load_session_details(session_id, cx);
                    }
                    Err(_) => {
                        view.messages
                            .push(MessageItem::new("assistant", action.failure_message()));
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
        self.session_events.clear();
        self.selected_artifact = None;
        cx.notify();
        self.load_session_details(session_id, cx);
    }

    pub(crate) fn select_artifact(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.artifacts.len() {
            return;
        }
        self.selected_artifact = Some(index);
        cx.notify();
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
            let session_events = client
                .get_session_events(session_id, None, None, Some(40))
                .await
                .ok()
                .map(|page| page.events)
                .unwrap_or_default();
            let artifacts = client
                .list_session_artifacts(session_id)
                .await
                .ok()
                .map(|items| items.into_iter().collect::<Vec<_>>())
                .unwrap_or_default();

            Ok(SessionLoadResult {
                session_id,
                session_head,
                session_history,
                session_events,
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
                        if let Some(index) = view.selected_session {
                            if let Some(summary) = view.sessions.get_mut(index) {
                                if summary.session_id == data.session_id {
                                    summary.status = view.session.status.clone();
                                    summary.title = view.session.title.clone();
                                }
                            }
                        }
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
                        view.session_events = data.session_events;
                        view.artifacts = data.artifacts;
                        view.selected_artifact = if view.artifacts.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    }
                    Err(_) => {
                        view.messages = vec![MessageItem::new(
                            "assistant",
                            "Unable to load session details.",
                        )];
                        view.session_events.clear();
                        view.artifacts.clear();
                        view.selected_artifact = None;
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

    fn reset_workspace_view(&mut self, message: &str) {
        self.tasks.clear();
        self.selected_task = None;
        self.sessions.clear();
        self.selected_session = None;
        self.messages = vec![MessageItem::new("assistant", message)];
        self.artifacts.clear();
        self.session_events.clear();
        self.selected_artifact = None;
        self.session_summary_map.clear();
        self.session = SessionInfo::placeholder();
        self.catchup_active_total = None;
        self.catchup_archived_total = None;
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
                        view.catchup_active_total = data.catchup_active_total;
                        view.catchup_archived_total = data.catchup_archived_total;
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
                        view.session_events = data.session_events;
                        view.artifacts = data.artifacts;
                        view.selected_artifact = if view.artifacts.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                        view.data_state = DataLoadState::Loaded;
                    }
                    Err(err) => {
                        view.reset_workspace_view("Unable to load workspace data.");
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
