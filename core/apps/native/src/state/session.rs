use gpui::{ClickEvent, Context, ListOffset, Window, px};
use gpui_tokio::Tokio;

use ctx_core::ids::SessionId;
use ctx_core::models::{Artifact, SessionEvent, SessionHead, SessionHistoryPage};

use super::{ArtifactPreviewState, ShellView};
use super::super::models::{
    build_message_items, session_info_from_head, session_info_from_summary, MessageItem,
    SessionInfo,
};

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

const MESSAGE_PLACEHOLDER_TEXTS: [&str; 3] = [
    "Loading session messages...",
    "No messages yet. Create one to begin.",
    "Unable to load session details.",
];

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

    pub(crate) fn on_new_messages_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message_auto_follow = true;
        self.new_message_count = 0;
        self.scroll_messages_to_bottom();
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

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(session_id) => {
                        view.load_session_details(session_id, cx);
                    }
                    Err(_) => {
                        view.push_message(MessageItem::new("assistant", action.failure_message()));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn ensure_message_list_handler(&mut self, cx: &mut Context<Self>) {
        if self.message_list_handler_set {
            return;
        }
        let view_handle = cx.entity();
        self.message_list_state
            .set_scroll_handler(move |event, _window, cx| {
                let _ = view_handle.update(cx, |view, cx| {
                    let auto_follow = !event.is_scrolled;
                    if view.message_auto_follow != auto_follow
                        || (auto_follow && view.new_message_count > 0)
                    {
                        view.message_auto_follow = auto_follow;
                        if auto_follow {
                            view.new_message_count = 0;
                        }
                        cx.notify();
                    }
                });
            });
        self.message_list_handler_set = true;
    }

    pub(super) fn replace_messages(&mut self, messages: Vec<MessageItem>) {
        self.messages = messages;
        self.message_list_len = self.messages.len();
        self.message_list_state.reset(self.message_list_len);
        self.message_auto_follow = true;
        self.new_message_count = 0;
    }

    fn clear_placeholder_messages(&mut self) {
        if self.messages.len() != 1 {
            return;
        }
        let content = self.messages[0].content.as_str();
        if MESSAGE_PLACEHOLDER_TEXTS
            .iter()
            .any(|placeholder| placeholder == &content)
        {
            self.messages.clear();
            self.message_list_state.reset(0);
            self.message_list_len = 0;
        }
    }

    pub(super) fn push_message(&mut self, message: MessageItem) {
        self.clear_placeholder_messages();
        let old_len = self.message_list_len;
        self.messages.push(message);
        let new_len = self.messages.len();
        if new_len != old_len {
            self.message_list_state.splice(old_len..old_len, new_len - old_len);
            self.message_list_len = new_len;
        }
        if self.message_auto_follow {
            self.scroll_messages_to_bottom();
        } else {
            self.new_message_count = self.new_message_count.saturating_add(new_len - old_len);
        }
    }

    fn scroll_messages_to_bottom(&mut self) {
        let count = self.messages.len();
        self.message_list_state.scroll_to(ListOffset {
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
        if let Some(summary) = self.session_summary_map.get(&session_id) {
            self.composer_provider_id = Some(summary.session.provider_id.clone());
            self.composer_model_id = Some(summary.session.model_id.clone());
        }
        self.replace_messages(vec![MessageItem::new(
            "assistant",
            "Loading session messages...",
        )]);
        self.artifacts.clear();
        self.artifact_preview = ArtifactPreviewState::None;
        self.session_events.clear();
        self.selected_artifact = None;
        self.resyncing_session = None;
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
                        if view.resyncing_session == Some(session_id) {
                            view.resyncing_session = None;
                        }
                    }
                    Err(_) => {
                        view.replace_messages(vec![MessageItem::new(
                            "assistant",
                            "Unable to load session details.",
                        )]);
                        view.session_events.clear();
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
        })
        .detach();
    }

    pub(super) fn is_session_selected(&self, session_id: SessionId) -> bool {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
            == Some(session_id)
    }
}
