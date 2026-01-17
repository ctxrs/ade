use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{cell::Flags, Config, Term};
use alacritty_terminal::vte::ansi;
use futures_util::{SinkExt, StreamExt};
use gpui::{ClickEvent, ClipboardItem, Context, FocusHandle, KeyDownEvent, Window};
use gpui_tokio::Tokio;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::ids::{SessionId, TaskId, TerminalId, WorktreeId, WorkspaceId};
use ctx_core::models::TerminalSession;

use super::ComposerState;
use crate::theme::ThemeColors;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalScope {
    Task,
    Workspace,
}

impl TerminalScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            TerminalScope::Task => "Task",
            TerminalScope::Workspace => "Workspace",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalContext {
    pub(crate) workspace_id: Option<WorkspaceId>,
    pub(crate) task_id: Option<TaskId>,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
}

#[derive(Clone, Debug)]
pub(crate) enum TerminalLoadState {
    Idle,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Clone, Debug)]
pub(crate) enum TerminalStreamState {
    Idle,
    Connecting,
    Connected,
    Reconnecting { reason: Option<String> },
    Error(String),
}

impl TerminalStreamState {
    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            TerminalStreamState::Reconnecting { reason } => reason.as_deref(),
            _ => None,
        }
    }
}

const DEFAULT_TERMINAL_COLUMNS: usize = 120;
const DEFAULT_TERMINAL_LINES: usize = 200;
const DEFAULT_TERMINAL_SCROLLBACK: usize = 1000;
const MAX_TERMINAL_STREAMS: usize = 8;

struct TerminalGridSize {
    columns: usize,
    screen_lines: usize,
}

