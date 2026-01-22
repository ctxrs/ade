use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext, AsyncApp, Bounds, ClickEvent, Context, Corner, ExternalPaths, Image, ImageFormat,
    KeyDownEvent, KeyUpEvent, Pixels, Window, point, px, EntityInputHandler,
};
use gpui_component::input::{Escape, IndentInline, InputEvent, InputState, MoveDown, MoveUp};
use gpui_component::RopeExt;
use gpui_tokio::Tokio;
use tokio::time::sleep;

use ctx_client::{EnvTarget, InstallInfo, InstallProgressEvent, InstallStateKind, ProviderOptions};
use ctx_core::ids::{MessageId, SessionId};
use ctx_core::models::{MessageAttachment, MessageDelivery, MessageRole};
use ctx_providers::adapters::ProviderHealth;

use super::ShellView;
use super::super::models::MessageItem;

#[allow(dead_code)]
const COMPOSER_HISTORY_LIMIT: usize = 20;
const FILE_COMPLETION_LIMIT: u32 = 10;
const AUTOCOMPLETE_DEBOUNCE_MS: u64 = 120;
const MENU_MARGIN: f32 = 10.0;
const MENU_GAP: f32 = 8.0;
const HARN_MENU_GAP: f32 = 10.0;
const AUTOCOMPLETE_ROW_HEIGHT: f32 = 30.0;
const AUTOCOMPLETE_CONTAINER_EXTRA_Y: f32 = 14.0;
const AUTOCOMPLETE_MIN_WIDTH: f32 = 340.0;
const AUTOCOMPLETE_MAX_WIDTH: f32 = 520.0;
const AUTOCOMPLETE_MAX_HEIGHT: f32 = 360.0;
const AUTOCOMPLETE_PREVIEW_WIDTH: f32 = 240.0;
const AUTOCOMPLETE_PREVIEW_MAX_HEIGHT: f32 = 285.0;
pub(crate) const MAX_TRACKS_PER_PROVIDER: usize = 4;

pub(crate) struct ComposerState {
    text: String,
    cursor: usize,
    selection_anchor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<String>,
}

#[allow(dead_code)]
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

    #[allow(dead_code)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkbenchModeId {
    Default,
    Research,
    Plan,
    Review,
}

