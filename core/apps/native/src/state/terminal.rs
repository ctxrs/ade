use gpui::{ClickEvent, Context, Window};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, TaskId, TerminalId, TrackId, WorktreeId, WorkspaceId};
use ctx_core::models::TerminalSession;

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
    Stubbed,
    Error(String),
}

impl TerminalStreamState {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            TerminalStreamState::Idle => "Idle",
            TerminalStreamState::Stubbed => "Stubbed",
            TerminalStreamState::Error(_) => "Error",
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
    pub(crate) stream_url: Option<String>,
    pub(crate) last_error: Option<String>,
}

impl TerminalPanelState {
    pub(crate) fn new(colors: ThemeColors) -> Self {
        Self {
            colors,
            scope: TerminalScope::Workspace,
            context: TerminalContext::default(),
            terminals: Vec::new(),
            selected_terminal_id: None,
            load_state: TerminalLoadState::Idle,
            stream_state: TerminalStreamState::Idle,
            stream_terminal_id: None,
            stream_url: None,
            last_error: None,
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
            self.clear_stream();
            self.load_terminals(cx);
        } else {
            self.reconcile_selection();
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
            self.reconcile_selection();
            cx.notify();
        }
    }

    pub(crate) fn select_terminal(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        self.selected_terminal_id = Some(terminal_id);
        self.prepare_stream_stub(terminal_id);
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

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.context.workspace_id != Some(workspace_id) {
                    return;
                }
                match result {
                    Ok(terminals) => {
                        view.terminals = terminals;
                        view.load_state = TerminalLoadState::Loaded;
                        view.last_error = None;
                        view.reconcile_selection();
                    }
                    Err(err) => {
                        view.load_state =
                            TerminalLoadState::Error("Unable to load terminals.".to_string());
                        view.last_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
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

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.context.workspace_id != Some(workspace_id) {
                    return;
                }
                match result {
                    Ok(terminal) => {
                        view.terminals.push(terminal.clone());
                        view.selected_terminal_id = Some(terminal.id);
                        view.prepare_stream_stub(terminal.id);
                        view.last_error = None;
                    }
                    Err(err) => {
                        view.last_error = Some(err.to_string());
                    }
                }
                view.reconcile_selection();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn delete_terminal(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            client.delete_terminal(terminal_id).await?;
            Ok(terminal_id)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(terminal_id) => {
                        view.terminals.retain(|terminal| terminal.id != terminal_id);
                        if view.selected_terminal_id == Some(terminal_id) {
                            view.selected_terminal_id = None;
                        }
                        if view.stream_terminal_id == Some(terminal_id) {
                            view.clear_stream();
                        }
                        view.reconcile_selection();
                        view.last_error = None;
                    }
                    Err(err) => {
                        view.last_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
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

    fn reconcile_selection(&mut self) {
        let scope_terminals = self.scope_terminals();
        if let Some(selected) = self.selected_terminal_id {
            if scope_terminals
                .iter()
                .any(|terminal| terminal.id == selected)
            {
                if self.stream_terminal_id != Some(selected) {
                    self.prepare_stream_stub(selected);
                }
                return;
            }
        }
        let previous_selection = self.selected_terminal_id;
        self.selected_terminal_id = scope_terminals.first().map(|terminal| terminal.id);
        if self.selected_terminal_id != previous_selection {
            if let Some(terminal_id) = self.selected_terminal_id {
                self.prepare_stream_stub(terminal_id);
            } else {
                self.clear_stream();
            }
        }
    }

    fn prepare_stream_stub(&mut self, terminal_id: TerminalId) {
        self.stream_terminal_id = Some(terminal_id);
        let stream_url = ctx_client::resolve_daemon_config()
            .and_then(ctx_client::Client::new)
            .and_then(|client| client.terminal_stream_url(terminal_id));
        match stream_url {
            Ok(url) => {
                self.stream_state = TerminalStreamState::Stubbed;
                self.stream_url = Some(url);
            }
            Err(err) => {
                self.stream_state = TerminalStreamState::Error(err.to_string());
                self.stream_url = None;
            }
        }
    }

    fn clear_stream(&mut self) {
        self.stream_terminal_id = None;
        self.stream_state = TerminalStreamState::Idle;
        self.stream_url = None;
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