impl TerminalGridSize {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for TerminalGridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

struct TerminalEmulator {
    term: Term<VoidListener>,
    parser: ansi::Processor,
}

impl TerminalEmulator {
    fn new() -> Self {
        let size = TerminalGridSize::new(DEFAULT_TERMINAL_COLUMNS, DEFAULT_TERMINAL_LINES);
        let config = Config {
            scrolling_history: DEFAULT_TERMINAL_SCROLLBACK,
            ..Default::default()
        };
        let term = Term::new(config, &size, VoidListener);
        let parser = ansi::Processor::new();
        Self { term, parser }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn render_text(&self) -> String {
        let grid = self.term.grid();
        let columns = grid.columns();
        let display_offset = grid.display_offset();
        let screen_lines = grid.screen_lines();
        let start_line = -(display_offset as i32);
        let end_line = start_line + screen_lines as i32;
        let mut lines = Vec::with_capacity(screen_lines);

        for line in start_line..end_line {
            let row = &grid[Line(line)];
            let mut line_buf = String::with_capacity(columns);
            for column in 0..columns {
                let cell = &row[Column(column)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                    || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                if cell.flags.contains(Flags::HIDDEN) {
                    line_buf.push(' ');
                } else {
                    line_buf.push(cell.c);
                }
                if let Some(zerowidth) = cell.zerowidth() {
                    for ch in zerowidth {
                        line_buf.push(*ch);
                    }
                }
            }
            lines.push(line_buf.trim_end_matches(' ').to_string());
        }

        while matches!(lines.last(), Some(line) if line.is_empty()) {
            lines.pop();
        }

        lines.join("\n")
    }
}

struct TerminalStreamEntry {
    output: String,
    emulator: TerminalEmulator,
    state: TerminalStreamState,
    stop_tx: Option<watch::Sender<bool>>,
    input_tx: Option<mpsc::UnboundedSender<String>>,
    generation: u64,
}

impl TerminalStreamEntry {
    fn new(state: TerminalStreamState) -> Self {
        let emulator = TerminalEmulator::new();
        let output = emulator.render_text();
        Self {
            output,
            emulator,
            state,
            stop_tx: None,
            input_tx: None,
            generation: 0,
        }
    }

    fn with_channels(
        emulator: TerminalEmulator,
        state: TerminalStreamState,
        stop_tx: watch::Sender<bool>,
        input_tx: mpsc::UnboundedSender<String>,
        generation: u64,
    ) -> Self {
        let output = emulator.render_text();
        Self {
            output,
            emulator,
            state,
            stop_tx: Some(stop_tx),
            input_tx: Some(input_tx),
            generation,
        }
    }

    fn append_output(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.emulator.feed_bytes(chunk.as_bytes());
        self.output = self.emulator.render_text();
    }

    fn clear_output(&mut self) {
        self.emulator.reset();
        self.output.clear();
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CreateTerminalOptions {
    pub(crate) task_id: Option<TaskId>,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) cwd: Option<String>,
    pub(crate) shell: Option<String>,
    pub(crate) scope: Option<TerminalScope>,
}

pub(crate) struct TerminalPanelState {
    pub(crate) colors: ThemeColors,
    pub(crate) scope: TerminalScope,
    pub(crate) context: TerminalContext,
    pub(crate) terminals: Vec<TerminalSession>,
    pub(crate) selected_terminal_id: Option<TerminalId>,
    pub(crate) load_state: TerminalLoadState,
    terminal_streams: HashMap<TerminalId, TerminalStreamEntry>,
    terminal_stream_lru: VecDeque<TerminalId>,
    stream_generation_counter: u64,
    pub(crate) last_error: Option<String>,
    pub(crate) input: ComposerState,
    pub(crate) input_focus: FocusHandle,
}

impl TerminalPanelState {
    pub(crate) fn new(colors: ThemeColors, input_focus: FocusHandle) -> Self {
        Self {
            colors,
            scope: TerminalScope::Workspace,
            context: TerminalContext::default(),
            terminals: Vec::new(),
            selected_terminal_id: None,
            load_state: TerminalLoadState::Idle,
            terminal_streams: HashMap::new(),
            terminal_stream_lru: VecDeque::new(),
            stream_generation_counter: 0,
            last_error: None,
            input: ComposerState::new(),
            input_focus,
        }
    }

    pub(crate) fn set_context(&mut self, context: TerminalContext, cx: &mut Context<Self>) {
        let workspace_changed = self.context.workspace_id != context.workspace_id;
        self.context = context;
        if self.scope == TerminalScope::Task && self.context.task_id.is_none() {
            self.scope = TerminalScope::Workspace;
        }
        if workspace_changed {
            self.terminals.clear();
            self.selected_terminal_id = None;
            self.load_state = TerminalLoadState::Idle;
            self.clear_streams(cx);
            self.load_terminals(cx);
        } else {
            self.reconcile_selection(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_scope(&mut self, scope: TerminalScope, cx: &mut Context<Self>) {
        if scope == TerminalScope::Task && self.context.task_id.is_none() {
            self.last_error = Some("No active task selected for task terminals.".to_string());
            cx.notify();
            return;
        }
        if self.scope != scope {
            self.scope = scope;
            self.reconcile_selection(cx);
            cx.notify();
        }
    }

    pub(crate) fn select_terminal(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        self.selected_terminal_id = Some(terminal_id);
        self.input.clear();
        self.start_terminal_stream(terminal_id, false, cx);
        self.touch_stream_lru(terminal_id);
        cx.notify();
    }

    pub(crate) fn load_terminals(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.context.workspace_id else {
            self.load_state = TerminalLoadState::Error("No workspace selected.".to_string());
            self.last_error = Some("Select a workspace to load terminals.".to_string());
            cx.notify();
            return;
        };

        self.load_state = TerminalLoadState::Loading;
        self.last_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let terminals = client.list_workspace_terminals(workspace_id).await?;
            Ok(terminals)
        });

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        if view.context.workspace_id != Some(workspace_id) {
                            return;
                        }
                        match result {
                            Ok(terminals) => {
                                view.terminals = terminals;
                                view.load_state = TerminalLoadState::Loaded;
                                view.last_error = None;
                                view.prune_streams();
                                view.reconcile_selection(cx);
                            }
                            Err(err) => {
                                view.load_state = TerminalLoadState::Error(
                                    "Unable to load terminals.".to_string(),
                                );
                                view.last_error = Some(err.to_string());
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    pub(crate) fn create_terminal(&mut self, opts: CreateTerminalOptions, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.context.workspace_id else {
            self.last_error = Some("Select a workspace before creating a terminal.".to_string());
            cx.notify();
            return;
        };

        self.last_error = None;
        let requested_scope = opts.scope.unwrap_or(self.scope);
        let effective_scope = if requested_scope == TerminalScope::Task
            && self.context.task_id.is_none()
        {
            TerminalScope::Workspace
        } else {
            requested_scope
        };
        if self.scope != effective_scope {
            self.scope = effective_scope;
        }

        let task_id = if effective_scope == TerminalScope::Task {
            opts.task_id.or(self.context.task_id)
        } else {
            None
        };
        let session_id = if effective_scope == TerminalScope::Task {
            opts.session_id.or(self.context.session_id)
        } else {
            None
        };
        let worktree_id = if effective_scope == TerminalScope::Task {
            opts.worktree_id.or(self.context.worktree_id)
        } else {
            opts.worktree_id
        };
        let cwd = clean_opt_string(opts.cwd);
        let shell = clean_opt_string(opts.shell);

        let request = ctx_client::CreateTerminalRequest {
            task_id,
            session_id,
            worktree_id,
            cwd,
            shell,
        };

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let terminal = client.create_workspace_terminal(workspace_id, &request).await?;
            Ok(terminal)
        });

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        if view.context.workspace_id != Some(workspace_id) {
                            return;
                        }
                        match result {
                            Ok(terminal) => {
                                view.terminals.push(terminal.clone());
                                view.selected_terminal_id = Some(terminal.id);
                                view.start_terminal_stream(terminal.id, false, cx);
                                view.touch_stream_lru(terminal.id);
                                view.last_error = None;
                            }
                            Err(err) => {
                                view.last_error = Some(err.to_string());
                            }
                        }
                        view.reconcile_selection(cx);
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    pub(crate) fn delete_terminal(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.delete_terminal(terminal_id).await?;
            Ok(terminal_id)
        });

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        match result {
                            Ok(terminal_id) => {
                                view.terminals.retain(|terminal| terminal.id != terminal_id);
                                if view.selected_terminal_id == Some(terminal_id) {
                                    view.selected_terminal_id = None;
                                }
                                view.stop_terminal_stream(terminal_id);
                                view.reconcile_selection(cx);
                                view.last_error = None;
                            }
                            Err(err) => {
                                view.last_error = Some(err.to_string());
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    pub(crate) fn on_refresh_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_terminals(cx);
    }

    pub(crate) fn on_create_terminal_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_terminal(CreateTerminalOptions::default(), cx);
    }

    pub(crate) fn on_scope_task_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(TerminalScope::Task, cx);
    }

    pub(crate) fn on_scope_workspace_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(TerminalScope::Workspace, cx);
    }

    pub(crate) fn on_reconnect_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal_id) = self.selected_terminal_id else {
            self.last_error = Some("Select a terminal to reconnect.".to_string());
            cx.notify();
            return;
        };
        self.last_error = None;
        self.start_terminal_stream(terminal_id, true, cx);
    }

    pub(crate) fn on_clear_output_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_stream_output().is_empty() {
            return;
        }
        if let Some(entry) = self.active_stream_entry_mut() {
            entry.clear_output();
        }
        cx.notify();
    }