impl WorkbenchModeId {
    pub(crate) fn label(self) -> &'static str {
        match self {
            WorkbenchModeId::Default => "Default",
            WorkbenchModeId::Research => "Research",
            WorkbenchModeId::Plan => "Plan",
            WorkbenchModeId::Review => "Review",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerVerbosity {
    Terse,
    Default,
    Verbose,
}

impl ComposerVerbosity {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ComposerVerbosity::Terse => "Terse",
            ComposerVerbosity::Default => "Default",
            ComposerVerbosity::Verbose => "Verbose",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ComposerMenuId {
    Harness,
    Model,
    Effort,
    Mode,
    Verbosity,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PopoverPlacement {
    pub(crate) position: gpui::Point<gpui::Pixels>,
    pub(crate) anchor: gpui::Corner,
    pub(crate) max_height: Option<gpui::Pixels>,
    pub(crate) max_width: Option<gpui::Pixels>,
}

#[derive(Clone, Debug)]
pub(crate) struct DraftTrack {
    pub(crate) key: String,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ComposerDraft {
    pub(crate) text: String,
    pub(crate) mode_id: WorkbenchModeId,
}

impl Default for ComposerDraft {
    fn default() -> Self {
        Self {
            text: String::new(),
            mode_id: WorkbenchModeId::Default,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ContextWindowInfo {
    pub(crate) window_tokens: Option<f64>,
    pub(crate) used_tokens: Option<f64>,
    pub(crate) remaining_tokens: Option<f64>,
    pub(crate) remaining_fraction: Option<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderInstallState {
    #[allow(dead_code)]
    pub(crate) install_id: String,
    pub(crate) state: InstallStateKind,
    pub(crate) pct: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct SlashCommandDescriptor {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerAutocompleteKind {
    Slash,
    At,
}

#[derive(Clone, Debug)]
pub(crate) struct ComposerAutocompleteToken {
    pub(crate) kind: ComposerAutocompleteKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) query: String,
}

#[derive(Clone, Debug)]
pub(crate) enum ComposerAutocompleteItemKind {
    Slash,
    File,
}

#[derive(Clone, Debug)]
pub(crate) struct ComposerAutocompleteItem {
    #[allow(dead_code)]
    pub(crate) key: String,
    pub(crate) kind: ComposerAutocompleteItemKind,
    pub(crate) label: String,
    pub(crate) insert_text: String,
    pub(crate) description: Option<String>,
    pub(crate) path: Option<String>,
}

#[derive(Clone, Debug)]
struct ComposerAutocompleteDismissed {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ComposerAutocompleteState {
    pub(crate) open: bool,
    pub(crate) loading: bool,
    pub(crate) items: Vec<ComposerAutocompleteItem>,
    pub(crate) active_index: usize,
    token: Option<ComposerAutocompleteToken>,
    dismissed: Option<ComposerAutocompleteDismissed>,
    request_id: u64,
}

impl ComposerAutocompleteState {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            loading: false,
            items: Vec::new(),
            active_index: 0,
            token: None,
            dismissed: None,
            request_id: 0,
        }
    }

    fn reset(&mut self) {
        self.open = false;
        self.loading = false;
        self.items.clear();
        self.active_index = 0;
        self.token = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerTarget {
    NewTask,
    Session(SessionId),
}

impl ShellView {
    pub(crate) fn init_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_subscriptions_set {
            return;
        }

        let new_input_sub = cx.subscribe_in(
            &self.composer_new_input,
            window,
            |view, _, event, window, cx| {
                view.handle_composer_input_event(event, window, cx)
            },
        );
        self.composer_subscriptions.push(new_input_sub);

        let session_input_sub = cx.subscribe_in(
            &self.composer_session_input,
            window,
            |view, _, event, window, cx| {
                view.handle_composer_input_event(event, window, cx)
            },
        );
        self.composer_subscriptions.push(session_input_sub);

        let harness_sub = cx.subscribe(&self.composer_harness_search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.composer_subscriptions.push(harness_sub);

        let model_sub = cx.subscribe(&self.composer_model_search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.composer_subscriptions.push(model_sub);

        let manual_sub = cx.subscribe(&self.composer_model_manual, |view, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                view.on_manual_model_input_change(cx);
            }
        });
        self.composer_subscriptions.push(manual_sub);

        self.composer_subscriptions_set = true;
    }

    pub(crate) fn focus_composer(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_has_focus = true;
        self.active_composer_input()
            .update(cx, |state, cx| state.focus(window, cx));
    }


    fn handle_composer_input_event(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.on_composer_input_change(window, cx),
            InputEvent::PressEnter { secondary } => {
                self.on_composer_press_enter(*secondary, window, cx);
            }
            InputEvent::Focus => {
                self.composer_has_focus = true;
                self.sync_composer_autocomplete(cx);
                self.update_autocomplete_positions(window, cx);
            }
            InputEvent::Blur => {
                self.composer_has_focus = false;
                self.dismiss_autocomplete(cx);
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_new_task_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.new_task_mode {
            self.new_task_mode_locked = true;
            self.composer_focus_pending = true;
            return;
        }
        self.new_task_mode = true;
        self.new_task_mode_locked = true;
        self.composer_focus_pending = true;
        self.apply_active_composer_state(window, cx);
    }

    #[allow(dead_code)]
    pub(crate) fn set_session_mode_active(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode {
            return;
        }
        self.new_task_mode = false;
        self.new_task_mode_locked = false;
        self.composer_focus_pending = false;
        self.apply_active_composer_state(window, cx);
    }

    pub(crate) fn on_composer_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.to_lowercase();
        if key == "b" && (modifiers.platform || modifiers.control) {
            self.set_sidebar_collapsed(!self.sidebar_collapsed, cx);
            cx.stop_propagation();
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => {
                if self.handle_autocomplete_tab(window, cx) {
                    cx.stop_propagation();
                }
            }
            "up" => {
                if self.handle_autocomplete_up(cx) {
                    cx.stop_propagation();
                }
            }
            "down" => {
                if self.handle_autocomplete_down(cx) {
                    cx.stop_propagation();
                }
            }
            "escape" => {
                if self.handle_autocomplete_escape(cx) {
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn on_composer_key_up(
        &mut self,
        _: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_composer_autocomplete(cx);
        self.update_autocomplete_positions(window, cx);
    }

    pub(crate) fn on_composer_mouse_up(
        &mut self,
        _: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_composer_autocomplete(cx);
        self.update_autocomplete_positions(window, cx);
    }

    pub(crate) fn on_composer_press_enter(
        &mut self,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_autocomplete_enter(secondary, window, cx) {
            cx.stop_propagation();
            return;
        }

        if secondary {
            return;
        }

        if self.can_send_message(cx) {
            self.send_composer_message(window, cx);
            cx.stop_propagation();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn on_composer_tab(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_autocomplete_tab(window, cx) {
            cx.stop_propagation();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn on_composer_arrow_up(
        &mut self,
        _: &MoveUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_autocomplete_up(cx) {
            cx.stop_propagation();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn on_composer_arrow_down(
        &mut self,
        _: &MoveDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_autocomplete_down(cx) {
            cx.stop_propagation();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn on_composer_escape(
        &mut self,
        _: &Escape,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.handle_autocomplete_escape(cx) {
            cx.stop_propagation();
        }
    }

    pub(crate) fn on_attach_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images".into()),
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let paths = match rx.await {
                    Ok(Ok(Some(paths))) => paths,
                    _ => return,
                };
                this.update(&mut cx, |view, cx| {
                    view.add_attachments_from_paths(paths, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn on_composer_drop(
        &mut self,
        paths: &ExternalPaths,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = paths.paths().to_vec();
        self.add_attachments_from_paths(items, cx);
    }

    pub(crate) fn remove_composer_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.composer_attachments.len() {
            return;
        }
        self.composer_attachments.remove(index);
        self.persist_active_attachments();
        cx.notify();
    }

    pub(crate) fn toggle_menu(
        &mut self,
        menu: ComposerMenuId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_open_menu == Some(menu) {
            self.composer_open_menu = None;
        } else {
            self.composer_open_menu = Some(menu);
            self.composer_menu_bounds.remove(&menu);
            self.composer_menu_placements.remove(&menu);
            if menu == ComposerMenuId::Harness {
                self.composer_harness_expanded_provider = None;
                self.composer_harness_search.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                    state.focus(window, cx);
                });
            }
        }
        self.composer_harness_count_menu_provider = None;
        self.composer_harness_count_menu_placement = None;
        self.composer_tooltip_open = None;
        cx.notify();
    }

    pub(crate) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.composer_open_menu.is_none()
            && self.composer_harness_count_menu_provider.is_none()
        {
            return;
        }
        self.composer_open_menu = None;
        self.composer_harness_count_menu_provider = None;
        self.composer_harness_count_menu_placement = None;
        self.composer_tooltip_open = None;
        cx.notify();
    }

    pub(crate) fn update_composer_menu_trigger_bounds(
        &mut self,
        menu: ComposerMenuId,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .composer_menu_trigger_bounds
            .get(&menu)
            .copied()
            == Some(bounds)
        {
            return;
        }
        self.composer_menu_trigger_bounds.insert(menu, bounds);
        self.recompute_menu_placement(menu, window);
        cx.notify();
    }

    pub(crate) fn update_composer_menu_bounds(
        &mut self,
        menu: ComposerMenuId,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_menu_bounds.get(&menu).copied() == Some(bounds) {
            return;
        }
        self.composer_menu_bounds.insert(menu, bounds);
        self.recompute_menu_placement(menu, window);
        cx.notify();
    }

    fn recompute_menu_placement(&mut self, menu: ComposerMenuId, window: &Window) {
        if self.composer_open_menu != Some(menu) {
            return;
        }
        let Some(trigger) = self.composer_menu_trigger_bounds.get(&menu).copied() else {
            return;
        };
        let Some(menu_bounds) = self.composer_menu_bounds.get(&menu).copied() else {
            return;
        };
        let placement = compute_menu_placement(menu, trigger, menu_bounds, window);
        self.composer_menu_placements.insert(menu, placement);
    }

    pub(crate) fn update_composer_tooltip_trigger_bounds(
        &mut self,
        menu: ComposerMenuId,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .composer_tooltip_trigger_bounds
            .get(&menu)
            .copied()
            == Some(bounds)
        {
            return;
        }
        self.composer_tooltip_trigger_bounds.insert(menu, bounds);
        self.recompute_tooltip_placement(menu, window);
        cx.notify();
    }

    pub(crate) fn update_composer_tooltip_bounds(
        &mut self,
        menu: ComposerMenuId,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_tooltip_bounds.get(&menu).copied() == Some(bounds) {
            return;
        }
        self.composer_tooltip_bounds.insert(menu, bounds);
        self.recompute_tooltip_placement(menu, window);
        cx.notify();
    }

    fn recompute_tooltip_placement(&mut self, menu: ComposerMenuId, window: &Window) {
        if self.composer_tooltip_open != Some(menu) {
            return;
        }
        let Some(trigger) = self
            .composer_tooltip_trigger_bounds
            .get(&menu)
            .copied()
        else {
            return;
        };
        let Some(bounds) = self.composer_tooltip_bounds.get(&menu).copied() else {
            return;
        };
        let placement = compute_tooltip_placement(trigger, bounds, window);
        self.composer_tooltip_placements.insert(menu, placement);
    }

    pub(crate) fn open_menu_tooltip(&mut self, menu: ComposerMenuId, cx: &mut Context<Self>) {
        if self.composer_tooltip_open == Some(menu) {
            return;
        }
        self.composer_tooltip_open = Some(menu);
        self.composer_tooltip_close_id = self.composer_tooltip_close_id.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn schedule_close_menu_tooltip(&mut self, menu: ComposerMenuId, cx: &mut Context<Self>) {
        self.composer_tooltip_close_id = self.composer_tooltip_close_id.wrapping_add(1);
        let close_id = self.composer_tooltip_close_id;
        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                sleep(Duration::from_millis(180)).await;
                this.update(&mut cx, |view, cx| {
                    if view.composer_tooltip_close_id == close_id
                        && view.composer_tooltip_open == Some(menu)
                    {
                        view.composer_tooltip_open = None;
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn update_composer_input_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_autocomplete_input_bounds == Some(bounds) {
            return;
        }
        self.composer_autocomplete_input_bounds = Some(bounds);
        self.update_autocomplete_positions(window, cx);
    }

    pub(crate) fn update_autocomplete_positions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer_autocomplete.open {
            self.composer_autocomplete_anchor_bounds = None;
            self.composer_autocomplete_menu_placement = None;
            self.composer_autocomplete_menu_width = None;
            self.composer_autocomplete_preview_placement = None;
            return;
        }
        self.update_autocomplete_anchor(window, cx);
        self.update_autocomplete_menu_placement(window);
    }

    fn update_autocomplete_anchor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input_bounds) = self.composer_autocomplete_input_bounds else {
            self.composer_autocomplete_anchor_bounds = None;
            return;
        };
        let input = self.active_composer_input();
        let selection = input.update(cx, |state, cx| {
            state.selected_text_range(false, window, cx)
        });
        let anchor_bounds = selection
            .and_then(|sel| {
                input.update(cx, |state, cx| {
                    state.bounds_for_range(sel.range, input_bounds, window, cx)
                })
            });
        self.composer_autocomplete_anchor_bounds = anchor_bounds;
    }

    fn update_autocomplete_menu_placement(&mut self, window: &Window) {
        let Some(anchor_bounds) = self.composer_autocomplete_anchor_bounds else {
            self.composer_autocomplete_menu_placement = None;
            self.composer_autocomplete_menu_width = None;
            return;
        };
        let Some(input_bounds) = self.composer_autocomplete_input_bounds else {
            self.composer_autocomplete_menu_placement = None;
            self.composer_autocomplete_menu_width = None;
            return;
        };
        if self.composer_autocomplete.items.is_empty() && self.composer_autocomplete.loading {
            self.composer_autocomplete_menu_placement = None;
            self.composer_autocomplete_menu_width = None;
            return;
        }

        let viewport = window.bounds().size;
        let margin = px(MENU_MARGIN);
        let offset = px(6.0);
        let width = clamp_px(
            input_bounds.size.width,
            px(AUTOCOMPLETE_MIN_WIDTH),
            px(AUTOCOMPLETE_MAX_WIDTH),
        );
        self.composer_autocomplete_menu_width = Some(width);

        let row_count = if !self.composer_autocomplete.items.is_empty() {
            self.composer_autocomplete.items.len() as f32
        } else {
            1.0
        };
        let estimated_height =
            row_count * AUTOCOMPLETE_ROW_HEIGHT + AUTOCOMPLETE_CONTAINER_EXTRA_Y;
        let min_height = AUTOCOMPLETE_ROW_HEIGHT + AUTOCOMPLETE_CONTAINER_EXTRA_Y;

        let space_below = viewport.height - margin - (anchor_bounds.bottom() + offset);
        let space_above = anchor_bounds.top() - margin - offset;
        let open_above = space_below < px(estimated_height) && space_above > space_below;

        let max_available = if open_above { space_above } else { space_below };
        let max_height = clamp_px(
            max_available.min(px(AUTOCOMPLETE_MAX_HEIGHT)),
            px(min_height),
            px(AUTOCOMPLETE_MAX_HEIGHT),
        );

        let left = clamp_px(
            input_bounds.left(),
            margin,
            viewport.width - margin - width,
        );
        let (anchor, position) = if open_above {
            (
                Corner::BottomLeft,
                point(left, anchor_bounds.top() - offset),
            )
        } else {
            (
                Corner::TopLeft,
                point(left, anchor_bounds.bottom() + offset),
            )
        };

        self.composer_autocomplete_menu_placement = Some(PopoverPlacement {
            position,
            anchor,
            max_height: Some(max_height),
            max_width: Some(width),
        });
    }

    pub(crate) fn update_autocomplete_preview_position(
        &mut self,
        row_bounds: Bounds<Pixels>,
        window: &Window,
    ) {
        let Some(menu_placement) = self.composer_autocomplete_menu_placement else {
            self.composer_autocomplete_preview_placement = None;
            return;
        };
        let Some(menu_width) = self.composer_autocomplete_menu_width else {
            self.composer_autocomplete_preview_placement = None;
            return;
        };
        let viewport = window.bounds().size;
        let margin = px(MENU_MARGIN);
        let gap = px(10.0);
        let preview_width = px(AUTOCOMPLETE_PREVIEW_WIDTH);
        let max_height = px(AUTOCOMPLETE_PREVIEW_MAX_HEIGHT);

        let mut left = menu_placement.position.x + menu_width + gap;
        if left + preview_width + margin > viewport.width {
            left = menu_placement.position.x - preview_width - gap;
        }
        left = clamp_px(left, margin, viewport.width - margin - preview_width);

        let mut top = row_bounds.top() - px(6.0);
        top = clamp_px(top, margin, viewport.height - margin - max_height);

        self.composer_autocomplete_preview_placement = Some(PopoverPlacement {
            position: point(left, top),
            anchor: Corner::TopLeft,
            max_height: Some(max_height),
            max_width: Some(preview_width),
        });
    }

    pub(crate) fn toggle_harness_count_menu(
        &mut self,
        provider_id: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_harness_count_menu_provider.as_deref() == Some(&provider_id) {
            self.close_harness_count_menu(cx);
            return;
        }
        let Some(anchor) = self
            .composer_harness_count_trigger_bounds
            .get(&provider_id)
            .copied()
        else {
            return;
        };
        let approx_bounds = Bounds::new(
            point(anchor.right() - px(140.0), anchor.bottom()),
            gpui::size(px(140.0), px(160.0)),
        );
        let placement = compute_count_menu_placement(anchor, approx_bounds, window);
        self.composer_harness_count_menu_provider = Some(provider_id);
        self.composer_harness_count_menu_bounds = None;
        self.composer_harness_count_menu_placement = Some(placement);
        cx.notify();
    }

    pub(crate) fn close_harness_count_menu(&mut self, cx: &mut Context<Self>) {
        self.composer_harness_count_menu_provider = None;
        self.composer_harness_count_menu_bounds = None;
        self.composer_harness_count_menu_placement = None;
        cx.notify();
    }

    pub(crate) fn update_harness_count_trigger_bounds(
        &mut self,
        provider_id: String,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .composer_harness_count_trigger_bounds
            .get(&provider_id)
            .copied()
            == Some(bounds)
        {
            return;
        }
        self.composer_harness_count_trigger_bounds
            .insert(provider_id.clone(), bounds);
        if self.composer_harness_count_menu_provider.as_deref() == Some(&provider_id) {
            if let Some(menu_bounds) = self.composer_harness_count_menu_bounds {
                let placement = compute_count_menu_placement(bounds, menu_bounds, window);
                self.composer_harness_count_menu_placement = Some(placement);
                cx.notify();
            }
        }
    }

    pub(crate) fn update_harness_count_menu_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_harness_count_menu_bounds == Some(bounds) {
            return;
        }
        self.composer_harness_count_menu_bounds = Some(bounds);
        let Some(provider_id) = self.composer_harness_count_menu_provider.clone() else {
            return;
        };
        let Some(anchor) = self
            .composer_harness_count_trigger_bounds
            .get(&provider_id)
            .copied()
        else {
            return;
        };
        let placement = compute_count_menu_placement(anchor, bounds, window);
        self.composer_harness_count_menu_placement = Some(placement);
        cx.notify();
    }

    pub(crate) fn set_mode_id(&mut self, mode: WorkbenchModeId, cx: &mut Context<Self>) {
        self.composer_mode_id = mode;
        match self.composer_target() {
            ComposerTarget::NewTask => self.composer_new_draft.mode_id = mode,
            ComposerTarget::Session(session_id) => {
                self.composer_session_drafts
                    .entry(session_id)
                    .or_default()
                    .mode_id = mode;
            }
        }
        cx.notify();
    }

    pub(crate) fn set_verbosity(&mut self, verbosity: ComposerVerbosity, cx: &mut Context<Self>) {
        self.composer_verbosity = verbosity;
        cx.notify();
    }

    pub(crate) fn toggle_recording(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer_recording = !self.composer_recording;
        cx.notify();
    }

    pub(crate) fn on_send_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_composer_message(window, cx);
    }

    pub(crate) fn can_send_message(&self, cx: &Context<Self>) -> bool {
        let has_text = self.active_composer_input().read(cx).value().trim().is_empty();
        let has_attachments = !self.composer_attachments.is_empty();
        !(has_text && !has_attachments)
    }

    fn send_composer_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_task_mode || self.selected_session_id().is_none() {
            self.start_new_task(window, cx);
        } else {
            self.send_session_message(window, cx);
        }
    }

    fn send_session_message(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let content = self.active_composer_input().read(cx).value().trim().to_string();
        if content.is_empty() && self.composer_attachments.is_empty() {
            return;
        }
        let attachments = self.composer_attachments.clone();
        let optimistic_id = MessageId::new();
        let mut optimistic_message = MessageItem::new(MessageRole::User, content.clone());
        optimistic_message.id = Some(optimistic_id);
        optimistic_message.attachments = attachments.clone();
        optimistic_message.delivery = MessageDelivery::Queued;
        self.push_message(optimistic_message, cx);
        self.mark_composer_cleared();
        cx.notify();

        let request_content = content.clone();
        let request_attachments = attachments.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let request = ctx_client::PostMessageRequest {
                content: request_content,
                delivery: None,
                attachments: request_attachments,
            };
            client.post_message(session_id, &request).await?;
            Ok(session_id)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(session_id) => {
                            view.load_session_details(session_id, cx);
                        }
                        Err(_) => {
                            view.remove_message_by_id(optimistic_id);
                            let should_restore = view.is_session_selected(session_id)
                                && view
                                    .active_composer_input()
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .is_empty()
                                && view.composer_attachments.is_empty()
                            ;
                            if should_restore {
                                if let Some(draft) =
                                    view.composer_session_drafts.get_mut(&session_id)
                                {
                                    draft.text = content.clone();
                                } else {
                                    view.composer_session_drafts.insert(
                                        session_id,
                                        ComposerDraft {
                                            text: content.clone(),
                                            mode_id: view.composer_mode_id,
                                        },
                                    );
                                }
                                view.composer_session_attachments
                                    .insert(session_id, attachments.clone());
                                view.composer_needs_apply = true;
                            }
                            if view.is_session_selected(session_id) {
                                view.push_message(
                                    MessageItem::new(
                                        MessageRole::Assistant,
                                        "Unable to send message.",
                                    ),
                                    cx,
                                );
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

    fn start_new_task(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let workspace_id = self.selected_workspace;
        if workspace_id.is_none() {
            return;
        }
        let workspace_id = workspace_id.unwrap();
        let content = self.active_composer_input().read(cx).value().trim().to_string();
        if content.is_empty() && self.composer_attachments.is_empty() {
            return;
        }
        if self.composer_start_busy {
            return;
        }
        self.new_task_mode_locked = false;

        self.ensure_primary_draft_track();
        let provider_ids: Vec<String> = self
            .composer_draft_tracks
            .iter()
            .map(|track| track.provider_id.clone())
            .collect();
        if provider_ids.is_empty() {
            return;
        }

        self.composer_start_busy = true;
        self.composer_start_error = None;
        cx.notify();

        let tracks = self.composer_draft_tracks.clone();
        let attachments = self.composer_attachments.clone();
        let content_clone = content.clone();
        let providers = self.providers.clone();
        let provider_options = self.composer_provider_options.clone();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;

            let title = derive_task_title(&content_clone);
            let task = client
                .create_task(
                    workspace_id,
                    &ctx_client::CreateTaskRequest {
                        title,
                        description: None,
                        create_default_session: Some(false),
                    },
                )
                .await?;

            let mut first_session_id = None;

            let to_start = if tracks.is_empty() {
                vec![DraftTrack {
                    key: "t1".to_string(),
                    provider_id: "codex".to_string(),
                    model_id: String::new(),
                }]
            } else {
                tracks
            };

            for dt in to_start {
                let installed = providers.iter().find(|p| p.provider_id == dt.provider_id);
                if let Some(provider) = installed {
                    if !provider.installed || !matches!(provider.health, ProviderHealth::Ok) {
                        let diag = provider.diagnostics.first().cloned();
                        return Err(anyhow::anyhow!(
                            diag
                                .map(|d| format!(
                                    "Harness “{}” unavailable: {}",
                                    dt.provider_id, d
                                ))
                                .unwrap_or_else(|| {
                                    format!(
                                        "Harness “{}” unavailable.",
                                        dt.provider_id
                                    )
                                })
                        ));
                    }
                } else {
                    return Err(anyhow::anyhow!(format!(
                        "Harness “{}” unavailable.",
                        dt.provider_id
                    )));
                }

                let opts = provider_options.get(&dt.provider_id).cloned();
                let model_ids = model_ids_from_provider_options(opts.as_ref());
                let model_id = if !dt.model_id.trim().is_empty() {
                    dt.model_id.clone()
                } else if !model_ids.is_empty() {
                    model_ids[0].clone()
                } else if dt.provider_id == "fake" {
                    "fake-model".to_string()
                } else {
                    "default".to_string()
                };

                let session = client
                    .create_session(
                        task.id,
                        &ctx_client::CreateSessionRequest {
                            provider_id: dt.provider_id.clone(),
                            model_id,
                            parent_session_id: first_session_id,
                            relationship: None,
                            env_target: Some(EnvTarget::Worktree),
                            worktree_id: None,
                            initial_prompt: None,
                        },
                    )
                    .await?;
                if first_session_id.is_none() {
                    first_session_id = Some(session.session.id);
                }

                let request = ctx_client::PostMessageRequest {
                    content: content_clone.clone(),
                    delivery: Some(MessageDelivery::Immediate),
                    attachments: attachments.clone(),
                };
                client
                    .post_message(session.session.id, &request)
                    .await?;
            }

            Ok((task.id, first_session_id))
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.composer_start_busy = false;
                    match result {
                        Ok((_task_id, first_session_id)) => {
                            view.mark_composer_cleared();
                            view.composer_start_error = None;
                            view.start_data_load(cx);
                            if let Some(session_id) = first_session_id {
                                view.load_session_details(session_id, cx);
                            }
                        }
                        Err(err) => {
                            view.composer_start_error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    #[allow(dead_code)]
    fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_composer_input()
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.clear_composer_state();
    }

    fn mark_composer_cleared(&mut self) {
        self.clear_composer_state();
        self.composer_needs_apply = true;
    }

    fn clear_composer_state(&mut self) {
        self.composer_attachments.clear();
        self.persist_active_attachments();
        self.composer_autocomplete.reset();
        match self.composer_target() {
            ComposerTarget::NewTask => self.composer_new_draft.text.clear(),
            ComposerTarget::Session(session_id) => {
                self.composer_session_drafts
                    .entry(session_id)
                    .or_default()
                    .text
                    .clear();
            }
        }
    }

    fn on_composer_input_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.active_composer_input().read(cx).value().to_string();
        match self.composer_target() {
            ComposerTarget::NewTask => self.composer_new_draft.text = text,
            ComposerTarget::Session(session_id) => {
                self.composer_session_drafts
                    .entry(session_id)
                    .or_default()
                    .text = text;
            }
        }
        self.sync_composer_autocomplete(cx);
        self.update_autocomplete_positions(window, cx);
    }

    fn on_manual_model_input_change(&mut self, cx: &mut Context<Self>) {
        let value = self.composer_model_manual.read(cx).value().to_string();
        self.composer_model_id = Some(value.clone());
        if self.new_task_mode {
            if let Some(track) = self.composer_draft_tracks.first_mut() {
                track.model_id = value;
            }
        }
        cx.notify();
    }

    fn composer_target(&self) -> ComposerTarget {
        if self.new_task_mode {
            ComposerTarget::NewTask
        } else if let Some(session_id) = self.selected_session_id() {
            ComposerTarget::Session(session_id)
        } else {
            ComposerTarget::NewTask
        }
    }

    fn composer_input_for_target(&self, target: ComposerTarget) -> &gpui::Entity<InputState> {
        match target {
            ComposerTarget::NewTask => &self.composer_new_input,
            ComposerTarget::Session(_) => &self.composer_session_input,
        }
    }

    pub(crate) fn active_composer_input(&self) -> &gpui::Entity<InputState> {
        self.composer_input_for_target(self.composer_target())
    }

    pub(crate) fn apply_active_composer_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.composer_target();
        let draft = match target {
            ComposerTarget::NewTask => self.composer_new_draft.clone(),
            ComposerTarget::Session(session_id) => self
                .composer_session_drafts
                .get(&session_id)
                .cloned()
                .unwrap_or_default(),
        };

        self.composer_mode_id = draft.mode_id;
        let draft_text = draft.text.clone();
        let focus_after = self.composer_has_focus;
        self.composer_input_for_target(target)
            .update(cx, move |state, cx| {
                state.set_value(draft_text, window, cx);
                if focus_after {
                    state.focus(window, cx);
                }
            });

        self.composer_attachments = match self.composer_target() {
            ComposerTarget::NewTask => self.composer_new_attachments.clone(),
            ComposerTarget::Session(session_id) => self
                .composer_session_attachments
                .get(&session_id)
                .cloned()
                .unwrap_or_default(),
        };

        self.ensure_composer_attachment_previews(cx);
        self.composer_autocomplete.reset();
        cx.notify();
    }

    pub(crate) fn update_composer_placeholders(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let loading = if self.new_task_mode {
            let provider_id = self
                .composer_draft_tracks
                .first()
                .map(|track| track.provider_id.clone())
                .or_else(|| self.composer_provider_id.clone());
            provider_id
                .as_deref()
                .map(|id| !self.composer_provider_options.contains_key(id))
                .unwrap_or(true)
        } else {
            false
        };
        let placeholder = if loading {
            "Loading models…"
        } else {
            "Search models"
        };
        if self.composer_model_search_placeholder != placeholder {
            self.composer_model_search_placeholder = placeholder.to_string();
            self.composer_model_search.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx);
            });
        }
    }

    pub(crate) fn ensure_composer_provider_options(&mut self, cx: &mut Context<Self>) {
        let mut provider_ids = BTreeSet::new();
        if self.new_task_mode {
            if self.composer_use_multiple_agents {
                for track in &self.composer_draft_tracks {
                    if !track.provider_id.trim().is_empty() {
                        provider_ids.insert(track.provider_id.clone());
                    }
                }
            } else if let Some(track) = self.composer_draft_tracks.first() {
                if !track.provider_id.trim().is_empty() {
                    provider_ids.insert(track.provider_id.clone());
                }
            } else if let Some(provider_id) = self.composer_provider_id.clone() {
                if !provider_id.trim().is_empty() {
                    provider_ids.insert(provider_id);
                }
            }
        } else if let Some(provider_id) = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
            .and_then(|summary| self.session_summary_map.get(&summary.session_id))
            .map(|summary| summary.session.provider_id.clone())
            .or_else(|| self.composer_provider_id.clone())
        {
            if !provider_id.trim().is_empty() {
                provider_ids.insert(provider_id);
            }
        }

        for provider_id in provider_ids {
            if self.composer_provider_options.contains_key(&provider_id) {
                continue;
            }
            if self
                .composer_provider_opts_busy
                .get(&provider_id)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let installed = self
                .providers
                .iter()
                .find(|provider| provider.provider_id == provider_id)
                .map(|provider| provider.installed && matches!(provider.health, ProviderHealth::Ok))
                .unwrap_or(false);
            if !installed {
                continue;
            }
            self.ensure_provider_options(provider_id, false, cx);
        }
    }

    pub(crate) fn ensure_track_model_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode || !self.composer_use_multiple_agents {
            self.composer_track_model_inputs.clear();
            self.composer_track_input_subscriptions.clear();
            self.composer_track_model_placeholders.clear();
            return;
        }

        let keys: HashSet<String> = self
            .composer_draft_tracks
            .iter()
            .map(|track| track.key.clone())
            .collect();
        self.composer_track_model_inputs
            .retain(|key, _| keys.contains(key));
        self.composer_track_input_subscriptions
            .retain(|key, _| keys.contains(key));
        self.composer_track_model_placeholders
            .retain(|key, _| keys.contains(key));

        for track in &self.composer_draft_tracks {
            let key = track.key.clone();
            let placeholder = if self.composer_provider_options.contains_key(&track.provider_id) {
                "model_id"
            } else {
                "Loading models…"
            };
            let input = self
                .composer_track_model_inputs
                .entry(key.clone())
                .or_insert_with(|| {
                    self.composer_track_model_placeholders
                        .insert(key.clone(), placeholder.to_string());
                    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
                });
            let value = track.model_id.clone();
            input.update(cx, |state, cx| {
                if state.value() != value {
                    state.set_value(value.clone(), window, cx);
                }
            });
            if self
                .composer_track_model_placeholders
                .get(&key)
                .map(|current| current.as_str() != placeholder)
                .unwrap_or(true)
            {
                input.update(cx, |state, cx| {
                    state.set_placeholder(placeholder, window, cx);
                });
                self.composer_track_model_placeholders
                    .insert(key.clone(), placeholder.to_string());
            }

            if !self.composer_track_input_subscriptions.contains_key(&key) {
                let key_for_sub = key.clone();
                let input_entity = input.clone();
                let input_for_read = input_entity.clone();
                let sub = cx.subscribe(&input_entity, move |view, _, event, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let value = input_for_read.read(cx).value().to_string();
                    view.update_track_model(&key_for_sub, value, cx);
                });
                self.composer_track_input_subscriptions.insert(key, sub);
            }
        }
    }

    fn persist_active_attachments(&mut self) {
        match self.composer_target() {
            ComposerTarget::NewTask => self.composer_new_attachments = self.composer_attachments.clone(),
            ComposerTarget::Session(session_id) => {
                self.composer_session_attachments
                    .insert(session_id, self.composer_attachments.clone());
            }
        }
    }

    fn ensure_composer_attachment_previews(&mut self, cx: &mut Context<Self>) {
        let attachments = self.composer_attachments.clone();
        for att in attachments {
            let MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                ..
            } = att
            else {
                continue;
            };
            if self.composer_attachment_images.contains_key(&blob_id)
                || self.composer_attachment_loading.contains(&blob_id)
            {
                continue;
            }
            self.composer_attachment_loading.insert(blob_id.clone());
            let mime = mime_type.clone();
            let blob_id_for_task = blob_id.clone();
            let blob_id_for_update = blob_id.clone();
            let task = Tokio::spawn_result(cx, async move {
                let config = ctx_client::resolve_daemon_config()?;
                let client = ctx_client::Client::new(config)?;
                let bytes = client.get_blob(&blob_id_for_task).await?;
                Ok((blob_id_for_task, mime, bytes))
            });

            cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        view.composer_attachment_loading.remove(&blob_id_for_update);
                        match result {
                            Ok((blob_id, mime, bytes)) => {
                                let format =
                                    ImageFormat::from_mime_type(&mime).unwrap_or(ImageFormat::Png);
                                view.composer_attachment_images
                                    .insert(blob_id, Arc::new(Image::from_bytes(format, bytes)));
                            }
                            Err(err) => {
                                view.composer_notice =
                                    Some(format!("Attachment fetch failed: {err}"));
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    fn sync_composer_autocomplete(&mut self, cx: &mut Context<Self>) {
        let input = self.active_composer_input();
        let text = input.read(cx).value().to_string();
        let cursor = input.read(cx).cursor();
        let next = detect_composer_autocomplete_token(&text, cursor);

        if let Some(ref token) = next {
            if let Some(ref dismissed) = self.composer_autocomplete.dismissed {
                if dismissed.start == token.start
                    && dismissed.end == token.end
                    && dismissed.text == text[token.start..token.end]
                {
                    self.composer_autocomplete.reset();
                    return;
                }
            }
        }

        if !same_autocomplete_token(self.composer_autocomplete.token.as_ref(), next.as_ref()) {
            self.composer_autocomplete.active_index = 0;
        }

        self.composer_autocomplete.token = next.clone();
        self.composer_autocomplete.open = next.is_some();

        if let Some(token) = next {
            match token.kind {
                ComposerAutocompleteKind::Slash => {
                    self.composer_autocomplete.loading = false;
                    self.composer_autocomplete.items = self
                        .composer_slash_items(&token)
                        .into_iter()
                        .take(10)
                        .collect();
                }
                ComposerAutocompleteKind::At => {
                    self.schedule_file_completions(token, cx);
                }
            }
        } else {
            self.composer_autocomplete.items.clear();
            self.composer_autocomplete.loading = false;
        }
        cx.notify();
    }

    fn schedule_file_completions(
        &mut self,
        token: ComposerAutocompleteToken,
        cx: &mut Context<Self>,
    ) {
        let session_id = if self.new_task_mode { None } else { self.selected_session_id() };
        let workspace_id = self.selected_workspace;
        if session_id.is_none() && workspace_id.is_none() {
            self.composer_autocomplete.items.clear();
            self.composer_autocomplete.loading = false;
            return;
        }

        self.composer_autocomplete.loading = true;
        self.composer_autocomplete.request_id += 1;
        let request_id = self.composer_autocomplete.request_id;
        let query = token.query.clone();

        let task = Tokio::spawn_result(cx, async move {
            tokio::time::sleep(Duration::from_millis(AUTOCOMPLETE_DEBOUNCE_MS)).await;
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let paths = if let Some(session_id) = session_id {
                client
                    .list_session_file_completions(session_id, &query, Some(FILE_COMPLETION_LIMIT))
                    .await?
            } else if let Some(workspace_id) = workspace_id {
                client
                    .list_workspace_file_completions(
                        workspace_id,
                        &query,
                        Some(FILE_COMPLETION_LIMIT),
                    )
                    .await?
            } else {
                Vec::new()
            };
            Ok((request_id, paths))
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    if view.composer_autocomplete.request_id != request_id {
                        return;
                    }
                    match result {
                        Ok((_request_id, paths)) => {
                            view.composer_autocomplete.items = paths
                                .into_iter()
                                .map(|path| {
                                    let label = file_label(&path);
                                    ComposerAutocompleteItem {
                                        key: format!("file:{path}"),
                                        kind: ComposerAutocompleteItemKind::File,
                                        label,
                                        insert_text: format!("@{path}"),
                                        description: None,
                                        path: Some(path),
                                    }
                                })
                                .collect();
                        }
                        Err(_) => {
                            view.composer_autocomplete.items.clear();
                        }
                    }
                    view.composer_autocomplete.loading = false;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn composer_slash_items(&self, token: &ComposerAutocompleteToken) -> Vec<ComposerAutocompleteItem> {
        let q = token.query.trim().to_ascii_lowercase();
        let commands = self.slash_commands();
        let mut items: Vec<ComposerAutocompleteItem> = commands
            .into_iter()
            .filter(|cmd| {
                if q.is_empty() {
                    return true;
                }
                let name = cmd.name.to_ascii_lowercase();
                let full = format!("/{name}");
                name.starts_with(&q) || name.contains(&q) || full.contains(&q)
            })
            .map(|cmd| ComposerAutocompleteItem {
                key: format!("slash:{}", cmd.name),
                kind: ComposerAutocompleteItemKind::Slash,
                label: format!("/{}", cmd.name),
                insert_text: format!("/{}", cmd.name),
                description: cmd.description,
                path: None,
            })
            .collect();
        items.truncate(10);
        items
    }

    fn slash_commands(&self) -> Vec<SlashCommandDescriptor> {
        if self.new_task_mode {
            return fallback_slash_commands_for_new_task();
        }

        if let Some(commands) = self.slash_commands_from_events() {
            if !commands.is_empty() {
                return commands;
            }
        }

        let provider_id = self
            .selected_session_id()
            .and_then(|session_id| self.session_summary_map.get(&session_id))
            .map(|summary| summary.session.provider_id.clone());
        fallback_slash_commands_for_session(provider_id.as_deref())
    }

    fn slash_commands_from_events(&self) -> Option<Vec<SlashCommandDescriptor>> {
        let last = self.session_events.iter().rev().find_map(|event| {
            let update = event
                .payload_json
                .get("acp_update")
                .and_then(|value| value.as_object())?;
            let session_update = update
                .get("sessionUpdate")
                .or_else(|| update.get("session_update"))
                .and_then(|value| value.as_str())?;
            if session_update != "available_commands_update" {
                return None;
            }
            Some(update.clone())
        })?;

        let list = last
            .get("availableCommands")
            .or_else(|| last.get("available_commands"))?;
        let arr = list.as_array()?;
        let mut out = Vec::new();
        for item in arr {
            let name = item
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .trim_start_matches('/')
                .to_string();
            if name.is_empty() {
                continue;
            }
            let description = item
                .get("description")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            out.push(SlashCommandDescriptor { name, description });
        }
        Some(out)
    }

    fn handle_autocomplete_enter(
        &mut self,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if secondary {
            return false;
        }
        if !self.composer_autocomplete.open {
            return false;
        }
        if self.composer_autocomplete.items.is_empty() {
            return false;
        }
        self.pick_autocomplete(self.composer_autocomplete.active_index, window, cx);
        true
    }

    fn handle_autocomplete_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.composer_autocomplete.open {
            return false;
        }
        if self.composer_autocomplete.items.is_empty() {
            return false;
        }
        self.pick_autocomplete(self.composer_autocomplete.active_index, window, cx);
        true
    }

    fn handle_autocomplete_up(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.composer_autocomplete.open {
            return false;
        }
        let len = self.composer_autocomplete.items.len();
        if len == 0 {
            self.composer_autocomplete.active_index = 0;
            return true;
        }
        let next = (self.composer_autocomplete.active_index + len - 1) % len;
        self.composer_autocomplete.active_index = next;
        self.composer_autocomplete_preview_placement = None;
        cx.notify();
        true
    }

    fn handle_autocomplete_down(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.composer_autocomplete.open {
            return false;
        }
        let len = self.composer_autocomplete.items.len();
        if len == 0 {
            self.composer_autocomplete.active_index = 0;
            return true;
        }
        let next = (self.composer_autocomplete.active_index + 1) % len;
        self.composer_autocomplete.active_index = next;
        self.composer_autocomplete_preview_placement = None;
        cx.notify();
        true
    }

    fn handle_autocomplete_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.composer_autocomplete.open {
            return false;
        }
        self.dismiss_autocomplete(cx);
        true
    }

    pub(crate) fn set_autocomplete_active_index(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.composer_autocomplete.items.len() {
            return;
        }
        self.composer_autocomplete.active_index = index;
        self.composer_autocomplete_preview_placement = None;
        cx.notify();
    }

    pub(crate) fn pick_autocomplete(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let token = match self.composer_autocomplete.token.clone() {
            Some(token) => token,
            None => return,
        };
        let item = match self.composer_autocomplete.items.get(index) {
            Some(item) => item.clone(),
            None => return,
        };
        let value = self.active_composer_input().read(cx).value().to_string();
        let output = apply_composer_autocomplete_completion(&value, &token, &item.insert_text);
        self.active_composer_input().update(cx, |state, cx| {
            state.set_value(&output.next_text, window, cx);
            let position = state.text().offset_to_position(output.next_cursor);
            state.set_cursor_position(position, window, cx);
        });
        self.composer_autocomplete.dismissed = None;
        self.composer_autocomplete.reset();
        self.sync_composer_autocomplete(cx);
    }

    fn dismiss_autocomplete(&mut self, cx: &mut Context<Self>) {
        if let Some(token) = self.composer_autocomplete.token.clone() {
            let value = self.active_composer_input().read(cx).value().to_string();
            if token.end <= value.len() {
                self.composer_autocomplete.dismissed = Some(ComposerAutocompleteDismissed {
                    start: token.start,
                    end: token.end,
                    text: value[token.start..token.end].to_string(),
                });
            }
        }
        self.composer_autocomplete.reset();
        self.composer_autocomplete_anchor_bounds = None;
        self.composer_autocomplete_menu_placement = None;
        self.composer_autocomplete_menu_width = None;
        self.composer_autocomplete_preview_placement = None;
        cx.notify();
    }

    pub(crate) fn add_attachment_from_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let mime_type = guess_mime_type(&path).to_string();
        if !mime_type.starts_with("image/") {
            self.composer_notice = Some("Only image attachments are supported for now.".to_string());
            cx.notify();
            return;
        }

        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.to_string());
        let task = Tokio::spawn_result(cx, async move {
            let bytes = tokio::fs::read(&path).await?;
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let resp = client
                .upload_blob(bytes.clone(), &mime_type, name.as_deref())
                .await?;
            Ok((resp, bytes, name))
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok((resp, bytes, name)) => {
                            view.composer_attachments.push(MessageAttachment::ImageRef {
                                blob_id: resp.blob_id.clone(),
                                mime_type: resp.mime_type.clone(),
                                name: resp.name.or(name),
                            });
                            if let Some(format) =
                                ImageFormat::from_mime_type(&resp.mime_type)
                            {
                                view.composer_attachment_images.insert(
                                    resp.blob_id,
                                    Arc::new(Image::from_bytes(format, bytes)),
                                );
                            }
                            view.composer_notice = None;
                            view.persist_active_attachments();
                            view.ensure_composer_attachment_previews(cx);
                        }
                        Err(err) => {
                            view.composer_notice =
                                Some(format!("Attachment failed: {err}"));
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn add_attachments_from_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            self.add_attachment_from_path(path, cx);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn composer_provider_options(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|provider| provider.provider_id.clone())
            .collect()
    }

    #[allow(dead_code)]
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
            .filter(|id| self.providers.iter().any(|provider| provider.provider_id == *id))
            .or_else(|| {
                self.providers
                    .iter()
                    .find(|provider| provider.provider_id == "codex")
                    .map(|provider| provider.provider_id.clone())
            })
            .or_else(|| {
                self.session_summary_map
                    .values()
                    .next()
                    .map(|summary| summary.session.provider_id.clone())
            })
            .or_else(|| self.providers.first().map(|provider| provider.provider_id.clone()));
        self.composer_provider_id = default_provider;

        let Some(provider_id) = self.composer_provider_id.clone() else {
            self.composer_model_id = None;
            return;
        };
        let mut models =
            model_ids_from_provider_options(self.composer_provider_options.get(&provider_id));
        if models.is_empty() {
            models = self.model_ids_for_provider(&provider_id);
        }
        let default_model =
            model_id_from_provider_options(self.composer_provider_options.get(&provider_id))
                .or_else(|| models.first().cloned());
        if self
            .composer_model_id
            .as_ref()
            .map(|id| !models.contains(id))
            .unwrap_or(true)
        {
            if let Some(default_model) = default_model {
                self.composer_model_id = Some(default_model);
            } else if self.composer_model_id.is_none() {
                if let Some(summary) = self
                    .session_summary_map
                    .values()
                    .find(|summary| summary.session.provider_id == provider_id)
                {
                    self.composer_model_id = Some(summary.session.model_id.clone());
                }
            }
        }

        self.ensure_primary_draft_track();
        if self.new_task_mode && self.composer_draft_tracks.len() == 1 {
            if let Some(track) = self.composer_draft_tracks.first_mut() {
                if track.model_id.trim().is_empty() {
                    if let Some(model_id) = self.composer_model_id.clone() {
                        track.model_id = model_id;
                    }
                }
            }
        }
    }

    fn ensure_primary_draft_track(&mut self) {
        if !self.new_task_mode {
            return;
        }
        if !self.composer_draft_tracks.is_empty() {
            return;
        }
        let Some(provider_id) = self.composer_provider_id.clone() else {
            return;
        };
        let model_id = self.composer_model_id.clone().unwrap_or_default();
        self.composer_draft_tracks.push(DraftTrack {
            key: "t1".to_string(),
            provider_id,
            model_id,
        });
    }

    fn set_single_draft_track(&mut self, provider_id: String) {
        let model_id = self.composer_model_id.clone().unwrap_or_default();
        self.composer_draft_tracks = vec![DraftTrack {
            key: "t1".to_string(),
            provider_id,
            model_id,
        }];
    }

    pub(crate) fn set_use_multiple_agents(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.composer_use_multiple_agents == enabled {
            return;
        }
        self.composer_use_multiple_agents = enabled;
        if !enabled {
            if let Some(primary) = self.composer_draft_tracks.first().cloned() {
                self.composer_provider_id = Some(primary.provider_id.clone());
                let model_id = primary.model_id.clone();
                if !model_id.trim().is_empty() {
                    self.composer_model_id = Some(model_id);
                }
                self.composer_draft_tracks = vec![primary];
            } else {
                self.ensure_primary_draft_track();
            }
        } else {
            self.ensure_primary_draft_track();
        }
        self.composer_harness_expanded_provider = None;
        self.composer_harness_count_menu_provider = None;
        self.composer_harness_count_menu_placement = None;
        cx.notify();
    }

    pub(crate) fn toggle_harness_provider(
        &mut self,
        provider_id: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode {
            return;
        }
        if self.composer_use_multiple_agents {
            let mut removed = false;
            self.composer_draft_tracks.retain(|track| {
                if track.provider_id == provider_id {
                    removed = true;
                    false
                } else {
                    true
                }
            });
            if removed {
                if self.composer_draft_tracks.is_empty() {
                    self.set_single_draft_track(provider_id.to_string());
                }
                if self.composer_provider_id.as_deref() == Some(provider_id) {
                    self.composer_provider_id =
                        self.composer_draft_tracks.first().map(|track| track.provider_id.clone());
                }
            } else {
                self.composer_draft_tracks.push(DraftTrack {
                    key: format!("t{}", self.composer_draft_tracks.len() + 1),
                    provider_id: provider_id.to_string(),
                    model_id: String::new(),
                });
                self.composer_provider_id = Some(provider_id.to_string());
            }
        } else {
            self.select_composer_provider(provider_id.to_string(), cx);
            return;
        }
        cx.notify();
    }

    pub(crate) fn toggle_harness_expanded_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.composer_harness_expanded_provider.as_deref() == Some(&provider_id) {
            self.composer_harness_expanded_provider = None;
        } else {
            self.composer_harness_expanded_provider = Some(provider_id);
        }
        cx.notify();
    }

    pub(crate) fn set_track_count_for_provider(
        &mut self,
        provider_id: String,
        count: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode || !self.composer_use_multiple_agents {
            return;
        }
        let clamped = count.clamp(1, MAX_TRACKS_PER_PROVIDER);
        let mut existing: Vec<DraftTrack> = self
            .composer_draft_tracks
            .iter()
            .filter(|track| track.provider_id == provider_id)
            .cloned()
            .collect();
        if existing.is_empty() {
            existing.push(DraftTrack {
                key: format!("t{}", self.composer_draft_tracks.len() + 1),
                provider_id: provider_id.clone(),
                model_id: String::new(),
            });
        }

        existing.truncate(clamped);
        while existing.len() < clamped {
            existing.push(DraftTrack {
                key: format!("t{}", self.composer_draft_tracks.len() + existing.len() + 1),
                provider_id: provider_id.clone(),
                model_id: String::new(),
            });
        }

        let mut next = Vec::new();
        let mut inserted = false;
        for track in &self.composer_draft_tracks {
            if track.provider_id != provider_id {
                next.push(track.clone());
                continue;
            }
            if !inserted {
                next.extend(existing.clone());
                inserted = true;
            }
        }
        if !inserted {
            next.extend(existing);
        }

        if next.is_empty() {
            return;
        }
        self.composer_draft_tracks = next;
        cx.notify();
    }

    pub(crate) fn add_track_for_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        if !self.new_task_mode || !self.composer_use_multiple_agents {
            return;
        }
        let existing = self
            .composer_draft_tracks
            .iter()
            .filter(|track| track.provider_id == provider_id)
            .count();
        if existing >= MAX_TRACKS_PER_PROVIDER {
            return;
        }
        self.composer_draft_tracks.push(DraftTrack {
            key: format!("t{}", self.composer_draft_tracks.len() + 1),
            provider_id,
            model_id: String::new(),
        });
        cx.notify();
    }

    pub(crate) fn remove_track_by_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.new_task_mode || !self.composer_use_multiple_agents {
            return;
        }
        let next: Vec<DraftTrack> = self
            .composer_draft_tracks
            .iter()
            .filter(|track| track.key != key)
            .cloned()
            .collect();
        if next.is_empty() {
            return;
        }
        self.composer_draft_tracks = next;
        cx.notify();
    }

    pub(crate) fn update_track_model(
        &mut self,
        key: &str,
        model_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode {
            return;
        }
        if let Some(track) = self
            .composer_draft_tracks
            .iter_mut()
            .find(|track| track.key == key)
        {
            track.model_id = model_id;
        }
        cx.notify();
    }

    pub(crate) fn select_composer_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
        self.composer_provider_id = Some(provider_id.clone());
        self.composer_open_menu = None;
        if self.new_task_mode {
            if self.composer_use_multiple_agents {
                let exists = self
                    .composer_draft_tracks
                    .iter()
                    .any(|track| track.provider_id == provider_id);
                if !exists {
                    self.composer_draft_tracks.push(DraftTrack {
                        key: format!("t{}", self.composer_draft_tracks.len() + 1),
                        provider_id: provider_id.clone(),
                        model_id: String::new(),
                    });
                }
            } else {
                self.set_single_draft_track(provider_id.clone());
            }
        }
        let mut models =
            model_ids_from_provider_options(self.composer_provider_options.get(&provider_id));
        if models.is_empty() {
            models = self.model_ids_for_provider(&provider_id);
        }
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

    pub(crate) fn select_composer_model(
        &mut self,
        model_id: String,
        cx: &mut Context<Self>,
    ) {
        self.composer_model_id = Some(model_id.clone());
        self.composer_open_menu = None;
        if self.new_task_mode {
            if let Some(track) = self.composer_draft_tracks.first_mut() {
                track.model_id = model_id.clone();
            }
        }
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

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(session_id) => view.load_session_details(session_id, cx),
                        Err(_) => {
                            view.push_message(
                                MessageItem::new(
                                    MessageRole::Assistant,
                                    "Unable to update session model.",
                                ),
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

    fn seed_primary_model_from_options(
        &mut self,
        provider_id: &str,
        options: &ProviderOptions,
        cx: &mut Context<Self>,
    ) {
        if !self.new_task_mode {
            return;
        }
        if self.composer_draft_tracks.len() != 1 {
            return;
        }
        let Some(track) = self.composer_draft_tracks.first_mut() else {
            return;
        };
        if track.provider_id != provider_id {
            return;
        }
        if !track.model_id.trim().is_empty() {
            return;
        }
        let Some(model_id) = model_id_from_provider_options(Some(options)) else {
            return;
        };
        track.model_id = model_id.clone();
        self.composer_model_id = Some(model_id);
        cx.notify();
    }

    pub(crate) fn ensure_provider_options(
        &mut self,
        provider_id: String,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self
            .composer_provider_opts_busy
            .get(&provider_id)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        if !force && self.composer_provider_options.contains_key(&provider_id) {
            return;
        }

        self.composer_provider_opts_busy
            .insert(provider_id.clone(), true);
        cx.notify();

        let provider_id_for_task = provider_id.clone();
        let provider_id_for_update = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let options = client
                .get_provider_options(workspace_id, &provider_id_for_task)
                .await?;
            Ok(options)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.composer_provider_opts_busy
                        .remove(&provider_id_for_update);
                    match result {
                        Ok(options) => {
                            view.seed_primary_model_from_options(
                                &provider_id_for_update,
                                &options,
                                cx,
                            );
                            view.composer_provider_options
                                .insert(provider_id_for_update.clone(), options);
                        }
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn authenticate_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            self.composer_provider_action_error = Some("No workspace selected.".to_string());
            cx.notify();
            return;
        };
        if self
            .composer_provider_auth_busy
            .get(&provider_id)
            .copied()
            .unwrap_or(false)
        {
            return;
        }

        self.composer_provider_auth_busy
            .insert(provider_id.clone(), true);
        self.composer_provider_action_notice = None;
        self.composer_provider_action_error = None;
        cx.notify();

        let provider_id_for_task = provider_id.clone();
        let provider_id_for_update = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let resp = client
                .authenticate_provider_for_workspace(workspace_id, &provider_id_for_task, None)
                .await?;
            Ok(resp)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.composer_provider_auth_busy
                        .remove(&provider_id_for_update);
                    match result {
                        Ok(resp) => {
                            if resp.status != "ok" {
                                view.composer_provider_action_notice =
                                    Some(format!("Authentication status: {}", resp.status));
                            }
                        }
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
                        }
                    }
                    view.ensure_provider_options(provider_id_for_update, true, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn verify_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            self.composer_provider_action_error = Some("No workspace selected.".to_string());
            cx.notify();
            return;
        };
        if self
            .composer_provider_verify_busy
            .get(&provider_id)
            .copied()
            .unwrap_or(false)
        {
            return;
        }

        self.composer_provider_verify_busy
            .insert(provider_id.clone(), true);
        self.composer_provider_action_notice = None;
        self.composer_provider_action_error = None;
        cx.notify();

        let provider_id_for_task = provider_id.clone();
        let provider_id_for_update = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let resp = client
                .verify_provider_for_workspace(workspace_id, &provider_id_for_task)
                .await?;
            Ok(resp)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.composer_provider_verify_busy
                        .remove(&provider_id_for_update);
                    match result {
                        Ok(resp) => {
                            if resp.status != "ok" {
                                let label = if resp.status == "network_error" {
                                    "Verify failed: offline/unreachable.".to_string()
                                } else {
                                    format!("Verify failed: {}", resp.status)
                                };
                                view.composer_provider_action_notice = Some(label);
                            }
                        }
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
                        }
                    }
                    view.ensure_provider_options(provider_id_for_update, true, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn install_provider(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.composer_install_polling.contains(&provider_id) {
            return;
        }
        self.composer_provider_action_error = None;
        self.composer_provider_action_notice = None;
        cx.notify();

        let provider_id_for_task = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let resp = client.install_provider(&provider_id_for_task).await?;
            Ok(resp)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(resp) => view.attach_install(resp.provider_id, resp.install_id, cx),
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(crate) fn install_all_providers(&mut self, cx: &mut Context<Self>) {
        if self.composer_install_all_busy {
            return;
        }
        self.composer_install_all_busy = true;
        self.composer_provider_action_error = None;
        self.composer_provider_action_notice = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let resp = client.install_all_providers().await?;
            Ok(resp)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    view.composer_install_all_busy = false;
                    match result {
                        Ok(installs) => {
                            for install in installs {
                                view.attach_install(install.provider_id, install.install_id, cx);
                            }
                        }
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn attach_install(&mut self, provider_id: String, install_id: String, cx: &mut Context<Self>) {
        if self.composer_install_polling.contains(&provider_id) {
            return;
        }
        self.composer_install_polling.insert(provider_id.clone());
        self.composer_provider_installs
            .insert(provider_id.clone(), ProviderInstallState {
                install_id: install_id.clone(),
                state: InstallStateKind::Running,
                pct: None,
            });
        cx.notify();

        cx.spawn(async move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let client = match ctx_client::resolve_daemon_config()
                .and_then(|config| ctx_client::Client::new(config))
            {
                Ok(client) => client,
                Err(err) => {
                    this.update(cx, |view, cx| {
                        view.composer_install_polling.remove(&provider_id);
                        view.composer_provider_action_error = Some(err.to_string());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            if let Ok(events) = client.list_install_events(&install_id).await {
                if let Some(event) = events.last() {
                    this.update(cx, |view, cx| {
                        view.update_install_event(&provider_id, event);
                        cx.notify();
                    })
                    .ok();
                }
            }

            loop {
                match client.get_install(&install_id).await {
                    Ok(info) => {
                        let running = matches!(info.state, InstallStateKind::Running);
                        this.update(cx, |view, cx| {
                            view.update_install_state(&provider_id, info);
                            if !running {
                                view.composer_install_polling.remove(&provider_id);
                                view.refresh_providers(cx);
                            }
                            cx.notify();
                        })
                        .ok();
                        if !running {
                            break;
                        }
                    }
                    Err(err) => {
                        this.update(cx, |view, cx| {
                            view.composer_install_polling.remove(&provider_id);
                            view.composer_provider_action_error = Some(err.to_string());
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                }

                tokio::time::sleep(Duration::from_millis(900)).await;
            }
        })
        .detach();
    }

    fn update_install_state(&mut self, provider_id: &str, info: InstallInfo) {
        let session = self.composer_provider_installs.entry(provider_id.to_string()).or_insert(
            ProviderInstallState {
                install_id: info.install_id,
                state: info.state.clone(),
                pct: None,
            },
        );
        session.state = info.state;
        if let Some(event) = info.last_event.as_ref() {
            self.update_install_event(provider_id, event);
        }
    }

    fn update_install_event(&mut self, provider_id: &str, event: &InstallProgressEvent) {
        let session = self.composer_provider_installs.entry(provider_id.to_string()).or_insert(
            ProviderInstallState {
                install_id: event.install_id.clone(),
                state: InstallStateKind::Running,
                pct: None,
            },
        );
        session.pct = install_progress_pct(event);
    }

    pub(crate) fn refresh_providers(&mut self, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let providers = client.list_providers().await?;
            Ok(providers)
        });

        cx.spawn(move |this: gpui::WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    match result {
                        Ok(providers) => {
                            view.providers = providers;
                            view.sync_composer_defaults();
                        }
                        Err(err) => {
                            view.composer_provider_action_error = Some(err.to_string());
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

fn install_progress_pct(event: &InstallProgressEvent) -> Option<f32> {
    let Some(total) = event.total_bytes else {
        return None;
    };
    if total == 0 {
        return None;
    }
    let done = event.bytes.unwrap_or(0);
    Some((done as f32 / total as f32) * 100.0)
}

fn detect_composer_autocomplete_token(
    text: &str,
    cursor: usize,
) -> Option<ComposerAutocompleteToken> {
    let safe_cursor = cursor.min(text.len());
    let token_start = match prev_whitespace_index(text, safe_cursor) {
        Some(prev_ws) => prev_ws + 1,
        None => 0,
    };
    if token_start >= text.len() {
        return None;
    }

    let trigger = text.as_bytes().get(token_start).copied()? as char;
    let kind = match trigger {
        '/' => ComposerAutocompleteKind::Slash,
        '@' => ComposerAutocompleteKind::At,
        _ => return None,
    };

    if token_start > 0 {
        let prev = prev_char_boundary(text, token_start);
        if prev < token_start && !is_whitespace(text.as_bytes()[prev]) {
            return None;
        }
    }

    if text
        .as_bytes()
        .get(token_start + 1)
        .copied()
        .map(|b| b as char)
        == Some(trigger)
    {
        return None;
    }

    let token_end = next_whitespace_index(text, safe_cursor);
    let query_start = token_start + 1;
    let query_end = token_end.min(safe_cursor);
    let query = text.get(query_start..query_end)?.to_string();

    Some(ComposerAutocompleteToken {
        kind,
        start: token_start,
        end: token_end,
        query,
    })
}

fn same_autocomplete_token(
    a: Option<&ComposerAutocompleteToken>,
    b: Option<&ComposerAutocompleteToken>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.kind == b.kind && a.start == b.start && a.end == b.end && a.query == b.query
        }
        _ => false,
    }
}

fn apply_composer_autocomplete_completion(
    text: &str,
    token: &ComposerAutocompleteToken,
    replacement: &str,
) -> ComposerAutocompleteOutput {
    let start = token.start.min(text.len());
    let end = token.end.min(text.len()).max(start);
    let before = &text[..start];
    let after = &text[end..];
    let needs_space = after
        .as_bytes()
        .first()
        .map(|b| !is_whitespace(*b))
        .unwrap_or(true);
    let insert = if needs_space {
        format!("{replacement} ")
    } else {
        replacement.to_string()
    };
    let next_text = format!("{before}{insert}{after}");
    let next_cursor = before.len() + insert.len();
    ComposerAutocompleteOutput {
        next_text,
        next_cursor,
    }
}

struct ComposerAutocompleteOutput {
    next_text: String,
    next_cursor: usize,
}

fn is_whitespace(ch: u8) -> bool {
    matches!(ch, b' ' | b'\n' | b'\t')
}

fn prev_whitespace_index(text: &str, cursor: usize) -> Option<usize> {
    let mut idx = cursor;
    while idx > 0 {
        idx = prev_char_boundary(text, idx);
        if is_whitespace(text.as_bytes()[idx]) {
            return Some(idx);
        }
    }
    None
}

fn next_whitespace_index(text: &str, cursor: usize) -> usize {
    let mut idx = cursor;
    while idx < text.len() {
        if is_whitespace(text.as_bytes()[idx]) {
            return idx;
        }
        idx = next_char_boundary(text, idx);
    }
    text.len()
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

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.to_string())
}

fn fallback_slash_commands_for_new_task() -> Vec<SlashCommandDescriptor> {
    vec![
        SlashCommandDescriptor {
            name: "review".to_string(),
            description: Some("Review my current changes and find issues".to_string()),
        },
        SlashCommandDescriptor {
            name: "review-branch".to_string(),
            description: Some("Review a branch".to_string()),
        },
        SlashCommandDescriptor {
            name: "review-commit".to_string(),
            description: Some("Review a commit".to_string()),
        },
        SlashCommandDescriptor {
            name: "init".to_string(),
            description: Some("Create an AGENTS.md file".to_string()),
        },
        SlashCommandDescriptor {
            name: "compact".to_string(),
            description: Some("Summarize conversation to save context".to_string()),
        },
        SlashCommandDescriptor {
            name: "logout".to_string(),
            description: Some("Log out".to_string()),
        },
        SlashCommandDescriptor {
            name: "help".to_string(),
            description: Some("Show help".to_string()),
        },
    ]
}

fn fallback_slash_commands_for_session(provider_id: Option<&str>) -> Vec<SlashCommandDescriptor> {
    match provider_id.unwrap_or("") {
        "codex" => vec![
            SlashCommandDescriptor {
                name: "review".to_string(),
                description: Some("Review my current changes and find issues".to_string()),
            },
            SlashCommandDescriptor {
                name: "review-branch".to_string(),
                description: Some("Review a branch".to_string()),
            },
            SlashCommandDescriptor {
                name: "review-commit".to_string(),
                description: Some("Review a commit".to_string()),
            },
            SlashCommandDescriptor {
                name: "init".to_string(),
                description: Some("Create an AGENTS.md file".to_string()),
            },
            SlashCommandDescriptor {
                name: "compact".to_string(),
                description: Some("Summarize conversation to save context".to_string()),
            },
            SlashCommandDescriptor {
                name: "logout".to_string(),
                description: Some("Log out".to_string()),
            },
        ],
        "claude" => vec![
            SlashCommandDescriptor {
                name: "login".to_string(),
                description: Some("Log in".to_string()),
            },
            SlashCommandDescriptor {
                name: "logout".to_string(),
                description: Some("Log out".to_string()),
            },
            SlashCommandDescriptor {
                name: "compact".to_string(),
                description: Some("Summarize conversation to save context".to_string()),
            },
            SlashCommandDescriptor {
                name: "help".to_string(),
                description: Some("Show help".to_string()),
            },
        ],
        _ => vec![SlashCommandDescriptor {
            name: "compact".to_string(),
            description: Some("Summarize conversation to save context".to_string()),
        }],
    }
}

pub(crate) fn model_ids_from_provider_options(opts: Option<&ProviderOptions>) -> Vec<String> {
    let Some(opts) = opts else {
        return Vec::new();
    };
    let raw = opts.models.as_ref();
    if raw.is_none() {
        return Vec::new();
    }
    let list = raw
        .and_then(|value| value.get("availableModels"))
        .or_else(|| raw.and_then(|value| value.get("available_models")))
        .or_else(|| raw.and_then(|value| value.get("models")))
        .or_else(|| raw);
    let Some(list) = list.and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|value| {
            value
                .get("modelId")
                .or_else(|| value.get("model_id"))
                .or_else(|| value.get("id"))
                .or_else(|| value.get("name"))
                .and_then(|value| value.as_str())
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn model_id_from_provider_options(opts: Option<&ProviderOptions>) -> Option<String> {
    let raw = opts?.models.as_ref()?;
    let current = raw
        .get("currentModelId")
        .or_else(|| raw.get("current_model_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if current.is_some() {
        return current;
    }

    let list = raw
        .get("availableModels")
        .or_else(|| raw.get("available_models"))
        .or_else(|| raw.get("models"))
        .or_else(|| Some(raw))?;
    let list = list.as_array()?;
    let first = list.first()?;
    first
        .get("modelId")
        .or_else(|| first.get("model_id"))
        .or_else(|| first.get("id"))
        .or_else(|| first.get("name"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn compute_menu_placement(
    menu: ComposerMenuId,
    trigger: Bounds<Pixels>,
    menu_bounds: Bounds<Pixels>,
    window: &Window,
) -> PopoverPlacement {
    let viewport = window.bounds().size;
    let margin = px(MENU_MARGIN);
    let gap = px(MENU_GAP);
    let menu_w = menu_bounds.size.width.max(px(160.0));
    let menu_h = menu_bounds.size.height.max(px(40.0));
    let max_width = px(440.0).min(viewport.width - margin * 2.0);

    if menu == ComposerMenuId::Harness {
        let mut left = trigger.right() + px(HARN_MENU_GAP);
        let mut top = trigger.top() + trigger.size.height / 2.0 - menu_h / 2.0;

        if left + menu_w > viewport.width - margin {
            left = (viewport.width - margin - menu_w).max(margin);
        }
        top = clamp_px(top, margin, viewport.height - margin - menu_h);

        let max_h = px(560.0).min((viewport.height - px(140.0)).max(px(120.0)));
        let max_height = Some(max_h);
        if menu_h > max_h {
            top = margin;
        }

        return PopoverPlacement {
            position: point(left, top),
            anchor: Corner::TopLeft,
            max_height,
            max_width: Some(px(320.0)),
        };
    }

    let mut left = trigger.left();
    let mut top = trigger.bottom() + gap;
    let available_down = viewport.height - margin - (trigger.bottom() + gap);
    let available_up = trigger.top() - margin - gap;
    let mut max_height = None;

    if available_down < menu_h && available_up > available_down {
        let max_h = px(120.0).max(available_up);
        let used_h = menu_h.min(max_h);
        top = trigger.top() - gap - used_h;
        if menu_h > max_h {
            max_height = Some(max_h);
        }
    } else {
        let max_h = px(120.0).max(available_down);
        if menu_h > max_h {
            max_height = Some(max_h);
        }
    }

    if left + menu_w > viewport.width - margin {
        left = viewport.width - margin - menu_w;
    }
    if left < margin {
        left = margin;
    }
    if top < margin {
        top = margin;
    }

    PopoverPlacement {
        position: point(left, top),
        anchor: Corner::TopLeft,
        max_height,
        max_width: Some(max_width),
    }
}

fn compute_tooltip_placement(
    trigger: Bounds<Pixels>,
    tooltip_bounds: Bounds<Pixels>,
    window: &Window,
) -> PopoverPlacement {
    let viewport = window.bounds().size;
    let margin = px(MENU_MARGIN);
    let gap = px(8.0);
    let max_width = px(440.0).min(viewport.width - margin * 2.0);

    let available_down = viewport.height - margin - (trigger.bottom() + gap);
    let available_up = trigger.top() - margin - gap;
    let mut open_above =
        available_down < tooltip_bounds.size.height && available_up > available_down;

    let mut max_height = px(320.0).min(if open_above {
        available_up.max(px(0.0))
    } else {
        available_down.max(px(0.0))
    });
    let mut effective_h = tooltip_bounds.size.height.min(max_height);

    let revised_open_above =
        available_down < effective_h && available_up > available_down;
    if revised_open_above != open_above {
        open_above = revised_open_above;
        max_height = px(320.0).min(if open_above {
            available_up.max(px(0.0))
        } else {
            available_down.max(px(0.0))
        });
        effective_h = tooltip_bounds.size.height.min(max_height);
    }

    let mut top = if open_above {
        trigger.top() - gap - effective_h
    } else {
        trigger.bottom() + gap
    };
    top = clamp_px(top, margin, viewport.height - margin - effective_h);

    let mut left = trigger.left();
    if left + tooltip_bounds.size.width > viewport.width - margin {
        left = viewport.width - margin - tooltip_bounds.size.width;
    }
    if left < margin {
        left = margin;
    }

    PopoverPlacement {
        position: point(left, top),
        anchor: Corner::TopLeft,
        max_height: Some(max_height),
        max_width: Some(max_width),
    }
}

fn compute_count_menu_placement(
    trigger: Bounds<Pixels>,
    menu_bounds: Bounds<Pixels>,
    window: &Window,
) -> PopoverPlacement {
    let viewport = window.bounds().size;
    let margin = px(MENU_MARGIN);
    let gap = px(6.0);
    let menu_w = menu_bounds.size.width;
    let menu_h = menu_bounds.size.height;

    let mut left = trigger.right() - menu_w;
    left = clamp_px(left, margin, viewport.width - margin - menu_w);

    let open_down = trigger.bottom() + gap + menu_h <= viewport.height - margin;
    let mut top = if open_down {
        trigger.bottom() + gap
    } else {
        trigger.top() - menu_h - gap
    };
    top = clamp_px(top, margin, viewport.height - margin - menu_h);

    PopoverPlacement {
        position: point(left, top),
        anchor: Corner::TopLeft,
        max_height: None,
        max_width: Some(px(220.0)),
    }
}

fn clamp_px(value: Pixels, min: Pixels, max: Pixels) -> Pixels {
    value.max(min).min(max)
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

fn derive_task_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "New task".to_string();
    }
    let mut lines = trimmed.lines();
    let first = lines.next().unwrap_or(trimmed);
    let mut title = first.trim().to_string();
    if title.len() > 80 {
        title.truncate(77);
        title.push_str("...");
    }
    title
}
