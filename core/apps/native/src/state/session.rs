use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};

use gpui::{
    AsyncApp, ClickEvent, ClipboardItem, Context, Image, ImageFormat, ListOffset, WeakEntity,
    Window, px,
};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, TurnId};
use ctx_core::models::{
    Artifact, MessageRole, SessionEvent, SessionHead, SessionHistoryPage, SessionTurn,
};
use ctx_client;

use super::{ArtifactPreviewState, ShellView};
use super::super::models::{
    attachment_cache_key, build_message_items, build_thread_list_items, message_item_from_model,
    session_info_from_head, session_info_from_summary, MessageAttachment, MessageItem,
    SessionInfo, ThreadItem, ThreadListItem, TurnToolSnapshot, WorkbenchTurnHeader,
};
use super::SessionViewVerbosity;

struct SessionLoadResult {
    session_id: SessionId,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    session_events: Vec<SessionEvent>,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Default)]
pub(crate) struct SessionThreadCache {
    pub(super) messages: Vec<MessageItem>,
    pub(super) session_turns: Vec<SessionTurn>,
    pub(super) session_turn_tools: HashMap<TurnId, Vec<TurnToolSnapshot>>,
    pub(super) session_events: Vec<SessionEvent>,
}

const COPIED_TIMEOUT: Duration = Duration::from_millis(1_000);
const SESSION_HISTORY_PAGE_LIMIT: u32 = 60;
const SESSION_HISTORY_PREFETCH_THRESHOLD: usize = 6;

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

const MESSAGE_PLACEHOLDER_TEXTS: [&str; 2] = [
    "No messages yet. Create one to begin.",
    "Unable to load session details.",
];

pub(super) fn clear_placeholder_messages_in(messages: &mut Vec<MessageItem>) {
    if messages.len() != 1 {
        return;
    }
    let content = messages[0].content.as_str();
    if MESSAGE_PLACEHOLDER_TEXTS
        .iter()
        .any(|placeholder| placeholder == &content)
    {
        messages.clear();
    }
}

impl ShellView {
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