    pub(crate) fn on_copy_output_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let output = self.active_stream_output().trim_end();
        if output.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(output.to_string()));
        cx.notify();
    }

    pub(crate) fn focus_input(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input_focus.focus(window, cx);
    }

    pub(crate) fn on_input_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_terminal_id.is_none() {
            return;
        }
        if let Some(payload) = terminal_input_payload(event) {
            self.send_input(payload, cx);
        }
    }

    pub(crate) fn on_send_input_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_buffered_input(cx);
    }

    pub(crate) fn send_input(&mut self, data: String, cx: &mut Context<Self>) {
        if data.is_empty() {
            return;
        }
        if self.selected_terminal_id.is_none() {
            self.last_error = Some("Select a terminal before sending input.".to_string());
            cx.notify();
            return;
        }
        let Some(input_tx) = self.active_stream_input_tx() else {
            self.last_error = Some("Terminal input is unavailable. Reconnect and try again.".to_string());
            cx.notify();
            return;
        };
        if input_tx.send(data).is_err() {
            self.last_error = Some("Terminal input channel closed. Reconnect and try again.".to_string());
        } else {
            self.last_error = None;
        }
        cx.notify();
    }

    fn send_buffered_input(&mut self, cx: &mut Context<Self>) {
        self.send_input("\r".to_string(), cx);
        self.input.clear();
        cx.notify();
    }

    pub(crate) fn scope_terminals(&self) -> Vec<&TerminalSession> {
        match (self.scope, self.context.task_id) {
            (TerminalScope::Task, Some(task_id)) => self
                .terminals
                .iter()
                .filter(|terminal| terminal.task_id == Some(task_id))
                .collect(),
            (TerminalScope::Task, None) => Vec::new(),
            (TerminalScope::Workspace, _) => self.terminals.iter().collect(),
        }
    }

    fn reconcile_selection(&mut self, cx: &mut Context<Self>) {
        let scope_terminals = self.scope_terminals();
        if let Some(selected) = self.selected_terminal_id {
            if scope_terminals
                .iter()
                .any(|terminal| terminal.id == selected)
            {
                self.start_terminal_stream(selected, false, cx);
                self.touch_stream_lru(selected);
                return;
            }
        }
        let previous_selection = self.selected_terminal_id;
        self.selected_terminal_id = scope_terminals.first().map(|terminal| terminal.id);
        if self.selected_terminal_id != previous_selection {
            if let Some(terminal_id) = self.selected_terminal_id {
                self.start_terminal_stream(terminal_id, false, cx);
                self.touch_stream_lru(terminal_id);
            }
            cx.notify();
        }
    }

    fn start_terminal_stream(
        &mut self,
        terminal_id: TerminalId,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !force {
            if let Some(entry) = self.terminal_streams.get(&terminal_id) {
                if !matches!(entry.state, TerminalStreamState::Error(_)) {
                    self.touch_stream_lru(terminal_id);
                    return;
                }
            }
        }
        let preserved_emulator = if force {
            self.stop_terminal_stream(terminal_id);
            None
        } else {
            self.stop_terminal_stream(terminal_id)
        };
        let stream_url = ctx_client::resolve_daemon_config()
            .and_then(ctx_client::Client::new)
            .and_then(|client| client.terminal_stream_url(terminal_id));
        let url = match stream_url {
            Ok(url) => url,
            Err(err) => {
                let mut entry =
                    TerminalStreamEntry::new(TerminalStreamState::Error(err.to_string()));
                if let Some(emulator) = preserved_emulator {
                    entry.emulator = emulator;
                    entry.output = entry.emulator.render_text();
                }
                self.terminal_streams.insert(terminal_id, entry);
                self.touch_stream_lru(terminal_id);
                cx.notify();
                return;
            }
        };

        let (stop_tx, stop_rx) = watch::channel(false);
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (update_tx, mut update_rx) = mpsc::unbounded_channel();

        self.stream_generation_counter = self.stream_generation_counter.wrapping_add(1);
        let generation = self.stream_generation_counter;
        let emulator = preserved_emulator.unwrap_or_else(TerminalEmulator::new);
        let entry = TerminalStreamEntry::with_channels(
            emulator,
            TerminalStreamState::Connecting,
            stop_tx,
            input_tx,
            generation,
        );
        self.terminal_streams.insert(terminal_id, entry);
        self.touch_stream_lru(terminal_id);
        self.enforce_stream_limit();
        cx.notify();

        let stream_task = Tokio::spawn_result(cx, async move {
            run_terminal_stream(url, stop_rx, input_rx, update_tx).await
        });

        cx.spawn(
            move |_: gpui::WeakEntity<Self>, _: &mut gpui::AsyncApp| async move {
            let _ = stream_task.await;
        },
        )
        .detach();

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    while let Some(update) = update_rx.recv().await {
                        let _ = this.update(&mut cx, |view, cx| {
                            view.handle_stream_update(terminal_id, update, generation, cx);
                        });
                    }
                }
            },
        )
        .detach();
    }

    fn handle_stream_update(
        &mut self,
        terminal_id: TerminalId,
        update: TerminalStreamUpdate,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.terminal_streams.get_mut(&terminal_id) else {
            return;
        };
        if generation != entry.generation {
            return;
        }
        match update {
            TerminalStreamUpdate::Status(status) => {
                entry.state = status;
            }
            TerminalStreamUpdate::Output(chunk) => {
                entry.append_output(&chunk);
            }
        }
        if self.selected_terminal_id == Some(terminal_id) {
            cx.notify();
        }
    }

    fn stop_terminal_stream(&mut self, terminal_id: TerminalId) -> Option<TerminalEmulator> {
        if let Some(entry) = self.terminal_streams.remove(&terminal_id) {
            if let Some(stop_tx) = entry.stop_tx {
                let _ = stop_tx.send(true);
            }
            self.remove_from_lru(terminal_id);
            return Some(entry.emulator);
        }
        None
    }

    fn clear_streams(&mut self, cx: &mut Context<Self>) {
        let terminal_ids: Vec<TerminalId> = self.terminal_streams.keys().copied().collect();
        for terminal_id in terminal_ids {
            self.stop_terminal_stream(terminal_id);
        }
        self.terminal_streams.clear();
        self.terminal_stream_lru.clear();
        cx.notify();
    }

    fn touch_stream_lru(&mut self, terminal_id: TerminalId) {
        self.remove_from_lru(terminal_id);
        self.terminal_stream_lru.push_back(terminal_id);
    }

    fn remove_from_lru(&mut self, terminal_id: TerminalId) {
        if let Some(position) = self
            .terminal_stream_lru
            .iter()
            .position(|id| *id == terminal_id)
        {
            self.terminal_stream_lru.remove(position);
        }
    }

    fn enforce_stream_limit(&mut self) {
        if self.terminal_streams.len() <= MAX_TERMINAL_STREAMS {
            return;
        }
        let selected = self.selected_terminal_id;
        while self.terminal_streams.len() > MAX_TERMINAL_STREAMS {
            let Some(candidate) = self
                .terminal_stream_lru
                .iter()
                .copied()
                .find(|id| Some(*id) != selected)
            else {
                break;
            };
            self.stop_terminal_stream(candidate);
        }
    }

    fn prune_streams(&mut self) {
        let keep: HashSet<TerminalId> = self.terminals.iter().map(|terminal| terminal.id).collect();
        let to_remove: Vec<TerminalId> = self
            .terminal_streams
            .keys()
            .copied()
            .filter(|id| !keep.contains(id))
            .collect();
        for terminal_id in to_remove {
            self.stop_terminal_stream(terminal_id);
        }
    }

    fn active_stream_entry(&self) -> Option<&TerminalStreamEntry> {
        self.selected_terminal_id
            .and_then(|terminal_id| self.terminal_streams.get(&terminal_id))
    }

    fn active_stream_entry_mut(&mut self) -> Option<&mut TerminalStreamEntry> {
        self.selected_terminal_id
            .and_then(|terminal_id| self.terminal_streams.get_mut(&terminal_id))
    }

    pub(crate) fn active_stream_state(&self) -> TerminalStreamState {
        self.active_stream_entry()
            .map(|entry| entry.state.clone())
            .unwrap_or(TerminalStreamState::Idle)
    }

    pub(crate) fn active_stream_output(&self) -> &str {
        self.active_stream_entry()
            .map(|entry| entry.output.as_str())
            .unwrap_or("")
    }

    fn active_stream_input_tx(&self) -> Option<mpsc::UnboundedSender<String>> {
        self.active_stream_entry()
            .and_then(|entry| entry.input_tx.clone())
    }
}

