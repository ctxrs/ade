use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use gpui::{AsyncApp, ClickEvent, ClipboardItem, Context, KeyDownEvent, WeakEntity, Window};
use gpui_tokio::Tokio;

use ctx_core::models::MessageAttachment;

use super::ShellView;
use super::super::models::MessageItem;

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

impl ShellView {
    pub(crate) fn focus_composer(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_focus.focus(window, cx);
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

    pub(crate) fn focus_composer_attachment(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_attachment_focus.focus(window, cx);
    }

    pub(crate) fn on_composer_attachment_key_down(
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
                    self.add_attachment_from_input(cx);
                    return;
                }
                "c" => {
                    self.copy_attachment_input(cx);
                    return;
                }
                "x" => {
                    self.cut_attachment_input(cx);
                    return;
                }
                "v" => {
                    self.paste_attachment_input(cx);
                    return;
                }
                "a" => {
                    self.composer_attachment_input.select_all();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match event.keystroke.key.as_str() {
            "enter" => {
                self.add_attachment_from_input(cx);
            }
            "backspace" => {
                if self.composer_attachment_input.delete_backward() {
                    cx.notify();
                }
            }
            "delete" => {
                if self.composer_attachment_input.delete_forward() {
                    cx.notify();
                }
            }
            "left" => {
                self.composer_attachment_input.move_left(modifiers.shift);
                cx.notify();
            }
            "right" => {
                self.composer_attachment_input.move_right(modifiers.shift);
                cx.notify();
            }
            "home" => {
                self.composer_attachment_input.move_home(modifiers.shift);
                cx.notify();
            }
            "end" => {
                self.composer_attachment_input.move_end(modifiers.shift);
                cx.notify();
            }
            _ => {
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                if let Some(text) = event.keystroke.key_char.as_ref() {
                    if text != "\n" && text != "\r" && text != "\t" {
                        self.composer_attachment_input.insert_text(text);
                        cx.notify();
                    }
                }
            }
        }
    }

    pub(crate) fn on_add_attachment_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_attachment_from_input(cx);
    }

