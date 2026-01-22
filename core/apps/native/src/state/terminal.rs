use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Indexed, Scroll};
use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{cell::Flags, viewport_to_point, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, Color, CursorShape, NamedColor, Rgb};
use futures_util::{SinkExt, StreamExt};
use gpui::{
    font, px, Bounds, ClickEvent, ClipboardItem, Context, FocusHandle, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point as UiPoint, Rgba,
    ScrollWheelEvent, SharedString, StrikethroughStyle, TextRun, UnderlineStyle, Window,
};
use gpui_tokio::Tokio;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use ctx_client::{PortPreviewEntry, UpdatePortForwardingSettingsRequest, UpdateSettingsRequest};
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
pub(crate) const TERMINAL_FONT_SIZE: f32 = 12.0;
pub(crate) const TERMINAL_LINE_HEIGHT: f32 = 16.0;
pub(crate) const MONO_FONT_FAMILY: &str =
    "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace";
const DIM_FACTOR: f32 = 0.66;
const DEFAULT_ANSI_COLORS: [Rgb; 16] = [
    Rgb { r: 0, g: 0, b: 0 },
    Rgb { r: 205, g: 0, b: 0 },
    Rgb { r: 0, g: 205, b: 0 },
    Rgb { r: 205, g: 205, b: 0 },
    Rgb { r: 0, g: 0, b: 238 },
    Rgb { r: 205, g: 0, b: 205 },
    Rgb { r: 0, g: 205, b: 205 },
    Rgb { r: 229, g: 229, b: 229 },
    Rgb { r: 127, g: 127, b: 127 },
    Rgb { r: 255, g: 0, b: 0 },
    Rgb { r: 0, g: 255, b: 0 },
    Rgb { r: 255, g: 255, b: 0 },
    Rgb { r: 92, g: 92, b: 255 },
    Rgb { r: 255, g: 0, b: 255 },
    Rgb { r: 0, g: 255, b: 255 },
    Rgb { r: 255, g: 255, b: 255 },
];

#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalRenderSnapshot {
    pub(crate) text: SharedString,
    pub(crate) runs: Vec<TextRun>,
}

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

#[derive(Clone, PartialEq)]
struct TerminalTextStyle {
    font: gpui::Font,
    color: Hsla,
    background_color: Option<Hsla>,
    underline: Option<UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
}

impl TerminalTextStyle {
    fn to_run(&self, len: usize) -> TextRun {
        TextRun {
            len,
            font: self.font.clone(),
            color: self.color,
            background_color: self.background_color,
            underline: self.underline,
            strikethrough: self.strikethrough,
        }
    }
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

