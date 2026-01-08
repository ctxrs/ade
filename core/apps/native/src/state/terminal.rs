use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use gpui::{ClickEvent, Context, FocusHandle, KeyDownEvent, Window};
use gpui_tokio::Tokio;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_core::ids::{SessionId, TaskId, TerminalId, TrackId, WorktreeId, WorkspaceId};
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
    pub(crate) track_id: Option<TrackId>,
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
    pub(crate) fn label(&self) -> &'static str {
        match self {
            TerminalStreamState::Idle => "Idle",
            TerminalStreamState::Connecting => "Connecting",
            TerminalStreamState::Connected => "Connected",
            TerminalStreamState::Reconnecting { .. } => "Reconnecting",
            TerminalStreamState::Error(_) => "Error",
        }
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            TerminalStreamState::Reconnecting { reason } => reason.as_deref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CreateTerminalOptions {
    pub(crate) task_id: Option<TaskId>,
    pub(crate) track_id: Option<TrackId>,
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
    pub(crate) stream_state: TerminalStreamState,
    pub(crate) stream_terminal_id: Option<TerminalId>,
    pub(crate) stream_output: String,
    pub(crate) stream_stop_tx: Option<watch::Sender<bool>>,
    pub(crate) stream_input_tx: Option<mpsc::UnboundedSender<String>>,
    pub(crate) stream_generation: u64,
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
            stream_state: TerminalStreamState::Idle,
            stream_terminal_id: None,
            stream_output: String::new(),
            stream_stop_tx: None,
            stream_input_tx: None,
            stream_generation: 0,
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
            self.clear_stream(cx);
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
        let track_id = if effective_scope == TerminalScope::Task {
            opts.track_id.or(self.context.track_id)
        } else {
            None
        };
        let session_id = if effective_scope == TerminalScope::Task {
            opts.session_id.or(self.context.session_id)
        } else {
            None
        };
        let worktree_id = opts.worktree_id;
        let cwd = clean_opt_string(opts.cwd);
        let shell = clean_opt_string(opts.shell);

        let request = ctx_client::CreateTerminalRequest {
            task_id,
            track_id,
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
                                if view.stream_terminal_id == Some(terminal_id) {
                                    view.clear_stream(cx);
                                }
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
        let modifiers = event.keystroke.modifiers;

        if modifiers.control && !modifiers.platform {
            match event.keystroke.key.as_str() {
                "c" => {
                    self.send_input("\u{3}".to_string(), cx);
                    return;
                }
                "d" => {
                    self.send_input("\u{4}".to_string(), cx);
                    return;
                }
                _ => {}
            }
        }

        match event.keystroke.key.as_str() {
            "enter" => {
                self.send_buffered_input(cx);
            }
            "backspace" => {
                if self.input.delete_backward() {
                    cx.notify();
                }
            }
            "delete" => {
                if self.input.delete_forward() {
                    cx.notify();
                }
            }
            "tab" => {
                self.input.insert_text("\t");
                cx.notify();
            }
            "left" => {
                self.input.move_left(modifiers.shift);
                cx.notify();
            }
            "right" => {
                self.input.move_right(modifiers.shift);
                cx.notify();
            }
            "home" => {
                self.input.move_home(modifiers.shift);
                cx.notify();
            }
            "end" => {
                self.input.move_end(modifiers.shift);
                cx.notify();
            }
            "up" => {
                if !modifiers.shift
                    && !modifiers.alt
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.function
                    && self.input.history_prev()
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
                    && self.input.history_next()
                {
                    cx.notify();
                }
            }
            _ => {
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                if let Some(text) = event.keystroke.key_char.as_ref() {
                    if text != "\n" && text != "\r" {
                        self.input.insert_text(text);
                        cx.notify();
                    }
                }
            }
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
        let Some(input_tx) = self.stream_input_tx.as_ref() else {
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
        let input = self.input.text().to_string();
        if input.is_empty() {
            self.send_input("\r".to_string(), cx);
            self.input.clear();
            cx.notify();
            return;
        }
        self.input.push_history(input.clone());
        self.send_input(format!("{input}\r"), cx);
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
                if self.stream_terminal_id != Some(selected) {
                    self.start_terminal_stream(selected, false, cx);
                }
                return;
            }
        }
        let previous_selection = self.selected_terminal_id;
        self.selected_terminal_id = scope_terminals.first().map(|terminal| terminal.id);
        if self.selected_terminal_id != previous_selection {
            if let Some(terminal_id) = self.selected_terminal_id {
                self.start_terminal_stream(terminal_id, false, cx);
            } else {
                self.clear_stream(cx);
            }
        }
    }

    fn start_terminal_stream(
        &mut self,
        terminal_id: TerminalId,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !force
            && self.stream_terminal_id == Some(terminal_id)
            && !matches!(self.stream_state, TerminalStreamState::Error(_))
        {
            return;
        }
        self.stop_terminal_stream();
        self.stream_terminal_id = Some(terminal_id);
        self.stream_output.clear();
        let stream_url = ctx_client::resolve_daemon_config()
            .and_then(ctx_client::Client::new)
            .and_then(|client| client.terminal_stream_url(terminal_id));
        let url = match stream_url {
            Ok(url) => url,
            Err(err) => {
                self.stream_state = TerminalStreamState::Error(err.to_string());
                cx.notify();
                return;
            }
        };

        let (stop_tx, stop_rx) = watch::channel(false);
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (update_tx, mut update_rx) = mpsc::unbounded_channel();

        self.stream_generation = self.stream_generation.wrapping_add(1);
        let generation = self.stream_generation;
        self.stream_stop_tx = Some(stop_tx);
        self.stream_input_tx = Some(input_tx);
        self.stream_state = TerminalStreamState::Connecting;
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
                            view.handle_stream_update(update, generation, cx);
                        });
                    }
                }
            },
        )
        .detach();
    }

    fn handle_stream_update(
        &mut self,
        update: TerminalStreamUpdate,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.stream_generation {
            return;
        }
        match update {
            TerminalStreamUpdate::Status(status) => {
                self.stream_state = status;
            }
            TerminalStreamUpdate::Output(chunk) => {
                self.append_stream_output(&chunk);
            }
        }
        cx.notify();
    }

    fn append_stream_output(&mut self, chunk: &str) {
        const MAX_STREAM_OUTPUT: usize = 20_000;
        if chunk.is_empty() {
            return;
        }
        self.stream_output.push_str(chunk);
        if self.stream_output.len() > MAX_STREAM_OUTPUT {
            let overflow = self.stream_output.len() - MAX_STREAM_OUTPUT;
            self.stream_output.drain(0..overflow);
        }
    }

    fn stop_terminal_stream(&mut self) {
        if let Some(stop_tx) = self.stream_stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        self.stream_input_tx = None;
    }

    fn clear_stream(&mut self, cx: &mut Context<Self>) {
        self.stop_terminal_stream();
        self.stream_terminal_id = None;
        self.stream_state = TerminalStreamState::Idle;
        self.stream_output.clear();
        cx.notify();
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