    pub(crate) fn on_jump_to_latest_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.thread_auto_follow = true;
        self.new_thread_item_count = 0;
        self.scroll_thread_to_bottom();
        cx.notify();
    }

    fn request_session_control(&mut self, action: SessionControlAction, cx: &mut Context<Self>) {
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

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(session_id) => {
                            view.load_session_details(session_id, cx);
                        }
                        Err(_) => {
                            view.push_message(
                                MessageItem::new(MessageRole::Assistant, action.failure_message()),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn ensure_thread_list_handler(&mut self, cx: &mut Context<Self>) {
        if self.thread_list_handler_set {
            return;
        }
        let view_handle = cx.entity();
        self.thread_list_state
            .set_scroll_handler(move |event, _window, cx| {
                let _ = view_handle.update(cx, |view, cx| {
                    let auto_follow = !event.is_scrolled;
                    if view.thread_auto_follow != auto_follow
                        || (auto_follow && view.new_thread_item_count > 0)
                    {
                        view.thread_auto_follow = auto_follow;
                        if auto_follow {
                            view.new_thread_item_count = 0;
                        }
                        cx.notify();
                    }
                    view.update_sticky_turn_header(event.visible_range.clone());
                    view.maybe_load_more_session_history(event.visible_range.clone(), cx);
                });
            });
        self.thread_list_handler_set = true;
    }

    fn maybe_load_more_session_history(
        &mut self,
        visible_range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.session_history_loading || !self.session_history_has_more {
            return;
        }
        if visible_range.start > SESSION_HISTORY_PREFETCH_THRESHOLD {
            return;
        }
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let Some(before_seq) = self.session_history_cursor else {
            return;
        };

        self.session_history_loading = true;
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let history = client
                .get_session_history(session_id, Some(before_seq), Some(SESSION_HISTORY_PAGE_LIMIT))
                .await?;
            Ok(history)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if !view.is_session_selected(session_id) {
                        return;
                    }
                    view.session_history_loading = false;
                    match result {
                        Ok(history) => {
                            view.session_history_cursor = history.next_cursor;
                            view.session_history_has_more = history.has_more;
                            view.prepend_session_history(history, cx);
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn prepend_session_history(&mut self, history: SessionHistoryPage, cx: &mut Context<Self>) {
        self.clear_placeholder_messages();

        let mut message_ids = HashSet::new();
        for message in &self.messages {
            if let Some(id) = message.id {
                message_ids.insert(id);
            }
        }
        let mut new_messages = Vec::new();
        for message in history.messages {
            if message_ids.contains(&message.id) {
                continue;
            }
            new_messages.push(message_item_from_model(&message));
        }
        let has_new_messages = !new_messages.is_empty();
        if has_new_messages {
            let mut merged = new_messages;
            merged.extend(self.messages.drain(..));
            self.messages = merged;
        }

        let mut turn_ids = HashSet::new();
        for turn in &self.session_turns {
            turn_ids.insert(turn.turn_id);
        }
        let mut new_turns = history
            .turns
            .into_iter()
            .filter(|turn| !turn_ids.contains(&turn.turn_id))
            .collect::<Vec<_>>();
        if !new_turns.is_empty() {
            new_turns.sort_by(|a, b| {
                a.started_at
                    .cmp(&b.started_at)
                    .then_with(|| a.updated_at.cmp(&b.updated_at))
            });
            let mut merged = new_turns;
            merged.extend(self.session_turns.drain(..));
            self.session_turns = merged;
        }

        if has_new_messages {
            self.prefetch_attachment_images(cx);
        }

        self.rebuild_thread_items_with_prepend();
    }

    fn rebuild_thread_items_with_prepend(&mut self) {
        let items = build_thread_list_items(
            &self.session_turns,
            &self.messages,
            &self.session_turn_tools,
            &self.session_events,
        );
        let items = filter_thread_list_items(items, self.verbosity);
        self.apply_thread_items(items);
        let new_len = self.thread_list_len;

        if new_len == 0 {
            self.sticky_turn_header = None;
            self.sticky_turn_header_at_top = true;
        }
    }

    fn schedule_copied_reset(&self, key: String, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                tokio::time::sleep(COPIED_TIMEOUT).await;
                this.update(&mut cx, |view, cx| {
                    let now = Instant::now();
                    if matches!(view.copied_flags.get(&key), Some(expiry) if *expiry <= now) {
                        view.copied_flags.remove(&key);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn copy_text_with_key(
        &mut self,
        key: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        self.copied_flags
            .insert(key.clone(), Instant::now() + COPIED_TIMEOUT);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.schedule_copied_reset(key, cx);
        cx.notify();
    }

    pub(crate) fn copied_flag(&self, key: &str) -> bool {
        self.copied_flags
            .get(key)
            .map(|expires| *expires > Instant::now())
            .unwrap_or(false)
    }


    pub(crate) fn on_toggle_turn_header(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_turn_headers.get(&id).copied().unwrap_or(false);
        self.expanded_turn_headers.insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_message(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_messages.get(&id).copied().unwrap_or(false);
        self.expanded_messages.insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_turn_details(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_turn_details.get(&id).copied().unwrap_or(false);
        self.expanded_turn_details.insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_tool(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_tools.get(&id).copied().unwrap_or(false);
        self.expanded_tools.insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(super) fn replace_messages(&mut self, messages: Vec<MessageItem>, cx: &mut Context<Self>) {
        self.messages = messages;
        self.prefetch_attachment_images(cx);
        self.rebuild_thread_items();
        self.thread_auto_follow = true;
        self.new_thread_item_count = 0;
    }

    pub(super) fn cache_session_thread_state(&mut self, session_id: SessionId) {
        self.session_thread_cache.insert(
            session_id,
            SessionThreadCache {
                messages: self.messages.clone(),
                session_turns: self.session_turns.clone(),
                session_turn_tools: self.session_turn_tools.clone(),
                session_events: self.session_events.clone(),
            },
        );
    }

    fn apply_cached_thread_state(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cache) = self.session_thread_cache.get(&session_id) else {
            return false;
        };
        self.session_turns = cache.session_turns.clone();
        self.session_turn_tools = cache.session_turn_tools.clone();
        self.session_events = cache.session_events.clone();
        self.replace_messages(cache.messages.clone(), cx);
        true
    }

    fn apply_empty_thread_state(&mut self, cx: &mut Context<Self>) {
        self.session_turns.clear();
        self.session_turn_tools.clear();
        self.session_events.clear();
        self.replace_messages(Vec::new(), cx);
    }

    fn prefetch_attachment_images(&mut self, cx: &mut Context<Self>) {
        let attachments: Vec<MessageAttachment> = self
            .messages
            .iter()
            .flat_map(|message| message.attachments.clone())
            .collect();
        let active_keys: HashSet<String> =
            attachments.iter().map(attachment_cache_key).collect();
        self.attachment_fetch_failed
            .retain(|key| active_keys.contains(key));

        for attachment in attachments {
            let cache_key = attachment_cache_key(&attachment);
            if self.composer_attachment_images.contains_key(&cache_key)
                || self.composer_attachment_loading.contains(&cache_key)
                || self.attachment_fetch_failed.contains(&cache_key)
            {
                continue;
            }
            match attachment {
                MessageAttachment::Image {
                    mime_type,
                    data_base64,
                    ..
                } => {
                    if let Some(image) = inline_attachment_image(&mime_type, &data_base64) {
                        self.composer_attachment_images
                            .insert(cache_key.clone(), image);
                        self.invalidate_thread_items_for_attachment(&cache_key);
                        cx.notify();
                    } else {
                        self.attachment_fetch_failed.insert(cache_key.clone());
                        self.invalidate_thread_items_for_attachment(&cache_key);
                        cx.notify();
                    }
                }
                MessageAttachment::ImageRef {
                    blob_id,
                    mime_type,
                    ..
                } => {
                    self.composer_attachment_loading.insert(blob_id.clone());
                    let blob_id_for_task = blob_id.clone();
                    let blob_id_for_update = blob_id.clone();
                    let mime = mime_type.clone();
                    let task = Tokio::spawn_result(cx, async move {
                        let config = ctx_client::resolve_daemon_config()?;
                        let client = ctx_client::Client::new(config)?;
                        let bytes = client.get_blob(&blob_id_for_task).await?;
                        Ok((blob_id_for_task, mime, bytes))
                    });

                    cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
                        let mut cx = cx.clone();
                        async move {
                            let result = task.await;
                            this.update(&mut cx, |view, cx| {
                                view.composer_attachment_loading
                                    .remove(&blob_id_for_update);
                                match result {
                                    Ok((blob_id, mime, bytes)) => {
                                        let format = ImageFormat::from_mime_type(&mime)
                                            .unwrap_or(ImageFormat::Png);
                                        view.composer_attachment_images.insert(
                                            blob_id,
                                            Arc::new(Image::from_bytes(format, bytes)),
                                        );
                                    }
                                    Err(_) => {
                                        view.attachment_fetch_failed.insert(
                                            blob_id_for_update.clone(),
                                        );
                                    }
                                }
                                view.invalidate_thread_items_for_attachment(&blob_id_for_update);
                                cx.notify();
                            })
                            .ok();
                        }
                    })
                    .detach();
                }
            }
        }
    }

    fn clear_placeholder_messages(&mut self) {
        clear_placeholder_messages_in(&mut self.messages);
    }

    pub(super) fn push_message(&mut self, message: MessageItem, cx: &mut Context<Self>) {
        self.clear_placeholder_messages();
        self.messages.push(message);
        self.prefetch_attachment_images(cx);
        let old_len = self.thread_list_len;
        self.rebuild_thread_items();
        let new_len = self.thread_list_len;
        if self.thread_auto_follow {
            self.scroll_thread_to_bottom();
        } else if new_len > old_len {
            self.new_thread_item_count =
                self.new_thread_item_count.saturating_add(new_len - old_len);
        }
    }

    fn scroll_thread_to_bottom(&mut self) {
        let count = self.thread_list_len;
        self.thread_list_state.scroll_to(ListOffset {
            item_ix: count,
            offset_in_item: px(0.0),
        });
    }

    pub(super) fn update_session_last_event_seq(&mut self, session_id: SessionId, seq: i64) {
        self.session_last_event_seq.insert(session_id, seq);
    }

    pub(super) fn selected_session_id(&self) -> Option<SessionId> {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
    }

    pub(crate) fn select_session(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(summary) = self.sessions.get(index) else {
            return;
        };
        let session_id = summary.session_id;
        self.selected_session = Some(index);
        self.new_task_mode = false;
        self.new_task_mode_locked = false;
        self.apply_active_composer_state(window, cx);
        self.session = self
            .session_summary_map
            .get(&session_id)
            .map(session_info_from_summary)
            .unwrap_or_else(SessionInfo::placeholder);
        self.hydrate_pane_state();
        if let Some(summary) = self.session_summary_map.get(&session_id) {
            self.composer_provider_id = Some(summary.session.provider_id.clone());
            self.composer_model_id = Some(summary.session.model_id.clone());
        }
        self.reset_thread_state();
        self.artifacts.clear();
        self.artifact_preview = ArtifactPreviewState::None;
        self.session_events.clear();
        self.selected_artifact = None;
        self.resyncing_session = None;
        if !self.apply_cached_thread_state(session_id, cx) {
            self.apply_empty_thread_state(cx);
        }
        cx.notify();
        self.load_session_details(session_id, cx);
    }

    pub(super) fn load_session_details(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let session_head = client
                .get_session_head(session_id, Some(40), Some(false))
                .await
                .ok();
            let session_history = client
                .get_session_history(session_id, None, Some(SESSION_HISTORY_PAGE_LIMIT))
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

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
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
                        if let Some(head) = data.session_head.as_ref() {
                            view.composer_provider_id = Some(head.session.provider_id.clone());
                            view.composer_model_id = Some(head.session.model_id.clone());
                        }
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
                                MessageRole::Assistant,
                                "No messages yet. Create one to begin.",
                            ));
                        }
                        view.session_history_cursor =
                            data.session_history.as_ref().and_then(|page| page.next_cursor);
                        view.session_history_has_more = data
                            .session_history
                            .as_ref()
                            .map(|page| page.has_more)
                            .unwrap_or(false);
                        view.session_history_loading = false;
                        view.session_events = data.session_events;
                        view.session_turns = merge_session_turns(
                            data.session_head.as_ref(),
                            data.session_history.as_ref(),
                        );
                        view.session_turn_tools =
                            build_turn_tool_snapshots(data.session_head.as_ref());
                        view.replace_messages(messages, cx);
                        view.cache_session_thread_state(data.session_id);
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
                        if view.resyncing_session == Some(session_id) {
                            view.resyncing_session = None;
                        }
                    }
                    Err(_) => {
                        view.replace_messages(vec![MessageItem::new(
                            MessageRole::Assistant,
                            "Unable to load session details.",
                        )], cx);
                        view.session_events.clear();
                        view.session_turns.clear();
                        view.session_turn_tools.clear();
                        view.thread_items.clear();
                        view.thread_list_state.reset(0);
                        view.thread_list_len = 0;
                        view.sticky_turn_header = None;
                        view.sticky_turn_header_at_top = true;
                        view.session_history_cursor = None;
                        view.session_history_has_more = false;
                        view.session_history_loading = false;
                        view.artifacts.clear();
                        view.selected_artifact = None;
                        view.artifact_preview = ArtifactPreviewState::None;
                        if view.resyncing_session == Some(session_id) {
                            view.resyncing_session = None;
                        }
                        if let Some(summary) = view.session_summary_map.get(&session_id) {
                            view.session = session_info_from_summary(summary);
                        }
                    }
                }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn is_session_selected(&self, session_id: SessionId) -> bool {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
            == Some(session_id)
    }

    fn reset_thread_state(&mut self) {
        self.session_turns.clear();
        self.session_history_cursor = None;
        self.session_history_has_more = false;
        self.session_history_loading = false;
        self.session_turn_tools.clear();
        self.thread_items.clear();
        self.thread_list_state.reset(0);
        self.thread_list_len = 0;
        self.thread_auto_follow = true;
        self.new_thread_item_count = 0;
        self.sticky_turn_header = None;
        self.sticky_turn_header_at_top = true;
        self.expanded_turn_headers.clear();
        self.expanded_messages.clear();
        self.expanded_turn_details.clear();
        self.expanded_tools.clear();
        self.turn_tools_loading.clear();
    }

    pub(crate) fn rebuild_thread_items(&mut self) {
        let items = build_thread_list_items(
            &self.session_turns,
            &self.messages,
            &self.session_turn_tools,
            &self.session_events,
        );
        let items = filter_thread_list_items(items, self.verbosity);
        let old_len = self.thread_list_len;
        self.apply_thread_items(items);
        let new_len = self.thread_list_len;
        if self.thread_auto_follow {
            self.new_thread_item_count = 0;
            self.scroll_thread_to_bottom();
        } else if new_len > old_len {
            self.new_thread_item_count =
                self.new_thread_item_count.saturating_add(new_len - old_len);
        }
        if new_len == 0 {
            self.sticky_turn_header = None;
            self.sticky_turn_header_at_top = true;
        }
    }

    fn apply_thread_items(&mut self, items: Vec<ThreadListItem>) {
        let old_items = std::mem::take(&mut self.thread_items);
        let old_len = old_items.len();
        let new_len = items.len();

        let mut start = 0;
        while start < old_len
            && start < new_len
            && old_items[start].id() == items[start].id()
        {
            start += 1;
        }

        let mut end_old = old_len;
        let mut end_new = new_len;
        while end_old > start
            && end_new > start
            && old_items[end_old - 1].id() == items[end_new - 1].id()
        {
            end_old -= 1;
            end_new -= 1;
        }

        if !(start == end_old && start == end_new) {
            self.thread_list_state
                .splice(start..end_old, end_new - start);
        }

        self.thread_items = items;
        self.thread_list_len = new_len;
    }

    fn invalidate_thread_item(&mut self, id: &str) {
        let Some(index) = self.thread_items.iter().position(|item| item.id() == id) else {
            return;
        };
        self.thread_list_state.splice(index..index + 1, 1);
    }

    fn invalidate_thread_items_for_attachment(&mut self, cache_key: &str) {
        let indexes = self
            .thread_items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if thread_item_contains_attachment(item, cache_key) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for index in indexes {
            self.thread_list_state.splice(index..index + 1, 1);
        }
    }

    fn update_sticky_turn_header(&mut self, visible_range: std::ops::Range<usize>) {
        if self.thread_items.is_empty() {
            self.sticky_turn_header = None;
            self.sticky_turn_header_at_top = true;
            return;
        }
        let start = visible_range
            .start
            .min(self.thread_items.len().saturating_sub(1));
        self.sticky_turn_header_at_top = matches!(
            self.thread_items.get(start),
            Some(ThreadListItem::TurnHeader { .. })
        );
        let mut header: Option<WorkbenchTurnHeader> = None;
        for item in self.thread_items.iter().take(start + 1) {
            if let ThreadListItem::TurnHeader { header: h, .. } = item {
                header = Some(h.clone());
            }
        }
        self.sticky_turn_header = header;
    }
}

fn merge_session_turns(
    head: Option<&SessionHead>,
    history: Option<&SessionHistoryPage>,
) -> Vec<SessionTurn> {
    let mut turns: HashMap<TurnId, SessionTurn> = HashMap::new();
    if let Some(history) = history {
        for turn in &history.turns {
            turns.insert(turn.turn_id, turn.clone());
        }
    }
    if let Some(head) = head {
        for turn in &head.turns {
            turns.insert(turn.turn_id, turn.clone());
        }
    }
    let mut out: Vec<SessionTurn> = turns.into_values().collect();
    out.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.updated_at.cmp(&b.updated_at))
    });
    out
}

fn build_turn_tool_snapshots(head: Option<&SessionHead>) -> HashMap<TurnId, Vec<TurnToolSnapshot>> {
    let mut out: HashMap<TurnId, Vec<TurnToolSnapshot>> = HashMap::new();
    let Some(head) = head else {
        return out;
    };
    for summary in &head.tool_summaries {
        out.entry(summary.turn_id)
            .or_default()
            .push(TurnToolSnapshot::from_summary(summary));
    }
    out
}

fn filter_thread_list_items(
    items: Vec<ThreadListItem>,
    verbosity: SessionViewVerbosity,
) -> Vec<ThreadListItem> {
    if !matches!(verbosity, SessionViewVerbosity::Terse) {
        return items;
    }
    items
        .into_iter()
        .filter(|item| match item {
            ThreadListItem::Item(ThreadItem::Tool(_))
            | ThreadListItem::Item(ThreadItem::ToolGroup { .. })
            | ThreadListItem::Item(ThreadItem::Thought { .. }) => false,
            _ => true,
        })
        .collect()
}

fn thread_item_contains_attachment(item: &ThreadListItem, cache_key: &str) -> bool {
    match item {
        ThreadListItem::TurnHeader { header, .. } => header
            .attachments
            .iter()
            .any(|attachment| attachment_cache_key(attachment) == cache_key),
        ThreadListItem::Item(ThreadItem::Message { attachments, .. }) => attachments
            .iter()
            .any(|attachment| attachment_cache_key(attachment) == cache_key),
        _ => false,
    }
}

fn inline_attachment_image(mime_type: &str, data_base64: &str) -> Option<Arc<Image>> {
    let format = ImageFormat::from_mime_type(mime_type).unwrap_or(ImageFormat::Png);
    let bytes = STANDARD.decode(data_base64).ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