    fn feed_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn render_plain_text(&self) -> String {
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

    fn render_snapshot(&self, theme: ThemeColors) -> TerminalRenderSnapshot {
        let grid = self.term.grid();
        let columns = grid.columns();
        let display_offset = grid.display_offset();
        let screen_lines = grid.screen_lines();
        let start_line = -(display_offset as i32);
        let end_line = start_line + screen_lines as i32;
        let content = self.term.renderable_content();
        let cursor = content.cursor;
        let cursor_rgb = resolve_named_color(NamedColor::Cursor, content.colors, theme);
        let default_fg = resolve_named_color(NamedColor::Foreground, content.colors, theme);
        let selection_range: Option<SelectionRange> =
            self.term.selection.as_ref().and_then(|selection| selection.to_range(&self.term));
        let selection_bg = rgba_to_rgb(theme.accent);
        let selection_fg = rgba_to_rgb(theme.bg);

        let mut text = String::with_capacity(columns * screen_lines);
        let mut runs = Vec::new();
        let mut current_style: Option<TerminalTextStyle> = None;
        let mut current_len = 0usize;

        let base_font = font(MONO_FONT_FAMILY);
        let newline_style = TerminalTextStyle {
            font: base_font.clone(),
            color: rgb_to_hsla(default_fg),
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        for line in start_line..end_line {
            let row = &grid[Line(line)];
            for column in 0..columns {
                let cell = &row[Column(column)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                    || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let point = TermPoint::new(Line(line), Column(column));

                let mut bold = cell.flags.contains(Flags::BOLD);
                let italic = cell.flags.contains(Flags::ITALIC);
                let dim = cell.flags.contains(Flags::DIM);
                let underline_flag = cell.flags.intersects(Flags::ALL_UNDERLINES);
                let strike = cell.flags.contains(Flags::STRIKEOUT);

                let mut fg =
                    resolve_color(cell.fg, content.colors, theme, true, bold, dim);
                let mut bg =
                    resolve_color(cell.bg, content.colors, theme, false, false, false);

                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }

                if cell.flags.contains(Flags::HIDDEN) {
                    fg = bg;
                }

                let is_selected = selection_range
                    .as_ref()
                    .map(|range| {
                        range.contains_cell(
                            &Indexed { point, cell },
                            cursor.point,
                            cursor.shape,
                        )
                    })
                    .unwrap_or(false);
                let is_cursor = cursor.shape != CursorShape::Hidden
                    && cursor.point.line.0 == line
                    && cursor.point.column.0 == column;

                let mut underline = if underline_flag {
                    let color = cell
                        .underline_color()
                        .map(|color| resolve_color(color, content.colors, theme, true, false, false))
                        .unwrap_or(fg);
                    Some(underline_style_for_cell(cell, color))
                } else {
                    None
                };

                let strikethrough = if strike {
                    Some(StrikethroughStyle {
                        thickness: px(1.0),
                        color: Some(rgb_to_hsla(fg)),
                    })
                } else {
                    None
                };

                if is_selected {
                    fg = selection_fg;
                    bg = selection_bg;
                }

                if is_cursor {
                    match cursor.shape {
                        CursorShape::Block | CursorShape::HollowBlock => {
                            let cursor_fg = bg;
                            fg = cursor_fg;
                            bg = cursor_rgb;
                            bold = true;
                        }
                        CursorShape::Underline | CursorShape::Beam => {
                            underline = Some(UnderlineStyle {
                                thickness: px(1.0),
                                color: Some(rgb_to_hsla(cursor_rgb)),
                                wavy: false,
                            });
                        }
                        CursorShape::Hidden => {}
                    }
                }

                let mut font = base_font.clone();
                if bold {
                    font = font.bold();
                }
                if italic {
                    font = font.italic();
                }

                let style = TerminalTextStyle {
                    font,
                    color: rgb_to_hsla(fg),
                    background_color: Some(rgb_to_hsla(bg)),
                    underline,
                    strikethrough,
                };

                let ch = if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
                push_styled_char(
                    &mut text,
                    &mut runs,
                    &mut current_style,
                    &mut current_len,
                    &style,
                    ch,
                );

                if let Some(zerowidth) = cell.zerowidth() {
                    for &ch in zerowidth {
                        push_styled_char(
                            &mut text,
                            &mut runs,
                            &mut current_style,
                            &mut current_len,
                            &style,
                            ch,
                        );
                    }
                }
            }

            if line + 1 < end_line {
                push_styled_char(
                    &mut text,
                    &mut runs,
                    &mut current_style,
                    &mut current_len,
                    &newline_style,
                    '\n',
                );
            }
        }

        if let Some(style) = current_style {
            if current_len > 0 {
                runs.push(style.to_run(current_len));
            }
        }

        TerminalRenderSnapshot {
            text: SharedString::from(text),
            runs,
        }
    }

    fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    fn scroll_display(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
    }

    fn start_selection(&mut self, point: TermPoint) {
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    fn update_selection(&mut self, point: TermPoint) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
            selection.include_all();
        }
    }

    fn selection_to_string(&self) -> Option<String> {
        self.term.selection_to_string()
    }
}

struct TerminalStreamEntry {
    output: String,
    rendered: TerminalRenderSnapshot,
    emulator: TerminalEmulator,
    state: TerminalStreamState,
    stop_tx: Option<watch::Sender<bool>>,
    input_tx: Option<mpsc::UnboundedSender<String>>,
    generation: u64,
}

impl TerminalStreamEntry {
    fn new(state: TerminalStreamState, theme: ThemeColors) -> Self {
        let emulator = TerminalEmulator::new();
        let output = emulator.render_plain_text();
        let rendered = emulator.render_snapshot(theme);
        Self {
            output,
            rendered,
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
        theme: ThemeColors,
    ) -> Self {
        let output = emulator.render_plain_text();
        let rendered = emulator.render_snapshot(theme);
        Self {
            output,
            rendered,
            emulator,
            state,
            stop_tx: Some(stop_tx),
            input_tx: Some(input_tx),
            generation,
        }
    }

    fn append_output(&mut self, chunk: &str, theme: ThemeColors) {
        if chunk.is_empty() {
            return;
        }
        self.emulator.feed_bytes(chunk.as_bytes());
        self.output = self.emulator.render_plain_text();
        self.rendered = self.emulator.render_snapshot(theme);
    }

    fn refresh_render(&mut self, theme: ThemeColors) {
        self.output = self.emulator.render_plain_text();
        self.rendered = self.emulator.render_snapshot(theme);
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
    pub(crate) ports: Vec<PortPreviewEntry>,
    pub(crate) ports_loading: bool,
    pub(crate) port_error: Option<String>,
    port_poll_token: u64,
    pub(crate) auto_forward_enabled: bool,
    pub(crate) auto_forward_loaded: bool,
    pub(crate) auto_forward_busy: bool,
    pub(crate) auto_forward_error: Option<String>,
    pub(crate) selected_terminal_id: Option<TerminalId>,
    pub(crate) load_state: TerminalLoadState,
    terminal_streams: HashMap<TerminalId, TerminalStreamEntry>,
    terminal_stream_lru: VecDeque<TerminalId>,
    stream_generation_counter: u64,
    pub(crate) last_error: Option<String>,
    pub(crate) input: ComposerState,
    pub(crate) input_focus: FocusHandle,
    terminal_output_bounds: Option<Bounds<Pixels>>,
    terminal_cell_metrics: Option<TerminalCellMetrics>,
    terminal_mouse_selecting: bool,
    terminal_mouse_button: Option<MouseButton>,
    terminal_scroll_remainder: f32,
}

impl TerminalPanelState {
    pub(crate) fn new(colors: ThemeColors, input_focus: FocusHandle) -> Self {
        Self {
            colors,
            scope: TerminalScope::Workspace,
            context: TerminalContext::default(),
            terminals: Vec::new(),
            ports: Vec::new(),
            ports_loading: false,
            port_error: None,
            port_poll_token: 0,
            auto_forward_enabled: true,
            auto_forward_loaded: false,
            auto_forward_busy: false,
            auto_forward_error: None,
            selected_terminal_id: None,
            load_state: TerminalLoadState::Idle,
            terminal_streams: HashMap::new(),
            terminal_stream_lru: VecDeque::new(),
            stream_generation_counter: 0,
            last_error: None,
            input: ComposerState::new(),
            input_focus,
            terminal_output_bounds: None,
            terminal_cell_metrics: None,
            terminal_mouse_selecting: false,
            terminal_mouse_button: None,
            terminal_scroll_remainder: 0.0,
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
            self.ports.clear();
            self.ports_loading = false;
            self.port_error = None;
            self.auto_forward_enabled = true;
            self.auto_forward_loaded = false;
            self.auto_forward_busy = false;
            self.auto_forward_error = None;
        } else {
            self.reconcile_selection(cx);
        }
        if self.context.workspace_id.is_some() {
            self.start_port_polling(cx);
            if workspace_changed || !self.auto_forward_loaded {
                self.refresh_port_settings(cx);
            }
        } else {
            self.port_poll_token = self.port_poll_token.wrapping_add(1);
        }
        cx.notify();
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

    pub(crate) fn on_toggle_auto_forward(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_auto_forward(cx);
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

    pub(crate) fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.context.workspace_id else {
            self.ports.clear();
            self.ports_loading = false;
            self.port_error = None;
            cx.notify();
            return;
        };

        self.ports_loading = true;
        self.port_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let ports = client.list_workspace_ports(workspace_id).await?;
            Ok(ports)
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
                            Ok(ports) => {
                                view.ports = ports;
                                view.ports_loading = false;
                                view.port_error = None;
                            }
                            Err(err) => {
                                view.ports_loading = false;
                                view.port_error = Some(err.to_string());
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

    fn start_port_polling(&mut self, cx: &mut Context<Self>) {
        self.port_poll_token = self.port_poll_token.wrapping_add(1);
        let token = self.port_poll_token;
        self.refresh_ports(cx);

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            loop {
                tokio::time::sleep(Duration::from_millis(4000)).await;
                let keep = this
                    .update(cx, |view, cx| {
                        if view.port_poll_token != token || view.context.workspace_id.is_none() {
                            return false;
                        }
                        view.refresh_ports(cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn refresh_port_settings(&mut self, cx: &mut Context<Self>) {
        self.auto_forward_loaded = false;
        self.auto_forward_error = None;
        self.auto_forward_busy = false;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = ctx_client::Client::from_env()?;
            let settings = client.get_settings().await?;
            Ok(settings)
        });

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        match result {
                            Ok(settings) => {
                                view.auto_forward_enabled = settings
                                    .port_forwarding
                                    .map(|port| port.auto_forward)
                                    .unwrap_or(true);
                                view.auto_forward_loaded = true;
                                view.auto_forward_error = None;
                            }
                            Err(err) => {
                                view.auto_forward_error = Some(err.to_string());
                                view.auto_forward_loaded = false;
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

    fn toggle_auto_forward(&mut self, cx: &mut Context<Self>) {
        if !self.auto_forward_loaded || self.auto_forward_busy {
            return;
        }
        let next = !self.auto_forward_enabled;
        self.auto_forward_busy = true;
        self.auto_forward_error = None;
        cx.notify();

        let request = UpdateSettingsRequest {
            dictation: None,
            telemetry: None,
            title_generation: None,
            resource_governance: None,
            provider_guard: None,
            subagents: None,
            compaction: None,
            network: None,
            port_forwarding: Some(UpdatePortForwardingSettingsRequest { auto_forward: next }),
        };

        let task = Tokio::spawn_result(cx, async move {
            let client = ctx_client::Client::from_env()?;
            let settings = client.update_settings(&request).await?;
            Ok(settings)
        });

        cx.spawn(
            move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = task.await;
                    this.update(&mut cx, |view, cx| {
                        match result {
                            Ok(settings) => {
                                view.auto_forward_enabled = settings
                                    .port_forwarding
                                    .map(|port| port.auto_forward)
                                    .unwrap_or(next);
                                view.auto_forward_loaded = true;
                                view.auto_forward_error = None;
                            }
                            Err(err) => {
                                view.auto_forward_error = Some(err.to_string());
                            }
                        }
                        view.auto_forward_busy = false;
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    pub(crate) fn open_port_preview(&mut self, port_id: String, cx: &mut Context<Self>) {
        let url = ctx_client::Client::from_env()
            .and_then(|client| client.port_preview_url(&port_id))
            .map_err(|err| err.to_string());
        match url {
            Ok(url) => {
                cx.open_url(&url);
                self.port_error = None;
            }
            Err(err) => {
                self.port_error = Some(err);
            }
        }
        cx.notify();
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

    pub(crate) fn update_terminal_output_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        padding: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (cell_width, line_height) = terminal_cell_dimensions(window);
        let metrics = TerminalCellMetrics {
            cell_width,
            line_height,
            padding,
        };

        if self.terminal_output_bounds != Some(bounds) || self.terminal_cell_metrics != Some(metrics)
        {
            self.terminal_output_bounds = Some(bounds);
            self.terminal_cell_metrics = Some(metrics);
        }

        self.resize_active_terminal_to_bounds(bounds, metrics, cx);
    }

    fn resize_active_terminal_to_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        metrics: TerminalCellMetrics,
        cx: &mut Context<Self>,
    ) {
        let colors = self.colors;
        let Some(entry) = self.active_stream_entry_mut() else {
            return;
        };

        let padding = metrics.padding + metrics.padding;
        let border = px(2.0);
        let inner_width: f32 = (bounds.size.width - padding - border).into();
        let inner_height: f32 = (bounds.size.height - padding - border).into();
        let inner_width = inner_width.max(0.0);
        let inner_height = inner_height.max(0.0);
        if inner_width == 0.0 || inner_height == 0.0 {
            return;
        }

        let cell_width: f32 = metrics.cell_width.into();
        let line_height: f32 = metrics.line_height.into();
        if cell_width <= 0.0 || line_height <= 0.0 {
            return;
        }

        let columns = (inner_width / cell_width).floor() as usize;
        let screen_lines = (inner_height / line_height).floor() as usize;
        let columns = columns.clamp(1, u16::MAX as usize);
        let screen_lines = screen_lines.clamp(1, u16::MAX as usize);

        let (current_cols, current_lines) = {
            let grid = entry.emulator.term.grid();
            (grid.columns(), grid.screen_lines())
        };
        if current_cols == columns && current_lines == screen_lines {
            return;
        }

        entry
            .emulator
            .term
            .resize(TerminalGridSize::new(columns, screen_lines));
        entry.refresh_render(colors);
        if let Some(input_tx) = entry.input_tx.as_ref() {
            let payload =
                format!("{{\"type\":\"resize\",\"cols\":{columns},\"rows\":{screen_lines}}}");
            let _ = input_tx.send(payload);
        }
        cx.notify();
    }

    pub(crate) fn on_output_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.terminal_output_bounds;
        let metrics = self.terminal_cell_metrics;
        let colors = self.colors;
        let (mode, point) = {
            let Some(entry) = self.active_stream_entry() else {
                return;
            };
            let Some(point) = terminal_mouse_point(bounds, metrics, entry, event.position) else {
                return;
            };
            (entry.emulator.mode(), point)
        };

        if terminal_mouse_reporting_enabled(mode) {
            if let Some(payload) = mouse_report_payload(
                mode,
                MouseReportKind::Press(event.button),
                point,
                event.modifiers,
            ) {
                self.terminal_mouse_button = Some(event.button);
                self.send_input(payload, cx);
            }
            self.terminal_mouse_selecting = false;
            return;
        }

        if event.button != MouseButton::Left {
            return;
        }

        if let Some(entry) = self.active_stream_entry_mut() {
            entry.emulator.start_selection(point.grid_point);
            entry.refresh_render(colors);
        }
        self.terminal_mouse_selecting = true;
        cx.notify();
    }

    pub(crate) fn on_output_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.terminal_output_bounds;
        let metrics = self.terminal_cell_metrics;
        let pressed_button = event.pressed_button.or(self.terminal_mouse_button);
        let selecting = self.terminal_mouse_selecting;
        let colors = self.colors;
        let (mode, point) = {
            let Some(entry) = self.active_stream_entry() else {
                return;
            };
            let Some(point) = terminal_mouse_point(bounds, metrics, entry, event.position) else {
                return;
            };
            (entry.emulator.mode(), point)
        };

        if terminal_mouse_reporting_enabled(mode) {
            if !terminal_should_report_motion(mode, pressed_button) {
                return;
            }
            if let Some(payload) = mouse_report_payload(
                mode,
                MouseReportKind::Motion(pressed_button),
                point,
                event.modifiers,
            ) {
                self.send_input(payload, cx);
            }
            return;
        }

        if !selecting {
            return;
        }
        if let Some(entry) = self.active_stream_entry_mut() {
            entry.emulator.update_selection(point.grid_point);
            entry.refresh_render(colors);
        }
        cx.notify();
    }

    pub(crate) fn on_output_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_output_mouse(event.button, event.position, event.modifiers, cx);
    }

    pub(crate) fn on_output_mouse_up_out(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_output_mouse(event.button, event.position, event.modifiers, cx);
    }

    pub(crate) fn on_output_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.terminal_output_bounds;
        let metrics = self.terminal_cell_metrics;
        let colors = self.colors;
        let line_height = self
            .terminal_cell_metrics
            .map(|metrics| metrics.line_height)
            .unwrap_or(px(TERMINAL_LINE_HEIGHT));
        let (mode, point) = {
            let Some(entry) = self.active_stream_entry() else {
                return;
            };
            let point = if terminal_mouse_reporting_enabled(entry.emulator.mode()) {
                terminal_mouse_point(bounds, metrics, entry, event.position)
            } else {
                None
            };
            (entry.emulator.mode(), point)
        };

        window.prevent_default();

        if terminal_mouse_reporting_enabled(mode) {
            let Some(point) = point else {
                return;
            };
            let direction = if event.delta.pixel_delta(line_height).y >= px(0.0) {
                ScrollDirection::Down
            } else {
                ScrollDirection::Up
            };
            if let Some(payload) = mouse_report_payload(
                mode,
                MouseReportKind::Scroll(direction),
                point,
                event.modifiers,
            ) {
                self.send_input(payload, cx);
            }
            return;
        }

        if terminal_should_alternate_scroll(mode) {
            let delta = event.delta.pixel_delta(line_height);
            let direction = if delta.y >= px(0.0) {
                ScrollDirection::Down
            } else {
                ScrollDirection::Up
            };
            let line_height_value = f32::from(line_height);
            let mut steps = (f32::from(delta.y) / line_height_value)
                .abs()
                .ceil() as usize;
            if steps == 0 {
                steps = 1;
            }
            steps = steps.min(12);
            let sequence = match direction {
                ScrollDirection::Up => "\u{1b}[A",
                ScrollDirection::Down => "\u{1b}[B",
            };
            self.send_input(sequence.repeat(steps), cx);
            return;
        }

        let delta = event.delta.pixel_delta(line_height);
        self.terminal_scroll_remainder += f32::from(delta.y) / f32::from(line_height);
        let lines = self.terminal_scroll_remainder.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.terminal_scroll_remainder -= lines as f32;
        if let Some(entry) = self.active_stream_entry_mut() {
            entry.emulator.scroll_display(Scroll::Delta(-lines));
            entry.refresh_render(colors);
        }
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

    pub(crate) fn scope_ports(&self) -> Vec<&PortPreviewEntry> {
        if self.scope == TerminalScope::Workspace {
            return self.ports.iter().collect();
        }

        let session_id = self.context.session_id;
        let worktree_id = self.context.worktree_id;
        let task_id = self.context.task_id;

        self.ports
            .iter()
            .filter(|entry| {
                if let Some(session_id) = session_id {
                    if entry.session_id == Some(session_id) {
                        return true;
                    }
                }
                if let Some(worktree_id) = worktree_id {
                    if entry.worktree_id == Some(worktree_id) {
                        return true;
                    }
                }
                if let Some(task_id) = task_id {
                    if entry.task_id == Some(task_id) {
                        return true;
                    }
                }
                false
            })
            .collect()
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
                let mut entry = TerminalStreamEntry::new(
                    TerminalStreamState::Error(err.to_string()),
                    self.colors,
                );
                if let Some(emulator) = preserved_emulator {
                    entry.emulator = emulator;
                    entry.refresh_render(self.colors);
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
            self.colors,
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
                entry.append_output(&chunk, self.colors);
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

    pub(crate) fn active_stream_rendered(&self) -> Option<&TerminalRenderSnapshot> {
        self.active_stream_entry().map(|entry| &entry.rendered)
    }

    fn active_stream_input_tx(&self) -> Option<mpsc::UnboundedSender<String>> {
        self.active_stream_entry()
            .and_then(|entry| entry.input_tx.clone())
    }

    fn finish_output_mouse(
        &mut self,
        button: MouseButton,
        position: UiPoint<Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.terminal_output_bounds;
        let metrics = self.terminal_cell_metrics;
        let colors = self.colors;
        let (mode, point) = {
            let Some(entry) = self.active_stream_entry() else {
                return;
            };
            let point = terminal_mouse_point(bounds, metrics, entry, position);
            (entry.emulator.mode(), point)
        };

        if terminal_mouse_reporting_enabled(mode) {
            if let Some(point) = point {
                if let Some(payload) =
                    mouse_report_payload(mode, MouseReportKind::Release(button), point, modifiers)
                {
                    self.send_input(payload, cx);
                }
            }
            self.terminal_mouse_button = None;
            self.terminal_mouse_selecting = false;
            return;
        }

        if button != MouseButton::Left {
            return;
        }

        let selection_text = if let Some(entry) = self.active_stream_entry_mut() {
            if let Some(point) = point {
                entry.emulator.update_selection(point.grid_point);
            }
            entry.refresh_render(colors);
            entry.emulator.selection_to_string()
        } else {
            None
        };
        self.terminal_mouse_selecting = false;
        if let Some(text) = selection_text {
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
        cx.notify();
    }
}

fn terminal_mouse_point(
    bounds: Option<Bounds<Pixels>>,
    metrics: Option<TerminalCellMetrics>,
    entry: &TerminalStreamEntry,
    position: UiPoint<Pixels>,
) -> Option<TerminalMousePoint> {
    let bounds = bounds?;
    let metrics = metrics?;
    let origin = bounds.origin + gpui::point(metrics.padding, metrics.padding);
    let x = position.x - origin.x;
    let y = position.y - origin.y;
    if x < px(0.0) || y < px(0.0) {
        return None;
    }

    let columns = entry.emulator.term.grid().columns();
    let screen_lines = entry.emulator.term.grid().screen_lines();
    let col = ((x / metrics.cell_width).floor() as i32).clamp(0, columns as i32 - 1);
    let row = ((y / metrics.line_height).floor() as i32).clamp(0, screen_lines as i32 - 1);

    let display_offset = entry.emulator.term.grid().display_offset();
    let grid_point =
        viewport_to_point(display_offset, TermPoint::new(row as usize, Column(col as usize)));

    Some(TerminalMousePoint {
        grid_point,
        column: col as usize + 1,
        line: row as usize + 1,
    })
}

fn terminal_mouse_reporting_enabled(mode: TermMode) -> bool {
    mode.contains(TermMode::MOUSE_MODE)
}

fn terminal_should_report_motion(mode: TermMode, pressed_button: Option<MouseButton>) -> bool {
    mode.contains(TermMode::MOUSE_MOTION)
        || (mode.contains(TermMode::MOUSE_DRAG) && pressed_button.is_some())
}

fn terminal_should_alternate_scroll(mode: TermMode) -> bool {
    mode.contains(TermMode::ALTERNATE_SCROLL) && mode.contains(TermMode::ALT_SCREEN)
}

fn push_styled_char(
    text: &mut String,
    runs: &mut Vec<TextRun>,
    current_style: &mut Option<TerminalTextStyle>,
    current_len: &mut usize,
    style: &TerminalTextStyle,
    ch: char,
) {
    if current_style.as_ref() != Some(style) {
        if let Some(prev) = current_style.take() {
            if *current_len > 0 {
                runs.push(prev.to_run(*current_len));
            }
        }
        *current_len = 0;
        *current_style = Some(style.clone());
    }

    text.push(ch);
    *current_len += ch.len_utf8();
}

fn rgba_to_rgb(color: Rgba) -> Rgb {
    Rgb {
        r: (color.r * 255.0).round().clamp(0.0, 255.0) as u8,
        g: (color.g * 255.0).round().clamp(0.0, 255.0) as u8,
        b: (color.b * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

fn rgb_to_hsla(color: Rgb) -> Hsla {
    let rgba = Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: 1.0,
    };
    Hsla::from(rgba)
}

fn dim_rgb(color: Rgb) -> Rgb {
    Rgb {
        r: (f32::from(color.r) * DIM_FACTOR).round().clamp(0.0, 255.0) as u8,
        g: (f32::from(color.g) * DIM_FACTOR).round().clamp(0.0, 255.0) as u8,
        b: (f32::from(color.b) * DIM_FACTOR).round().clamp(0.0, 255.0) as u8,
    }
}

fn resolve_color(
    color: Color,
    palette: &Colors,
    theme: ThemeColors,
    is_fg: bool,
    bold: bool,
    dim: bool,
) -> Rgb {
    match color {
        Color::Named(named) => {
            let mut name = named;
            let mut apply_dim = dim;

            if dim {
                if let Some(dimmed) = dim_named_color(name) {
                    name = dimmed;
                    apply_dim = false;
                }
            } else if is_fg && bold {
                name = name.to_bright();
            }

            let mut rgb = palette[name].unwrap_or_else(|| fallback_named_color(name, theme));
            if apply_dim && is_fg {
                rgb = dim_rgb(rgb);
            }
            rgb
        }
        Color::Indexed(index) => {
            let mut idx = index;
            if is_fg && bold && idx < 8 {
                idx += 8;
            }
            let mut rgb = palette[idx as usize].unwrap_or_else(|| xterm_color(idx, theme));
            if dim && is_fg {
                rgb = dim_rgb(rgb);
            }
            rgb
        }
        Color::Spec(rgb) => {
            if dim && is_fg {
                dim_rgb(rgb)
            } else {
                rgb
            }
        }
    }
}

fn resolve_named_color(name: NamedColor, palette: &Colors, theme: ThemeColors) -> Rgb {
    palette[name].unwrap_or_else(|| fallback_named_color(name, theme))
}

fn fallback_named_color(color: NamedColor, theme: ThemeColors) -> Rgb {
    match color {
        NamedColor::Foreground | NamedColor::BrightForeground => rgba_to_rgb(theme.text),
        NamedColor::Background => rgba_to_rgb(theme.panel),
        NamedColor::Cursor => rgba_to_rgb(theme.accent),
        NamedColor::DimForeground => rgba_to_rgb(theme.muted),
        NamedColor::Black => DEFAULT_ANSI_COLORS[0],
        NamedColor::Red => DEFAULT_ANSI_COLORS[1],
        NamedColor::Green => DEFAULT_ANSI_COLORS[2],
        NamedColor::Yellow => DEFAULT_ANSI_COLORS[3],
        NamedColor::Blue => DEFAULT_ANSI_COLORS[4],
        NamedColor::Magenta => DEFAULT_ANSI_COLORS[5],
        NamedColor::Cyan => DEFAULT_ANSI_COLORS[6],
        NamedColor::White => DEFAULT_ANSI_COLORS[7],
        NamedColor::BrightBlack => DEFAULT_ANSI_COLORS[8],
        NamedColor::BrightRed => DEFAULT_ANSI_COLORS[9],
        NamedColor::BrightGreen => DEFAULT_ANSI_COLORS[10],
        NamedColor::BrightYellow => DEFAULT_ANSI_COLORS[11],
        NamedColor::BrightBlue => DEFAULT_ANSI_COLORS[12],
        NamedColor::BrightMagenta => DEFAULT_ANSI_COLORS[13],
        NamedColor::BrightCyan => DEFAULT_ANSI_COLORS[14],
        NamedColor::BrightWhite => DEFAULT_ANSI_COLORS[15],
        NamedColor::DimBlack => dim_rgb(DEFAULT_ANSI_COLORS[0]),
        NamedColor::DimRed => dim_rgb(DEFAULT_ANSI_COLORS[1]),
        NamedColor::DimGreen => dim_rgb(DEFAULT_ANSI_COLORS[2]),
        NamedColor::DimYellow => dim_rgb(DEFAULT_ANSI_COLORS[3]),
        NamedColor::DimBlue => dim_rgb(DEFAULT_ANSI_COLORS[4]),
        NamedColor::DimMagenta => dim_rgb(DEFAULT_ANSI_COLORS[5]),
        NamedColor::DimCyan => dim_rgb(DEFAULT_ANSI_COLORS[6]),
        NamedColor::DimWhite => dim_rgb(DEFAULT_ANSI_COLORS[7]),
    }
}

fn xterm_color(index: u8, theme: ThemeColors) -> Rgb {
    match index {
        0..=15 => fallback_named_color(
            match index {
                0 => NamedColor::Black,
                1 => NamedColor::Red,
                2 => NamedColor::Green,
                3 => NamedColor::Yellow,
                4 => NamedColor::Blue,
                5 => NamedColor::Magenta,
                6 => NamedColor::Cyan,
                7 => NamedColor::White,
                8 => NamedColor::BrightBlack,
                9 => NamedColor::BrightRed,
                10 => NamedColor::BrightGreen,
                11 => NamedColor::BrightYellow,
                12 => NamedColor::BrightBlue,
                13 => NamedColor::BrightMagenta,
                14 => NamedColor::BrightCyan,
                _ => NamedColor::BrightWhite,
            },
            theme,
        ),
        16..=231 => {
            let idx = index - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            Rgb {
                r: color_cube_value(r),
                g: color_cube_value(g),
                b: color_cube_value(b),
            }
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            Rgb {
                r: value,
                g: value,
                b: value,
            }
        }
    }
}

fn color_cube_value(index: u8) -> u8 {
    match index {
        0 => 0,
        _ => 55 + (index * 40),
    }
}

fn dim_named_color(color: NamedColor) -> Option<NamedColor> {
    match color {
        NamedColor::Foreground => Some(NamedColor::DimForeground),
        NamedColor::Black | NamedColor::BrightBlack => Some(NamedColor::DimBlack),
        NamedColor::Red | NamedColor::BrightRed => Some(NamedColor::DimRed),
        NamedColor::Green | NamedColor::BrightGreen => Some(NamedColor::DimGreen),
        NamedColor::Yellow | NamedColor::BrightYellow => Some(NamedColor::DimYellow),
        NamedColor::Blue | NamedColor::BrightBlue => Some(NamedColor::DimBlue),
        NamedColor::Magenta | NamedColor::BrightMagenta => Some(NamedColor::DimMagenta),
        NamedColor::Cyan | NamedColor::BrightCyan => Some(NamedColor::DimCyan),
        NamedColor::White | NamedColor::BrightWhite => Some(NamedColor::DimWhite),
        _ => None,
    }
}

fn underline_style_for_cell(
    cell: &alacritty_terminal::term::cell::Cell,
    color: Rgb,
) -> UnderlineStyle {
    let mut style = UnderlineStyle {
        thickness: px(1.0),
        color: Some(rgb_to_hsla(color)),
        wavy: false,
    };
    if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
        style.thickness = px(2.0);
    }
    if cell.flags.contains(Flags::UNDERCURL) {
        style.wavy = true;
    }
    style
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

// Runs on a Tokio runtime (spawned via Tokio::spawn_result).
#[allow(clippy::disallowed_methods)]
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

fn terminal_cell_dimensions(window: &Window) -> (Pixels, Pixels) {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font(MONO_FONT_FAMILY));
    let font_size = px(TERMINAL_FONT_SIZE);
    let cell_width = text_system.em_advance(font_id, font_size).unwrap_or(px(8.0));
    let line_height = text_system.ascent(font_id, font_size)
        + text_system.descent(font_id, font_size);
    let line_height = if f32::from(line_height) > 0.0 {
        line_height
    } else {
        px(TERMINAL_LINE_HEIGHT)
    };
    (cell_width, line_height)
}

pub(crate) fn terminal_line_height(window: &Window) -> Pixels {
    terminal_cell_dimensions(window).1
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalCellMetrics {
    cell_width: Pixels,
    line_height: Pixels,
    padding: Pixels,
}

#[derive(Clone, Copy, Debug)]
struct TerminalMousePoint {
    grid_point: TermPoint,
    column: usize,
    line: usize,
}

#[derive(Clone, Copy, Debug)]
enum ScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug)]
enum MouseReportKind {
    Press(MouseButton),
    Release(MouseButton),
    Motion(Option<MouseButton>),
    Scroll(ScrollDirection),
}

fn mouse_report_payload(
    mode: TermMode,
    kind: MouseReportKind,
    point: TerminalMousePoint,
    modifiers: gpui::Modifiers,
) -> Option<String> {
    let modifier_bits = (modifiers.shift as u8) * 4
        + (modifiers.alt as u8) * 8
        + (modifiers.control as u8) * 16;

    let (button_code, release) = match kind {
        MouseReportKind::Press(button) => (mouse_button_code(button)?, false),
        MouseReportKind::Release(button) => (mouse_button_code(button)?, true),
        MouseReportKind::Motion(button) => {
            let base = button.and_then(mouse_button_code).unwrap_or(3);
            (base + 32, false)
        }
        MouseReportKind::Scroll(direction) => (
            match direction {
                ScrollDirection::Up => 64,
                ScrollDirection::Down => 65,
            },
            false,
        ),
    };

    let button = button_code + modifier_bits;
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if release { 'm' } else { 'M' };
        return Some(format!(
            "\u{1b}[<{button};{};{}{suffix}",
            point.column, point.line
        ));
    }

    let encoded_button = if release { 3 } else { button };
    Some(format!(
        "\u{1b}[M{}{}{}",
        encode_mouse_byte(encoded_button + 32),
        encode_mouse_byte(point.column as u8 + 32),
        encode_mouse_byte(point.line as u8 + 32),
    ))
}

fn mouse_button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

fn encode_mouse_byte(value: u8) -> char {
    char::from_u32(value as u32).unwrap_or('\u{0}')
}
