use std::ops::Range;

use gpui::{ClickEvent, ClipboardItem, Context, KeyDownEvent, Window};
use gpui_tokio::Tokio;

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
                        view.push_message(MessageItem::new(
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
}
