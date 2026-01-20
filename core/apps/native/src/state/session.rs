use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};

use gpui::{
    AsyncApp, ClickEvent, ClipboardItem, Context, Image, ImageFormat, ListAlignment, ListOffset,
    ListState, WeakEntity, Window, px,
};
use gpui_tokio::Tokio;
use serde_json::Value;

use ctx_core::ids::{MessageId, SessionId, TurnId};
use ctx_core::models::{
    MessageRole, SessionEvent, SessionHeadSnapshot, SessionHistoryPage, SessionSnapshot,
    SessionState, SessionTurn, SessionTurnStatus,
};
use ctx_client;

use super::{ArtifactPreviewState, ContextWindowInfo, ShellView};
use super::super::models::{
    attachment_cache_key, build_message_items, build_thread_list_items, message_item_from_model,
    session_info_from_head, session_info_from_summary, MessageAttachment, MessageItem,
    SessionInfo, ThreadItem, ThreadListItem, ThreadToolItem, TurnToolSnapshot, WorkbenchTurnHeader,
};
use super::SessionViewVerbosity;

struct SessionLoadResult {
    session_id: SessionId,
    session_snapshot: Option<SessionSnapshot>,
}

enum HistoryLoadResult {
    Cached(SessionHistoryPage),
    Fetched(SessionHistoryPage),
}

#[derive(Clone, Default)]
pub(crate) struct SessionThreadCache {
    pub(super) messages: Vec<MessageItem>,
    pub(super) session_turns: Vec<SessionTurn>,
    pub(super) session_turn_tools: HashMap<TurnId, Vec<TurnToolSnapshot>>,
    pub(super) session_events: Vec<SessionEvent>,
    pub(super) history_cursor: Option<i64>,
    pub(super) has_more_history: bool,
}