fn clean_opt_string(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

enum TerminalStreamUpdate {
    Status(TerminalStreamState),
    Output(String),
}

async fn run_terminal_stream(
    ws_url: String,
    mut stop_rx: watch::Receiver<bool>,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    update_tx: mpsc::UnboundedSender<TerminalStreamUpdate>,
) -> anyhow::Result<()> {
    let mut backoff = Duration::from_secs(1);
    loop {
        if *stop_rx.borrow() {
            let _ = update_tx.send(TerminalStreamUpdate::Status(TerminalStreamState::Idle));
            return Ok(());
        }

        if update_tx
            .send(TerminalStreamUpdate::Status(TerminalStreamState::Connecting))
            .is_err()
        {
            return Ok(());
        }

        let (socket, _) = match connect_async(&ws_url).await {
            Ok(connection) => connection,
            Err(err) => {
                let _ = update_tx.send(TerminalStreamUpdate::Status(
                    TerminalStreamState::Reconnecting {
                        reason: Some(err.to_string()),
                    },
                ));
                tokio::time::sleep(backoff).await;
                backoff = (backoff + backoff).min(Duration::from_secs(10));
                continue;
            }
        };

        backoff = Duration::from_secs(1);
        let (mut write, mut read) = socket.split();

        let _ = update_tx.send(TerminalStreamUpdate::Status(
            TerminalStreamState::Connected,
        ));

        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    let _ = update_tx.send(TerminalStreamUpdate::Status(TerminalStreamState::Idle));
                    return Ok(());
                }
                input = input_rx.recv() => {
                    if let Some(data) = input {
                        if write.send(WsMessage::Text(data.into())).await.is_err() {
                            break;
                        }
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(frame)) => {
                            match frame {
                                WsMessage::Ping(payload) => {
                                    let _ = write.send(WsMessage::Pong(payload)).await;
                                }
                                WsMessage::Close(_) => break,
                                _ => {
                                    if let Some(text) = parse_terminal_stream_output(frame) {
                                        if update_tx.send(TerminalStreamUpdate::Output(text)).is_err() {
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                        Some(Err(_)) | None => break,
                    }
                }
            }
        }

        let _ = update_tx.send(TerminalStreamUpdate::Status(
            TerminalStreamState::Reconnecting {
                reason: Some("stream disconnected".to_string()),
            },
        ));
        tokio::time::sleep(backoff).await;
        backoff = (backoff + backoff).min(Duration::from_secs(10));
    }
}

fn parse_terminal_stream_output(message: WsMessage) -> Option<String> {
    match message {
        WsMessage::Text(text) => Some(text.to_string()),
        WsMessage::Binary(bytes) => Some(String::from_utf8_lossy(&bytes).to_string()),
        _ => None,
    }
}

fn terminal_input_payload(event: &KeyDownEvent) -> Option<String> {
    let modifiers = event.keystroke.modifiers;
    if modifiers.platform {
        return None;
    }

    if modifiers.control && !modifiers.platform {
        if let Some(text) = event.keystroke.key_char.as_ref() {
            if let Some(ch) = text.chars().next() {
                if ch.is_ascii() {
                    let upper = ch.to_ascii_uppercase();
                    let code = (upper as u8) & 0x1f;
                    if code != 0 {
                        return Some((code as char).to_string());
                    }
                }
            }
        }
    }

    match event.keystroke.key.as_str() {
        "enter" => return Some("\r".to_string()),
        "tab" => {
            return if modifiers.shift {
                Some("\u{1b}[Z".to_string())
            } else {
                Some("\t".to_string())
            };
        }
        "backspace" => return Some("\u{7f}".to_string()),
        "delete" => return Some("\u{1b}[3~".to_string()),
        "left" => return Some("\u{1b}[D".to_string()),
        "right" => return Some("\u{1b}[C".to_string()),
        "up" => return Some("\u{1b}[A".to_string()),
        "down" => return Some("\u{1b}[B".to_string()),
        "home" => return Some("\u{1b}[H".to_string()),
        "end" => return Some("\u{1b}[F".to_string()),
        "pageup" => return Some("\u{1b}[5~".to_string()),
        "pagedown" => return Some("\u{1b}[6~".to_string()),
        "escape" => return Some("\u{1b}".to_string()),
        _ => {}
    }

    if modifiers.alt {
        if let Some(text) = event.keystroke.key_char.as_ref() {
            if !text.is_empty() {
                return Some(format!("\u{1b}{text}"));
            }
        }
        return None;
    }

    if modifiers.control || modifiers.function {
        return None;
    }

    if let Some(text) = event.keystroke.key_char.as_ref() {
        if text != "\n" && text != "\r" {
            return Some(text.to_string());
        }
    }
    None
}