    pub(crate) fn remove_composer_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.composer_attachments.len() {
            return;
        }
        self.composer_attachments.remove(index);
        cx.notify();
    }

    pub(crate) fn toggle_composer_provider_menu(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_provider_menu_open = !self.composer_provider_menu_open;
        if self.composer_provider_menu_open {
            self.composer_model_menu_open = false;
        }
        cx.notify();
    }

    pub(crate) fn toggle_composer_model_menu(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_model_menu_open = !self.composer_model_menu_open;
        if self.composer_model_menu_open {
            self.composer_provider_menu_open = false;
        }
        cx.notify();
    }

    pub(crate) fn select_composer_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        self.composer_provider_id = Some(provider_id.clone());
        self.composer_provider_menu_open = false;
        self.composer_model_menu_open = false;
        let models = self.model_ids_for_provider(&provider_id);
        if !models.is_empty()
            && self
                .composer_model_id
                .as_ref()
                .map(|id| !models.contains(id))
                .unwrap_or(true)
        {
            self.composer_model_id = Some(models[0].clone());
        }
        cx.notify();
    }

    pub(crate) fn select_composer_model(&mut self, model_id: String, cx: &mut Context<Self>) {
        self.composer_model_id = Some(model_id.clone());
        self.composer_model_menu_open = false;
        cx.notify();

        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let model_id_for_task = model_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client
                .set_session_model(session_id, &model_id_for_task)
                .await?;
            Ok(session_id)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(session_id) => view.load_session_details(session_id, cx),
                        Err(_) => {
                            view.messages.push(MessageItem::new(
                                "assistant",
                                "Unable to update session model.",
                            ));
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn composer_provider_options(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|provider| provider.name.clone())
            .collect()
    }

    pub(crate) fn composer_model_options(&self) -> Vec<String> {
        let Some(provider_id) = self.composer_provider_id.as_deref() else {
            return Vec::new();
        };
        let mut models = self.model_ids_for_provider(provider_id);
        if let Some(current) = self.composer_model_id.as_ref() {
            if !models.contains(current) {
                models.insert(0, current.clone());
            }
        }
        models
    }

    pub(super) fn sync_composer_defaults(&mut self) {
        let default_provider = self
            .composer_provider_id
            .clone()
            .filter(|id| self.providers.iter().any(|provider| provider.name == *id))
            .or_else(|| {
                self.session_summary_map
                    .values()
                    .next()
                    .map(|summary| summary.session.provider_id.clone())
            })
            .or_else(|| self.providers.first().map(|provider| provider.name.clone()));
        self.composer_provider_id = default_provider;

        let Some(provider_id) = self.composer_provider_id.clone() else {
            self.composer_model_id = None;
            return;
        };
        let models = self.model_ids_for_provider(&provider_id);
        if models.is_empty() {
            if self.composer_model_id.is_none() {
                if let Some(summary) = self
                    .session_summary_map
                    .values()
                    .find(|summary| summary.session.provider_id == provider_id)
                {
                    self.composer_model_id = Some(summary.session.model_id.clone());
                }
            }
        } else if self
            .composer_model_id
            .as_ref()
            .map(|id| !models.contains(id))
            .unwrap_or(true)
        {
            self.composer_model_id = Some(models[0].clone());
        }
    }

    pub(crate) fn on_send_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.send_composer_message(cx);
    }

    fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let content = self.composer.text().trim().to_string();
        if content.is_empty() && self.composer_attachments.is_empty() {
            return;
        }
        let history_entry = content.clone();
        let attachments = self.composer_attachments.clone();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let request = ctx_client::PostMessageRequest {
                content,
                delivery: None,
                attachments,
            };
            client.post_message(session_id, &request).await?;
            Ok(session_id)
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(session_id) => {
                            view.composer.push_history(history_entry);
                            view.composer.clear();
                            view.composer_attachments.clear();
                            view.composer_attachment_input.clear();
                            view.composer_notice = None;
                            view.load_session_details(session_id, cx);
                        }
                        Err(_) => {
                            view.push_message(MessageItem::new(
                                "assistant",
                                "Unable to send message.",
                            ));
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
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

    fn copy_attachment_input(&mut self, cx: &mut Context<Self>) {
        let selection = self.composer_attachment_input.selection_range();
        let text = if selection.is_empty() {
            self.composer_attachment_input.text().to_string()
        } else {
            self.composer_attachment_input
                .text()
                .get(selection)
                .unwrap_or_default()
                .to_string()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut_attachment_input(&mut self, cx: &mut Context<Self>) {
        let selection = self.composer_attachment_input.selection_range();
        let text = if selection.is_empty() {
            self.composer_attachment_input.text().to_string()
        } else {
            self.composer_attachment_input
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
            self.composer_attachment_input.clear();
        } else {
            self.composer_attachment_input.delete_backward();
        }
        cx.notify();
    }

    fn paste_attachment_input(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if !text.is_empty() {
                self.composer_attachment_input.insert_text(&text);
                cx.notify();
            }
        }
    }

    fn add_attachment_from_input(&mut self, cx: &mut Context<Self>) {
        let raw = self.composer_attachment_input.text().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let path = PathBuf::from(raw.clone());
        let display_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.to_string())
            .unwrap_or(raw);
        let mime_type = guess_mime_type(&path);
        if !mime_type.starts_with("image/") {
            self.composer_notice = Some("Only image attachments are supported for now.".to_string());
            cx.notify();
            return;
        }

        self.composer_notice = None;
        let task = Tokio::spawn_result(cx, async move {
            let bytes = tokio::fs::read(&path).await?;
            let data_base64 = STANDARD.encode(&bytes);
            Ok((data_base64, mime_type.to_string(), display_name))
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok((data_base64, mime_type, name)) => {
                            view.composer_attachments.push(MessageAttachment::Image {
                                mime_type,
                                data_base64,
                                name: Some(name),
                            });
                            view.composer_attachment_input.clear();
                            view.composer_notice = None;
                        }
                        Err(err) => {
                            view.composer_notice = Some(format!("Attachment failed: {err}"));
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn model_ids_for_provider(&self, provider_id: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        for summary in self.session_summary_map.values() {
            if summary.session.provider_id == provider_id {
                seen.insert(summary.session.model_id.clone());
            }
        }
        seen.into_iter().collect()
    }
}

fn guess_mime_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
}