impl SessionThreadCache {
    pub(super) fn from_snapshot(head: &SessionHeadSnapshot) -> Self {
        let messages = head
            .messages
            .iter()
            .map(message_item_from_model)
            .collect::<Vec<_>>();
        let session_turns = head.turns.clone();
        let session_turn_tools = build_turn_tool_snapshots(Some(head));
        let session_events = head.events.clone();
        let history_cursor = head
            .history_cursor
            .or_else(|| head.turns.first().and_then(|turn| turn.start_seq.or(turn.end_seq)));
        let has_more_history = head.has_more_history || head.has_more_turns;
        Self {
            messages,
            session_turns,
            session_turn_tools,
            session_events,
            history_cursor,
            has_more_history,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(crate) struct ThreadRenderCacheKey {
    session_id: SessionId,
    snapshot_rev: i64,
    last_event_seq: i64,
}

#[derive(Clone, Copy)]
pub(super) enum ThreadRenderUpdateKind {
    Default,
    History,
}

#[derive(Clone)]
pub(super) struct ThreadRenderModel {
    key: ThreadRenderCacheKey,
    items: Arc<Vec<ThreadListItem>>,
    update_kind: ThreadRenderUpdateKind,
}

pub(crate) struct ThreadRenderCache {
    entries: HashMap<ThreadRenderCacheKey, Arc<ThreadRenderModel>>,
    order: VecDeque<ThreadRenderCacheKey>,
    capacity: usize,
}

impl ThreadRenderCache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    pub(super) fn contains(&self, key: &ThreadRenderCacheKey) -> bool {
        self.entries.contains_key(key)
    }

    pub(super) fn get(&mut self, key: &ThreadRenderCacheKey) -> Option<Arc<ThreadRenderModel>> {
        let model = self.entries.get(key).cloned();
        if model.is_some() {
            self.touch(*key);
        }
        model
    }

    pub(super) fn insert(&mut self, model: Arc<ThreadRenderModel>) {
        let key = model.key;
        self.entries.insert(key, model);
        self.touch(key);
        self.evict_if_needed();
    }

    pub(super) fn remove_session(&mut self, session_id: SessionId) {
        self.entries
            .retain(|key, _| key.session_id != session_id);
        self.order.retain(|key| key.session_id != session_id);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    fn touch(&mut self, key: ThreadRenderCacheKey) {
        if let Some(index) = self.order.iter().position(|existing| *existing == key) {
            self.order.remove(index);
        }
        self.order.push_back(key);
    }

    fn evict_if_needed(&mut self) {
        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionThreadViewState {
    list_state: ListState,
    thread_auto_follow: bool,
    new_thread_item_count: usize,
    sticky_turn_header: Option<WorkbenchTurnHeader>,
    sticky_turn_header_at_top: bool,
    expanded_turn_headers: Arc<HashMap<String, bool>>,
    expanded_messages: Arc<HashMap<String, bool>>,
    expanded_turn_details: Arc<HashMap<String, bool>>,
    expanded_tools: Arc<HashMap<String, bool>>,
    turn_tools_loading: HashSet<TurnId>,
}

struct ThreadRenderSource {
    key: ThreadRenderCacheKey,
    turns: Vec<SessionTurn>,
    messages: Vec<MessageItem>,
    tools: HashMap<TurnId, Vec<TurnToolSnapshot>>,
    events: Vec<SessionEvent>,
    verbosity: SessionViewVerbosity,
    update_kind: ThreadRenderUpdateKind,
}

const COPIED_TIMEOUT: Duration = Duration::from_millis(1_000);
const SESSION_HISTORY_PAGE_LIMIT: u32 = 60;
const SESSION_HISTORY_PREFETCH_THRESHOLD: usize = 6;
const LAYOUT_HASH_SEED: u64 = 0xcbf29ce484222325;
const LAYOUT_HASH_PRIME: u64 = 0x100000001b3;
pub(crate) const THREAD_RENDER_CACHE_LIMIT: usize = 24;

fn fresh_thread_list_state() -> ListState {
    ListState::new(0, ListAlignment::Bottom, px(160.0))
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
        let cache = self.ats_cache.clone();
        let task = Tokio::spawn_result(cx, async move {
            if let Ok(Some(history)) = cache
                .load_session_history_page(session_id, before_seq, SESSION_HISTORY_PAGE_LIMIT)
                .await
            {
                return Ok(HistoryLoadResult::Cached(history));
            }

            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let history = client
                .get_session_history(session_id, Some(before_seq), Some(SESSION_HISTORY_PAGE_LIMIT))
                .await?;
            let _ = cache
                .save_session_history_page(before_seq, SESSION_HISTORY_PAGE_LIMIT, &history)
                .await;
            Ok(HistoryLoadResult::Fetched(history))
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
                        Ok(HistoryLoadResult::Cached(history))
                        | Ok(HistoryLoadResult::Fetched(history)) => {
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

        if let Some(session_id) = self.selected_session_id() {
            self.cache_session_thread_state(session_id);
            self.invalidate_thread_render_cache(session_id);
            self.schedule_thread_render_model(session_id, ThreadRenderUpdateKind::History, cx);
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
        Arc::make_mut(&mut self.expanded_turn_headers).insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_message(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_messages.get(&id).copied().unwrap_or(false);
        Arc::make_mut(&mut self.expanded_messages).insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_turn_details(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_turn_details.get(&id).copied().unwrap_or(false);
        Arc::make_mut(&mut self.expanded_turn_details).insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(crate) fn on_toggle_tool(&mut self, id: String, cx: &mut Context<Self>) {
        let expanded = self.expanded_tools.get(&id).copied().unwrap_or(false);
        Arc::make_mut(&mut self.expanded_tools).insert(id.clone(), !expanded);
        self.invalidate_thread_item(&id);
        cx.notify();
    }

    pub(super) fn replace_messages(&mut self, messages: Vec<MessageItem>, cx: &mut Context<Self>) {
        self.messages = messages;
        self.prefetch_attachment_images(cx);
        self.thread_auto_follow = true;
        self.new_thread_item_count = 0;
        if let Some(session_id) = self.selected_session_id() {
            self.schedule_thread_render_model(session_id, ThreadRenderUpdateKind::Default, cx);
        }
    }

    pub(super) fn cache_session_thread_state(&mut self, session_id: SessionId) {
        self.session_thread_cache.insert(
            session_id,
            SessionThreadCache {
                messages: self.messages.clone(),
                session_turns: self.session_turns.clone(),
                session_turn_tools: self.session_turn_tools.clone(),
                session_events: self.session_events.clone(),
                history_cursor: self.session_history_cursor,
                has_more_history: self.session_history_has_more,
            },
        );
    }

    pub(super) fn apply_cached_thread_data(
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
        self.session_history_cursor = cache.history_cursor;
        self.session_history_has_more = cache.has_more_history;
        self.session_history_loading = false;
        self.messages = cache.messages.clone();
        self.prefetch_attachment_images(cx);
        true
    }

    fn apply_empty_thread_state(&mut self, cx: &mut Context<Self>) {
        self.session_turns.clear();
        self.session_turn_tools.clear();
        self.session_events.clear();
        self.session_history_cursor = None;
        self.session_history_has_more = false;
        self.session_history_loading = false;
        self.messages.clear();
        self.prefetch_attachment_images(cx);
        self.reset_thread_view_state();
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
        if let Some(session_id) = self.selected_session_id() {
            self.schedule_thread_render_model(session_id, ThreadRenderUpdateKind::Default, cx);
        }
    }

    pub(super) fn remove_message_by_id(
        &mut self,
        message_id: MessageId,
        cx: &mut Context<Self>,
    ) {
        let before = self.messages.len();
        self.messages
            .retain(|message| message.id != Some(message_id));
        if self.messages.len() == before {
            return;
        }
        if let Some(session_id) = self.selected_session_id() {
            self.schedule_thread_render_model(session_id, ThreadRenderUpdateKind::Default, cx);
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

    fn thread_render_cache_key(&self, session_id: SessionId) -> Option<ThreadRenderCacheKey> {
        let snapshot_rev = self.active_snapshot_rev?;
        let last_event_seq = self.session_last_event_seq.get(&session_id).copied()?;
        Some(ThreadRenderCacheKey {
            session_id,
            snapshot_rev,
            last_event_seq,
        })
    }

    fn cache_thread_view_state(&mut self, session_id: SessionId) {
        self.session_thread_view_state.insert(
            session_id,
            SessionThreadViewState {
                list_state: self.thread_list_state.clone(),
                thread_auto_follow: self.thread_auto_follow,
                new_thread_item_count: self.new_thread_item_count,
                sticky_turn_header: self.sticky_turn_header.clone(),
                sticky_turn_header_at_top: self.sticky_turn_header_at_top,
                expanded_turn_headers: self.expanded_turn_headers.clone(),
                expanded_messages: self.expanded_messages.clone(),
                expanded_turn_details: self.expanded_turn_details.clone(),
                expanded_tools: self.expanded_tools.clone(),
                turn_tools_loading: self.turn_tools_loading.clone(),
            },
        );
    }

    fn apply_thread_view_state(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cache) = self.session_thread_view_state.get(&session_id) else {
            self.reset_thread_view_state();
            self.ensure_thread_list_handler(cx);
            return false;
        };
        self.thread_list_state = cache.list_state.clone();
        self.thread_auto_follow = cache.thread_auto_follow;
        self.new_thread_item_count = cache.new_thread_item_count;
        self.sticky_turn_header = cache.sticky_turn_header.clone();
        self.sticky_turn_header_at_top = cache.sticky_turn_header_at_top;
        self.expanded_turn_headers = cache.expanded_turn_headers.clone();
        self.expanded_messages = cache.expanded_messages.clone();
        self.expanded_turn_details = cache.expanded_turn_details.clone();
        self.expanded_tools = cache.expanded_tools.clone();
        self.turn_tools_loading = cache.turn_tools_loading.clone();
        self.thread_list_handler_set = false;
        self.ensure_thread_list_handler(cx);
        true
    }

    fn thread_render_source(
        &self,
        session_id: SessionId,
        update_kind: ThreadRenderUpdateKind,
    ) -> Option<ThreadRenderSource> {
        let key = self.thread_render_cache_key(session_id)?;
        let (turns, messages, tools, events) = if self.is_session_selected(session_id) {
            (
                self.session_turns.clone(),
                self.messages.clone(),
                self.session_turn_tools.clone(),
                self.session_events.clone(),
            )
        } else {
            let cache = self.session_thread_cache.get(&session_id)?;
            (
                cache.session_turns.clone(),
                cache.messages.clone(),
                cache.session_turn_tools.clone(),
                cache.session_events.clone(),
            )
        };
        Some(ThreadRenderSource {
            key,
            turns,
            messages,
            tools,
            events,
            verbosity: self.verbosity,
            update_kind,
        })
    }

    pub(super) fn schedule_thread_render_model(
        &mut self,
        session_id: SessionId,
        update_kind: ThreadRenderUpdateKind,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.thread_render_source(session_id, update_kind) else {
            return;
        };
        if self.thread_render_cache.contains(&source.key) {
            return;
        }
        if self.thread_render_inflight.contains_key(&session_id) {
            return;
        }
        self.spawn_thread_render_model(source, cx);
    }

    fn spawn_thread_render_model(&mut self, source: ThreadRenderSource, cx: &mut Context<Self>) {
        let ThreadRenderSource {
            key,
            turns,
            messages,
            tools,
            events,
            verbosity,
            update_kind,
        } = source;
        self.thread_render_inflight.insert(key.session_id, key);

        let task = Tokio::spawn_result(cx, async move {
            let items = build_thread_list_items(&turns, &messages, &tools, &events);
            let items = filter_thread_list_items(items, verbosity);
            Ok(ThreadRenderModel {
                key,
                items: Arc::new(items),
                update_kind,
            })
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.thread_render_inflight.remove(&key.session_id);
                    let Ok(model) = result else {
                        view.schedule_thread_render_model(
                            key.session_id,
                            ThreadRenderUpdateKind::Default,
                            cx,
                        );
                        return;
                    };
                    if view.thread_render_cache_key(model.key.session_id) != Some(model.key) {
                        view.schedule_thread_render_model(
                            model.key.session_id,
                            ThreadRenderUpdateKind::Default,
                            cx,
                        );
                        return;
                    }
                    let model = Arc::new(model);
                    view.thread_render_cache.insert(model.clone());
                    if view.is_session_selected(model.key.session_id) {
                        view.apply_thread_render_model(model, cx);
                    }
                    view.schedule_thread_render_model(
                        key.session_id,
                        ThreadRenderUpdateKind::Default,
                        cx,
                    );
                })
                .ok();
            }
        })
        .detach();
    }

    fn apply_thread_render_model(&mut self, model: Arc<ThreadRenderModel>, cx: &mut Context<Self>) {
        let update_new_items = matches!(model.update_kind, ThreadRenderUpdateKind::Default);
        self.apply_thread_items(model.items.clone(), update_new_items);
        cx.notify();
    }

    fn apply_cached_thread_render_model(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(key) = self.thread_render_cache_key(session_id) else {
            return false;
        };
        let Some(model) = self.thread_render_cache.get(&key) else {
            return false;
        };
        self.apply_thread_items(model.items.clone(), false);
        cx.notify();
        true
    }

    fn invalidate_thread_render_cache(&mut self, session_id: SessionId) {
        self.thread_render_cache.remove_session(session_id);
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
        let prev_session_id = self.selected_session_id();
        if prev_session_id == Some(session_id) {
            return;
        }
        if let Some(prev_session_id) = prev_session_id {
            self.cache_thread_view_state(prev_session_id);
        }
        self.selected_session = Some(index);
        self.new_task_mode = false;
        self.new_task_mode_locked = false;
        self.composer_focus_pending = false;
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
        self.artifacts.clear();
        self.artifacts_session_id = None;
        self.artifact_preview = ArtifactPreviewState::None;
        self.clear_artifact_prefetch_cache();
        self.session_events.clear();
        self.selected_artifact = None;
        self.resyncing_session = None;
        let applied_view_state = self.apply_thread_view_state(session_id, cx);
        let used_cache = self.apply_cached_thread_data(session_id, cx);
        if !used_cache {
            self.apply_empty_thread_state(cx);
        } else if !self.apply_cached_thread_render_model(session_id, cx) && applied_view_state {
            self.clear_thread_items(true);
        }
        cx.notify();
        if !used_cache {
            self.load_session_details(session_id, cx);
        } else {
            self.load_session_state(session_id, cx);
            self.schedule_thread_render_model(session_id, ThreadRenderUpdateKind::Default, cx);
        }
    }

    pub(super) fn load_session_details(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            Ok(SessionLoadResult {
                session_id,
                session_snapshot: client
                    .get_session_snapshot(session_id, Some(40), Some(false))
                    .await
                    .ok(),
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
                            let snapshot = data.session_snapshot.as_ref();
                            view.session = snapshot
                                .map(|snapshot| session_info_from_head(&snapshot.head))
                                .or_else(|| {
                                    view.session_summary_map
                                        .get(&data.session_id)
                                        .map(session_info_from_summary)
                                })
                                .unwrap_or_else(SessionInfo::placeholder);
                            if let Some(snapshot) = snapshot {
                                view.composer_provider_id =
                                    Some(snapshot.head.session.provider_id.clone());
                                view.composer_model_id =
                                    Some(snapshot.head.session.model_id.clone());
                            }
                            if let Some(index) = view.selected_session {
                                if let Some(summary) = view.sessions.get_mut(index) {
                                    if summary.session_id == data.session_id {
                                        summary.status = view.session.status.clone();
                                        summary.title = view.session.title.clone();
                                    }
                                }
                            }
                            if let Some(snapshot) = snapshot {
                                let head = &snapshot.head;
                                let mut messages = build_message_items(Some(head), None);
                                if messages.is_empty() {
                                    messages.push(MessageItem::new(
                                        MessageRole::Assistant,
                                        "No messages yet. Create one to begin.",
                                    ));
                                }
                                view.session_history_cursor = head
                                    .history_cursor
                                    .or_else(|| {
                                        head.turns.first().and_then(|turn| {
                                            turn.start_seq.or(turn.end_seq)
                                        })
                                    });
                                view.session_history_has_more =
                                    head.has_more_history || head.has_more_turns;
                                view.session_history_loading = false;
                                view.session_events = head.events.clone();
                                view.session_turns = head.turns.clone();
                                view.session_turn_tools = build_turn_tool_snapshots(Some(head));
                                view.replace_messages(messages, cx);
                                view.cache_session_thread_state(data.session_id);
                                view.update_session_last_event_seq(
                                    head.session.id,
                                    head.last_event_seq,
                                );
                                view.update_session_head_meta(head.session.id, head);
                                view.persist_cached_session_head(head.session.id, cx);
                                if let Some(state) = snapshot.state.as_ref() {
                                    view.apply_session_state(data.session_id, state, cx);
                                } else {
                                    view.load_session_state(data.session_id, cx);
                                }
                                if view.resyncing_session == Some(session_id) {
                                    view.resyncing_session = None;
                                }
                            } else {
                                view.replace_messages(vec![MessageItem::new(
                                    MessageRole::Assistant,
                                    "Unable to load session details.",
                                )], cx);
                                view.session_events.clear();
                                view.session_turns.clear();
                                view.session_turn_tools.clear();
                                view.clear_thread_items(true);
                                view.session_history_cursor = None;
                                view.session_history_has_more = false;
                                view.session_history_loading = false;
                                if view.resyncing_session == Some(session_id) {
                                    view.resyncing_session = None;
                                }
                                if let Some(summary) = view.session_summary_map.get(&session_id) {
                                    view.session = session_info_from_summary(summary);
                                }
                            }
                        }
                        Err(_) => {
                            view.replace_messages(
                                vec![MessageItem::new(
                                    MessageRole::Assistant,
                                    "Unable to load session details.",
                                )],
                                cx,
                            );
                            view.session_events.clear();
                            view.session_turns.clear();
                            view.session_turn_tools.clear();
                            view.clear_thread_items(true);
                            view.session_history_cursor = None;
                            view.session_history_has_more = false;
                            view.session_history_loading = false;
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

    fn apply_session_state(
        &mut self,
        session_id: SessionId,
        state: &SessionState,
        cx: &mut Context<Self>,
    ) {
        self.session_state_cache.insert(session_id, state.clone());
        if !self.is_session_selected(session_id) {
            return;
        }
        self.apply_artifacts_update(session_id, state.artifacts.clone(), cx);
    }

    pub(super) fn load_session_state(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(state) = self.session_state_cache.get(&session_id).cloned() {
            self.apply_session_state(session_id, &state, cx);
            return;
        }
        if !self.session_state_loading.insert(session_id) {
            return;
        }

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let state = client.get_session_state(session_id).await.ok();
            Ok((session_id, state))
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    let Ok((session_id, state)) = result else {
                        view.session_state_loading.remove(&session_id);
                        return;
                    };
                    view.session_state_loading.remove(&session_id);
                    if let Some(state) = state {
                        view.apply_session_state(session_id, &state, cx);
                    } else {
                        view.load_session_artifacts(session_id, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn load_session_artifacts(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        if self.artifacts_session_id == Some(session_id) && !self.artifacts.is_empty() {
            return;
        }
        self.artifacts_session_id = Some(session_id);

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let artifacts = client
                .list_session_artifacts(session_id)
                .await
                .ok()
                .map(|items| items.into_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            Ok((session_id, artifacts))
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    let Ok((session_id, artifacts)) = result else {
                        return;
                    };
                    if view.artifacts_session_id != Some(session_id) {
                        return;
                    }
                    view.artifacts = artifacts;
                    view.selected_artifact = if view.artifacts.is_empty() {
                        None
                    } else {
                        Some(0)
                    };
                    view.load_artifact_preview(cx);
                    view.prefetch_artifacts(session_id, cx);
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

    fn reset_thread_view_state(&mut self) {
        self.thread_items = Arc::new(Vec::new());
        self.thread_item_layout_hashes.clear();
        self.thread_list_state = fresh_thread_list_state();
        self.thread_list_len = 0;
        self.thread_auto_follow = true;
        self.new_thread_item_count = 0;
        self.sticky_turn_header = None;
        self.sticky_turn_header_at_top = true;
        Arc::make_mut(&mut self.expanded_turn_headers).clear();
        Arc::make_mut(&mut self.expanded_messages).clear();
        Arc::make_mut(&mut self.expanded_turn_details).clear();
        Arc::make_mut(&mut self.expanded_tools).clear();
        self.turn_tools_loading.clear();
        self.thread_list_handler_set = false;
        self.hovered_turn_header = None;
        self.composer_context_window = None;
    }

    fn apply_thread_items(&mut self, items: Arc<Vec<ThreadListItem>>, update_new_items: bool) {
        let old_items = self.thread_items.clone();
        let old_layout_hashes = std::mem::take(&mut self.thread_item_layout_hashes);
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

        let mut layout_invalidations = Vec::new();
        let mut new_layout_hashes = HashMap::with_capacity(new_len);
        for (index, item) in items.iter().enumerate() {
            let layout_hash = self.thread_item_layout_hash(item);
            let id = item.id().to_string();
            if let Some(old_hash) = old_layout_hashes.get(&id) {
                if *old_hash != layout_hash {
                    layout_invalidations.push(index);
                }
            }
            new_layout_hashes.insert(id, layout_hash);
        }

        if !(start == end_old && start == end_new) {
            self.thread_list_state
                .splice(start..end_old, end_new - start);
        }

        self.thread_items = items;
        self.thread_list_len = new_len;
        self.thread_item_layout_hashes = new_layout_hashes;

        for index in layout_invalidations {
            self.thread_list_state.splice(index..index + 1, 1);
        }

        if update_new_items {
            if self.thread_auto_follow {
                self.new_thread_item_count = 0;
                self.scroll_thread_to_bottom();
            } else if new_len > old_len {
                self.new_thread_item_count =
                    self.new_thread_item_count.saturating_add(new_len - old_len);
            }
        }

        if new_len == 0 {
            self.sticky_turn_header = None;
            self.sticky_turn_header_at_top = true;
        }
        self.refresh_composer_context_window();
    }

    fn clear_thread_items(&mut self, reset_list_state: bool) {
        self.thread_items = Arc::new(Vec::new());
        self.thread_item_layout_hashes.clear();
        self.thread_list_len = 0;
        self.new_thread_item_count = 0;
        self.sticky_turn_header = None;
        self.sticky_turn_header_at_top = true;
        if reset_list_state {
            self.thread_list_state = fresh_thread_list_state();
            self.thread_list_handler_set = false;
        }
    }

    fn refresh_composer_context_window(&mut self) {
        self.composer_context_window = latest_context_window_info(&self.session_turns);
    }

    fn invalidate_thread_item(&mut self, id: &str) {
        let Some((index, item)) = self
            .thread_items
            .iter()
            .enumerate()
            .find(|(_, item)| item.id() == id)
        else {
            return;
        };
        let layout_hash = self.thread_item_layout_hash(item);
        self.thread_item_layout_hashes
            .insert(id.to_string(), layout_hash);
        self.thread_list_state.splice(index..index + 1, 1);
    }

    fn invalidate_thread_items_for_attachment(&mut self, cache_key: &str) {
        let updates = self
            .thread_items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if thread_item_contains_attachment(item, cache_key) {
                    Some((index, item.id().to_string(), self.thread_item_layout_hash(item)))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (index, id, layout_hash) in updates {
            self.thread_item_layout_hashes.insert(id, layout_hash);
            self.thread_list_state.splice(index..index + 1, 1);
        }
    }

    fn thread_item_layout_hash(&self, item: &ThreadListItem) -> u64 {
        let mut hash = LAYOUT_HASH_SEED;
        match item {
            ThreadListItem::TurnHeader { id, header } => {
                let lines = line_count(&header.plain_text);
                let is_long = lines > 4 || header.plain_text.len() > 280;
                let expanded = self
                    .expanded_turn_headers
                    .get(id)
                    .copied()
                    .unwrap_or(!is_long);
                hash_mix(&mut hash, 1);
                hash_mix(&mut hash, expanded as u64);
                hash_mix(&mut hash, is_long as u64);
                hash_mix(&mut hash, header.plain_text.len() as u64);
                hash_mix(&mut hash, lines as u64);
                hash_mix(&mut hash, header.content.trim().is_empty() as u64);
                hash_mix(&mut hash, self.attachments_layout_hash(&header.attachments));
            }
            ThreadListItem::Item(ThreadItem::Message {
                id,
                role,
                content,
                attachments,
                ..
            }) => {
                let lines = line_count(content);
                let is_long = lines > 20 || content.len() > 1500;
                let expanded = self.expanded_messages.get(id).copied().unwrap_or(!is_long);
                let role_tag = match role {
                    MessageRole::User => 1,
                    MessageRole::Assistant => 2,
                    MessageRole::System => 3,
                };
                hash_mix(&mut hash, 2);
                hash_mix(&mut hash, role_tag);
                hash_mix(&mut hash, expanded as u64);
                hash_mix(&mut hash, is_long as u64);
                hash_mix(&mut hash, content.len() as u64);
                hash_mix(&mut hash, lines as u64);
                hash_mix(&mut hash, self.attachments_layout_hash(attachments));
            }
            ThreadListItem::Item(ThreadItem::Assistant { content, .. }) => {
                let lines = line_count(content);
                hash_mix(&mut hash, 3);
                hash_mix(&mut hash, content.len() as u64);
                hash_mix(&mut hash, lines as u64);
            }
            ThreadListItem::Item(ThreadItem::Thought { content, .. }) => {
                let lines = line_count(content);
                hash_mix(&mut hash, 4);
                hash_mix(&mut hash, content.len() as u64);
                hash_mix(&mut hash, lines as u64);
            }
            ThreadListItem::Item(ThreadItem::TurnStatus {
                status,
                custom_status,
                assistant_messages_content,
                ..
            }) => {
                let status_tag = match status {
                    SessionTurnStatus::Queued => 1,
                    SessionTurnStatus::Running => 2,
                    SessionTurnStatus::Completed => 3,
                    SessionTurnStatus::Interrupted => 4,
                    SessionTurnStatus::Failed => 5,
                };
                let custom_len = custom_status.as_ref().map(|value| value.len()).unwrap_or(0);
                let has_copy_button = matches!(status, SessionTurnStatus::Completed)
                    && assistant_messages_content
                        .as_ref()
                        .map(|content| !content.trim().is_empty())
                        .unwrap_or(false);
                hash_mix(&mut hash, 5);
                hash_mix(&mut hash, status_tag);
                hash_mix(&mut hash, custom_len as u64);
                hash_mix(&mut hash, has_copy_button as u64);
            }
            ThreadListItem::Item(ThreadItem::Tool(tool)) => {
                let expanded = self.expanded_tools.get(&tool.id).copied().unwrap_or(false);
                hash_mix(&mut hash, 6);
                hash_mix(&mut hash, self.thread_tool_layout_hash(tool, expanded));
            }
            ThreadListItem::Item(ThreadItem::ToolGroup { id, tools, .. }) => {
                let expanded = self
                    .expanded_turn_details
                    .get(id)
                    .copied()
                    .unwrap_or(false);
                hash_mix(&mut hash, 7);
                hash_mix(&mut hash, expanded as u64);
                hash_mix(&mut hash, tools.len() as u64);
                if expanded {
                    for tool in tools {
                        let tool_expanded =
                            self.expanded_tools.get(&tool.id).copied().unwrap_or(false);
                        hash_mix(&mut hash, self.thread_tool_layout_hash(tool, tool_expanded));
                    }
                }
            }
            ThreadListItem::Item(ThreadItem::Spacer { .. }) => {
                hash_mix(&mut hash, 8);
            }
        }
        hash
    }

    fn attachments_layout_hash(&self, attachments: &[MessageAttachment]) -> u64 {
        let mut hash = LAYOUT_HASH_SEED;
        let mut loaded = 0u64;
        let mut loading = 0u64;
        let mut failed = 0u64;
        for attachment in attachments {
            match attachment {
                MessageAttachment::ImageRef { blob_id, .. } => {
                    if self.composer_attachment_loading.contains(blob_id) {
                        loading += 1;
                    } else if self.attachment_fetch_failed.contains(blob_id) {
                        failed += 1;
                    } else if self.composer_attachment_images.contains_key(blob_id) {
                        loaded += 1;
                    }
                }
                MessageAttachment::Image { .. } => {
                    loaded += 1;
                }
            }
        }
        hash_mix(&mut hash, attachments.len() as u64);
        hash_mix(&mut hash, loaded);
        hash_mix(&mut hash, loading);
        hash_mix(&mut hash, failed);
        hash
    }

    fn thread_tool_layout_hash(&self, tool: &ThreadToolItem, expanded: bool) -> u64 {
        let mut hash = LAYOUT_HASH_SEED;
        hash_mix(&mut hash, expanded as u64);
        hash_mix(&mut hash, tool.title.len() as u64);
        hash_mix(&mut hash, tool.status.len() as u64);
        if expanded {
            let input_len = tool
                .input
                .as_ref()
                .map(|value| value.to_string().len())
                .unwrap_or(0);
            hash_mix(&mut hash, tool.output_text.len() as u64);
            hash_mix(&mut hash, input_len as u64);
            hash_mix(&mut hash, tool.output_text.trim().is_empty() as u64);
            hash_mix(&mut hash, (input_len == 0) as u64);
        }
        hash
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

fn latest_context_window_info(turns: &[SessionTurn]) -> Option<ContextWindowInfo> {
    let mut latest: Option<&SessionTurn> = None;
    for turn in turns {
        if turn.metrics_json.is_none() {
            continue;
        }
        if latest
            .map(|existing| turn.updated_at >= existing.updated_at)
            .unwrap_or(true)
        {
            latest = Some(turn);
        }
    }
    let metrics = latest?.metrics_json.as_ref()?;
    normalize_context_window_metrics(metrics)
}

fn normalize_context_window_metrics(metrics: &Value) -> Option<ContextWindowInfo> {
    let window_tokens = pick_number(metrics, &[
        "context_window_tokens",
        "context_window_size",
        "context_window",
        "context_size",
        "window_tokens",
        "max_context_tokens",
        "max_tokens",
    ])?;
    if window_tokens <= 0.0 {
        return None;
    }

    let input_tokens = pick_number(metrics, &[
        "total_input_tokens",
        "input_tokens",
        "prompt_tokens",
        "input",
    ]);
    let output_tokens = pick_number(metrics, &[
        "total_output_tokens",
        "output_tokens",
        "completion_tokens",
        "output",
    ]);
    let context_tokens_estimate =
        pick_number(metrics, &["context_tokens_estimate", "context_tokens"]);

    let mut used_tokens = if input_tokens.is_some() || output_tokens.is_some() {
        Some(input_tokens.unwrap_or(0.0) + output_tokens.unwrap_or(0.0))
    } else {
        context_tokens_estimate
    };

    let mut remaining_tokens =
        pick_number(metrics, &["remaining_tokens_estimate", "remaining_tokens"]);
    let mut remaining_fraction =
        pick_number(metrics, &["remaining_fraction", "remaining_pct", "remaining_percent"]);

    if let Some(fraction) = remaining_fraction {
        if fraction > 1.0 {
            remaining_fraction = if fraction <= 100.0 {
                Some(fraction / 100.0)
            } else {
                None
            };
        }
    }

    if let Some(fraction) = remaining_fraction {
        remaining_fraction = Some(fraction.max(0.0).min(1.0));
    }

    if remaining_tokens.is_none() {
        if let Some(used) = used_tokens {
            remaining_tokens = Some((window_tokens - used).max(0.0));
        }
    }
    if used_tokens.is_none() {
        if let Some(remaining) = remaining_tokens {
            used_tokens = Some((window_tokens - remaining).max(0.0));
        }
    }
    if remaining_fraction.is_none() {
        if let Some(used) = used_tokens {
            remaining_fraction = Some((1.0 - used / window_tokens).max(0.0).min(1.0));
        }
    }
    if used_tokens.is_none() {
        if let Some(fraction) = remaining_fraction {
            used_tokens = Some((window_tokens * (1.0 - fraction)).max(0.0));
        }
    }

    Some(ContextWindowInfo {
        window_tokens: Some(window_tokens),
        used_tokens,
        remaining_tokens,
        remaining_fraction,
    })
}

fn pick_number(metrics: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = metrics.get(*key) {
            if let Some(parsed) = coerce_number(value) {
                return Some(parsed);
            }
        }
    }
    None
}

fn coerce_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(num) => num.as_f64(),
        Value::String(raw) => raw.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn hash_mix(state: &mut u64, value: u64) {
    *state ^= value;
    *state = state.wrapping_mul(LAYOUT_HASH_PRIME);
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 1;
    }
    text.as_bytes()
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

fn build_turn_tool_snapshots(
    head: Option<&SessionHeadSnapshot>,
) -> HashMap<TurnId, Vec<TurnToolSnapshot>> {
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
