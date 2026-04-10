mod io;
#[cfg(test)]
mod tests;
mod translate;

use crate::app_server::{
    AppServerClient, AppServerInbound, ModelListResponse, ThreadLoadedListResponse,
    ThreadReadResponse, ThreadStartLikeResponse, ThreadStatus, TurnStartResponse,
};
use crate::builtins::{
    build_app_server_config_overrides, build_current_model_id, build_model_infos,
    build_session_command_infos, command_names, merge_config_overrides,
    normalize_ctx_system_prompt_append, resolve_codex_home, split_model_and_effort,
};
use crate::protocol::{
    CrpChannel, CrpCommand, CrpCommandInfo, CrpEvent, CrpModelInfo, CrpSessionConfig, CrpTurnStatus,
};
use crate::RuntimeOptions;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, warn};

use self::io::{dispatch_event, read_commands, run_writer, CrpEventRouter, CrpWriter};
use self::translate::{
    canonical_context_window_from_thread_usage, emit_turn_request_error,
    emit_unsupported_server_request_notice, translate_notification,
};

const DATA_PLANE_BUFFER_CAPACITY: usize = 256;

enum RuntimeCommand {
    Parsed(Box<CrpCommand>),
    ParseError { message: String },
}

#[derive(Default)]
struct ReasoningSummaryState {
    text: String,
}

struct TurnRuntimeState {
    message_id: Option<String>,
    emitted_final: bool,
    reasoning_summaries: HashMap<(String, i64), ReasoningSummaryState>,
    reasoning_text_seen: HashSet<String>,
}

impl TurnRuntimeState {
    fn new() -> Self {
        Self {
            message_id: None,
            emitted_final: false,
            reasoning_summaries: HashMap::new(),
            reasoning_text_seen: HashSet::new(),
        }
    }
}

struct TurnTracker {
    session_id: String,
    turns: HashMap<String, TurnRuntimeState>,
}

impl TurnTracker {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            turns: HashMap::new(),
        }
    }

    fn ensure_turn(&mut self, turn_id: &str) -> &mut TurnRuntimeState {
        self.turns
            .entry(turn_id.to_string())
            .or_insert_with(TurnRuntimeState::new)
    }
}

struct TurnAliasState {
    app_to_crp: HashMap<String, String>,
    crp_to_app: HashMap<String, String>,
    pending_compact_turns: VecDeque<String>,
    active_app_turn_id: Option<String>,
    active_crp_turn_id: Option<String>,
    latest_token_usage: Option<crate::app_server::ThreadTokenUsage>,
    token_usage_by_app_turn: HashMap<String, crate::app_server::ThreadTokenUsage>,
}

impl TurnAliasState {
    fn new() -> Self {
        Self {
            app_to_crp: HashMap::new(),
            crp_to_app: HashMap::new(),
            pending_compact_turns: VecDeque::new(),
            active_app_turn_id: None,
            active_crp_turn_id: None,
            latest_token_usage: None,
            token_usage_by_app_turn: HashMap::new(),
        }
    }

    fn bind_turn_alias(
        &mut self,
        app_turn_id: String,
        requested_crp_turn_id: Option<String>,
    ) -> String {
        let crp_turn_id = requested_crp_turn_id.unwrap_or_else(|| app_turn_id.clone());
        self.app_to_crp
            .insert(app_turn_id.clone(), crp_turn_id.clone());
        self.crp_to_app
            .insert(crp_turn_id.clone(), app_turn_id.clone());
        self.active_app_turn_id = Some(app_turn_id);
        self.active_crp_turn_id = Some(crp_turn_id.clone());
        crp_turn_id
    }

    fn app_turn_id_for_crp(&self, crp_turn_id: Option<&str>) -> Option<String> {
        crp_turn_id.and_then(|id| self.crp_to_app.get(id).cloned())
    }

    fn ensure_crp_turn_id(&mut self, app_turn_id: &str) -> String {
        if let Some(existing) = self.app_to_crp.get(app_turn_id) {
            return existing.clone();
        }
        if let Some(pending) = self.pending_compact_turns.pop_front() {
            return self.bind_turn_alias(app_turn_id.to_string(), Some(pending));
        }
        self.bind_turn_alias(app_turn_id.to_string(), None)
    }

    fn note_terminal_turn(&mut self, app_turn_id: &str) {
        if self.active_app_turn_id.as_deref() == Some(app_turn_id) {
            self.active_app_turn_id = None;
        }
        if let Some(crp_turn_id) = self.app_to_crp.get(app_turn_id).cloned() {
            if self.active_crp_turn_id.as_deref() == Some(crp_turn_id.as_str()) {
                self.active_crp_turn_id = None;
            }
        }
    }

    fn note_token_usage(
        &mut self,
        app_turn_id: &str,
        token_usage: crate::app_server::ThreadTokenUsage,
    ) {
        self.latest_token_usage = Some(token_usage.clone());
        self.token_usage_by_app_turn
            .insert(app_turn_id.to_string(), token_usage);
    }

    fn take_context_window_for_app_turn(&mut self, app_turn_id: &str) -> Option<Value> {
        let token_usage = self
            .token_usage_by_app_turn
            .remove(app_turn_id)
            .or_else(|| self.latest_token_usage.clone())?;
        canonical_context_window_from_thread_usage(&token_usage)
    }
}

struct AppServerSessionState {
    tracker: TurnTracker,
    client: AppServerClient,
    thread_id: String,
    default_cwd: PathBuf,
    default_model: String,
    default_effort: Option<String>,
    opened_commands: Vec<CrpCommandInfo>,
    opened_slash_commands: Vec<String>,
    turn_aliases: TurnAliasState,
}

enum RuntimeInput {
    Command(Box<RuntimeCommand>),
    AppServer(AppServerInbound),
}

pub async fn run(options: RuntimeOptions) -> Result<()> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(read_commands(cmd_tx));

    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (data_tx, data_rx) = mpsc::channel(DATA_PLANE_BUFFER_CAPACITY);
    let router = CrpEventRouter::new(control_tx, data_tx);

    let writer = CrpWriter::new();
    tokio::spawn(async move {
        if let Err(err) = run_writer(writer, control_rx, data_rx).await {
            error!(?err, "crp writer task failed");
        }
    });

    let mut session: Option<AppServerSessionState> = None;

    loop {
        match next_runtime_input(&mut session, &mut cmd_rx).await {
            Some(RuntimeInput::Command(command)) => {
                handle_command(*command, &mut session, &router, &options).await?;
            }
            Some(RuntimeInput::AppServer(event)) => {
                if let Some(session_state) = session.as_mut() {
                    handle_app_server_event(session_state, event, &router).await;
                }
            }
            None => break,
        }
    }

    if let Some(session_state) = session.as_mut() {
        session_state.client.shutdown().await;
    }

    Ok(())
}

async fn next_runtime_input(
    session: &mut Option<AppServerSessionState>,
    cmd_rx: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
) -> Option<RuntimeInput> {
    let Some(session_state) = session.as_mut() else {
        return cmd_rx
            .recv()
            .await
            .map(|command| RuntimeInput::Command(Box::new(command)));
    };
    tokio::select! {
        Some(cmd) = cmd_rx.recv() => Some(RuntimeInput::Command(Box::new(cmd))),
        maybe_event = session_state.client.next_inbound() => maybe_event.map(RuntimeInput::AppServer),
        else => None,
    }
}

async fn handle_command(
    command: RuntimeCommand,
    session: &mut Option<AppServerSessionState>,
    router: &CrpEventRouter,
    options: &RuntimeOptions,
) -> Result<()> {
    match command {
        RuntimeCommand::ParseError { message } => {
            warn!(%message, "failed to parse CRP command");
        }
        RuntimeCommand::Parsed(command) => match *command {
            CrpCommand::ToolResult {
                session_id,
                turn_id,
                tool_call_id,
                status,
                output,
                error,
            } => {
                let _ = (session_id, turn_id, tool_call_id, status, output, error);
                warn!("tool.result ignored: app-server-backed runtime owns tool execution");
            }
            command => {
                handle_parsed_command(command, session, router, options).await?;
            }
        },
    }
    Ok(())
}

async fn handle_parsed_command(
    command: CrpCommand,
    session: &mut Option<AppServerSessionState>,
    router: &CrpEventRouter,
    options: &RuntimeOptions,
) -> Result<()> {
    match command {
        CrpCommand::SessionOpen {
            session_id,
            provider_session_id,
            config,
        } => {
            if session.is_some() {
                warn!("session.open ignored: session already active");
                return Ok(());
            }

            let config = config.unwrap_or_default();
            let provider_session_id = provider_session_id.or_else(|| {
                std::env::var("CTX_PROVIDER_SESSION_REF")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });

            let mut state = open_session(config, provider_session_id, options).await?;
            let provider_session_id = state.thread_id.clone();
            let session_id = session_id.unwrap_or_else(|| provider_session_id.clone());
            state.tracker.session_id = session_id.clone();

            let _ = router.send_control(CrpEvent::SessionOpened {
                session_id,
                provider_session_id: Some(provider_session_id),
                commands: Some(state.opened_commands.clone()),
                slash_commands: Some(state.opened_slash_commands.clone()),
            });

            *session = Some(state);
        }
        CrpCommand::SessionPrompt {
            session_id,
            turn_id,
            prompt,
            items,
            model,
            reasoning_effort,
            cwd,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.prompt ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.prompt ignored: session_id mismatch");
                    return Ok(());
                }
            }

            let items = match (items, prompt) {
                (Some(items), _) => items,
                (None, Some(prompt)) => vec![json!({
                    "type": "text",
                    "text": prompt,
                })],
                (None, None) => {
                    warn!("session.prompt ignored: missing prompt or items");
                    return Ok(());
                }
            };

            let cwd = cwd.unwrap_or_else(|| state.default_cwd.clone());
            let requested_model = model.unwrap_or_else(|| {
                if let Some(effort) = state.default_effort.as_deref() {
                    format!("{}/{}", state.default_model, effort)
                } else {
                    state.default_model.clone()
                }
            });
            let (model, effort_override) = split_model_and_effort(&requested_model);
            let effort = effort_override.or(reasoning_effort);
            let requested_turn_id = turn_id.clone();

            match state
                .client
                .request::<TurnStartResponse>(
                    "turn/start",
                    json!({
                        "threadId": state.thread_id,
                        "input": items,
                        "cwd": cwd.to_string_lossy().to_string(),
                        "model": model,
                        "effort": effort,
                    }),
                )
                .await
            {
                Ok(response) => {
                    let _ = state
                        .turn_aliases
                        .bind_turn_alias(response.turn.id.clone(), requested_turn_id);
                }
                Err(err) => emit_turn_request_error(
                    router,
                    &state.tracker.session_id,
                    turn_id,
                    "turn_start_failed",
                    err.to_string(),
                ),
            }
        }
        CrpCommand::SessionCompact {
            session_id,
            turn_id,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.compact ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.compact ignored: session_id mismatch");
                    return Ok(());
                }
            }
            if let Some(turn_id) = turn_id.clone() {
                state.turn_aliases.pending_compact_turns.push_back(turn_id);
            }
            if let Err(err) = state
                .client
                .request::<Value>(
                    "thread/compact/start",
                    json!({ "threadId": state.thread_id }),
                )
                .await
            {
                emit_turn_request_error(
                    router,
                    &state.tracker.session_id,
                    turn_id,
                    "thread_compact_start_failed",
                    err.to_string(),
                );
            }
        }
        CrpCommand::SessionUndo {
            session_id,
            turn_id,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.undo ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.undo ignored: session_id mismatch");
                    return Ok(());
                }
            }
            match state
                .client
                .request::<Value>(
                    "thread/rollback",
                    json!({ "threadId": state.thread_id, "numTurns": 1 }),
                )
                .await
            {
                Ok(_) => {
                    if let Some(turn_id) = turn_id {
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::TurnCompleted {
                                session_id: state.tracker.session_id.clone(),
                                turn_id,
                                status: CrpTurnStatus::Success,
                                context_window: None,
                                error: None,
                            },
                        );
                    }
                }
                Err(err) => emit_turn_request_error(
                    router,
                    &state.tracker.session_id,
                    turn_id,
                    "thread_rollback_failed",
                    err.to_string(),
                ),
            }
        }
        CrpCommand::SessionReview {
            session_id,
            turn_id,
            instructions,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.review ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.review ignored: session_id mismatch");
                    return Ok(());
                }
            }

            let target = instructions
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(|instructions| json!({"type": "custom", "instructions": instructions}))
                .unwrap_or_else(|| json!({"type": "uncommittedChanges"}));
            let requested_turn_id = turn_id.clone();
            match state
                .client
                .request::<serde_json::Value>(
                    "review/start",
                    json!({
                        "threadId": state.thread_id,
                        "target": target,
                    }),
                )
                .await
            {
                Ok(value) => {
                    let turn_id_from_response = value
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if !turn_id_from_response.is_empty() {
                        let _ = state
                            .turn_aliases
                            .bind_turn_alias(turn_id_from_response, requested_turn_id);
                    }
                }
                Err(err) => emit_turn_request_error(
                    router,
                    &state.tracker.session_id,
                    turn_id,
                    "review_start_failed",
                    err.to_string(),
                ),
            }
        }
        CrpCommand::SessionSetModel {
            session_id,
            model_id,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.set_model ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.set_model ignored: session_id mismatch");
                    return Ok(());
                }
            }
            let Some(model_id) = model_id
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                warn!("session.set_model ignored: missing model_id");
                return Ok(());
            };

            let response = state
                .client
                .request::<ModelListResponse>("model/list", json!({ "includeHidden": false }))
                .await;
            match response {
                Ok(response) => {
                    let (model, effort) = split_model_and_effort(&model_id);
                    let exists = response.data.iter().any(|candidate| candidate.id == model);
                    if exists {
                        state.default_model = model;
                        state.default_effort = effort;
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::SessionNotice {
                                session_id: state.tracker.session_id.clone(),
                                turn_id: None,
                                code: "session_model_updated".to_string(),
                                severity: Some("info".to_string()),
                                message: Some(format!("session model updated to {model_id}")),
                                details: Some(json!({ "model_id": model_id })),
                                transient: Some(false),
                            },
                        );
                    } else {
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::SessionNotice {
                                session_id: state.tracker.session_id.clone(),
                                turn_id: None,
                                code: "session_model_update_failed".to_string(),
                                severity: Some("error".to_string()),
                                message: Some(format!(
                                    "failed to update session model to {model_id}: model not found"
                                )),
                                details: Some(json!({ "model_id": model_id })),
                                transient: Some(false),
                            },
                        );
                    }
                }
                Err(err) => dispatch_event(
                    router,
                    CrpChannel::Control,
                    CrpEvent::SessionNotice {
                        session_id: state.tracker.session_id.clone(),
                        turn_id: None,
                        code: "session_model_update_failed".to_string(),
                        severity: Some("error".to_string()),
                        message: Some(format!(
                            "failed to update session model to {model_id}: {err}"
                        )),
                        details: Some(json!({ "model_id": model_id })),
                        transient: Some(false),
                    },
                ),
            }
        }
        CrpCommand::SessionStatus { session_id } => {
            let Some(state) = session.as_mut() else {
                warn!("session.status ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.status ignored: session_id mismatch");
                    return Ok(());
                }
            }

            match query_session_status(state).await {
                Ok(details) => dispatch_event(
                    router,
                    CrpChannel::Control,
                    CrpEvent::SessionNotice {
                        session_id: state.tracker.session_id.clone(),
                        turn_id: None,
                        code: "session_status".to_string(),
                        severity: Some("info".to_string()),
                        message: Some(if details["quiescent"] == json!(true) {
                            "session is quiescent".to_string()
                        } else {
                            "session is busy".to_string()
                        }),
                        details: Some(details),
                        transient: Some(false),
                    },
                ),
                Err(err) => dispatch_event(
                    router,
                    CrpChannel::Control,
                    CrpEvent::SessionNotice {
                        session_id: state.tracker.session_id.clone(),
                        turn_id: None,
                        code: "session_status_failed".to_string(),
                        severity: Some("error".to_string()),
                        message: Some(err.to_string()),
                        details: None,
                        transient: Some(false),
                    },
                ),
            }
        }
        CrpCommand::ModelsList { config } => {
            let models = probe_models(config.unwrap_or_default(), options).await?;
            let _ = router.send_control(CrpEvent::ModelsList {
                models: models.models,
                current_model_id: models.current_model_id,
                catalog_source: models.catalog_source,
            });
        }
        CrpCommand::SessionCancel {
            session_id,
            turn_id,
        } => {
            let Some(state) = session.as_mut() else {
                warn!("session.cancel ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref() {
                if expected != state.tracker.session_id {
                    warn!(%expected, "session.cancel ignored: session_id mismatch");
                    return Ok(());
                }
            }
            let app_turn_id = state
                .turn_aliases
                .app_turn_id_for_crp(turn_id.as_deref())
                .or_else(|| state.turn_aliases.active_app_turn_id.clone());
            let Some(app_turn_id) = app_turn_id else {
                warn!("session.cancel ignored: no active turn to interrupt");
                return Ok(());
            };
            if let Err(err) = state
                .client
                .request::<Value>(
                    "turn/interrupt",
                    json!({ "threadId": state.thread_id, "turnId": app_turn_id }),
                )
                .await
            {
                dispatch_event(
                    router,
                    CrpChannel::Control,
                    CrpEvent::SessionNotice {
                        session_id: state.tracker.session_id.clone(),
                        turn_id,
                        code: "turn_interrupt_failed".to_string(),
                        severity: Some("error".to_string()),
                        message: Some(err.to_string()),
                        details: None,
                        transient: Some(false),
                    },
                );
            }
        }
        CrpCommand::ToolResult { .. } => {
            warn!("tool.result ignored: app-server-backed runtime owns tool execution");
        }
    }

    Ok(())
}

struct ModelsProbe {
    models: Vec<CrpModelInfo>,
    current_model_id: Option<String>,
    catalog_source: Option<String>,
}

async fn open_session(
    session_config: CrpSessionConfig,
    provider_session_id: Option<String>,
    options: &RuntimeOptions,
) -> Result<AppServerSessionState> {
    let workdir = session_config
        .spawn_cwd
        .clone()
        .or_else(|| session_config.cwd.clone())
        .unwrap_or(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut client = AppServerClient::start(&workdir).await?;
    let developer_instructions =
        normalize_ctx_system_prompt_append(std::env::var("CTX_SYSTEM_PROMPT_APPEND").ok());
    let config_overrides = merge_config_overrides(
        options.config_overrides.clone(),
        build_app_server_config_overrides(&session_config),
    );
    let effort = session_effort(&session_config);
    let (thread_response, thread_id) = if let Some(provider_session_id) = provider_session_id {
        match client
            .request::<ThreadStartLikeResponse>(
                "thread/resume",
                json!({
                    "threadId": provider_session_id,
                    "cwd": session_config.cwd.as_ref().map(|path| path.to_string_lossy().to_string()),
                    "model": session_config.model.as_ref().map(|model| split_model_and_effort(model).0),
                    "effort": effort,
                    "modelProvider": session_config.model_provider,
                    "approvalPolicy": session_config.approval_policy,
                    "sandbox": session_config.sandbox_mode,
                    "config": config_overrides,
                    "developerInstructions": developer_instructions,
                    "personality": session_config.personality,
                    "persistExtendedHistory": false,
                }),
            )
            .await
        {
            Ok(response) => {
                let thread_id = response.thread.id.clone();
                (response, thread_id)
            }
            Err(err) => {
                warn!(provider_session_id = %provider_session_id, ?err, "thread/resume failed; starting a new thread");
                let response =
                    start_thread(&mut client, &session_config, developer_instructions, options)
                        .await?;
                let thread_id = response.thread.id.clone();
                (response, thread_id)
            }
        }
    } else {
        let response = start_thread(
            &mut client,
            &session_config,
            developer_instructions,
            options,
        )
        .await?;
        let thread_id = response.thread.id.clone();
        (response, thread_id)
    };

    let codex_home = resolve_codex_home();
    let opened_commands = build_session_command_infos(&codex_home);
    let opened_slash_commands = command_names(&opened_commands);

    Ok(AppServerSessionState {
        tracker: TurnTracker::new(String::new()),
        client,
        thread_id,
        default_cwd: PathBuf::from(thread_response.cwd),
        default_model: thread_response.model,
        default_effort: thread_response.reasoning_effort,
        opened_commands,
        opened_slash_commands,
        turn_aliases: TurnAliasState::new(),
    })
}

async fn start_thread(
    client: &mut AppServerClient,
    session_config: &CrpSessionConfig,
    developer_instructions: Option<String>,
    options: &RuntimeOptions,
) -> Result<ThreadStartLikeResponse> {
    let config_overrides = merge_config_overrides(
        options.config_overrides.clone(),
        build_app_server_config_overrides(session_config),
    );
    client
        .request::<ThreadStartLikeResponse>(
            "thread/start",
            json!({
                "model": session_config.model.as_ref().map(|model| split_model_and_effort(model).0),
                "effort": session_effort(session_config),
                "modelProvider": session_config.model_provider,
                "cwd": session_config.cwd.as_ref().map(|path| path.to_string_lossy().to_string()),
                "approvalPolicy": session_config.approval_policy,
                "sandbox": session_config.sandbox_mode,
                "config": config_overrides,
                "developerInstructions": developer_instructions,
                "personality": session_config.personality,
                "ephemeral": Value::Null,
                "experimentalRawEvents": false,
                "persistExtendedHistory": false,
            }),
        )
        .await
}

fn build_session_status_details(
    root_thread_id: &str,
    active_turn_id: Option<String>,
    thread_statuses: Vec<(String, ThreadStatus)>,
) -> Value {
    let mut loaded_thread_ids = Vec::new();
    let mut active_thread_ids = Vec::new();
    let mut system_error_thread_ids = Vec::new();
    let mut busy_reasons = Vec::new();

    for (thread_id, status) in thread_statuses {
        loaded_thread_ids.push(thread_id.clone());
        match status {
            ThreadStatus::Active { .. } => active_thread_ids.push(thread_id),
            ThreadStatus::SystemError => system_error_thread_ids.push(thread_id),
            ThreadStatus::NotLoaded | ThreadStatus::Idle => {}
        }
    }

    loaded_thread_ids.sort();
    loaded_thread_ids.dedup();
    active_thread_ids.sort();
    active_thread_ids.dedup();
    system_error_thread_ids.sort();
    system_error_thread_ids.dedup();

    if active_turn_id.is_some() {
        busy_reasons.push("active_turn".to_string());
    }
    if !active_thread_ids.is_empty() {
        busy_reasons.push("loaded_thread_active".to_string());
    }

    json!({
        "quiescent": active_turn_id.is_none() && active_thread_ids.is_empty(),
        "root_thread_id": root_thread_id,
        "active_turn_id": active_turn_id,
        "loaded_thread_ids": loaded_thread_ids,
        "active_thread_ids": active_thread_ids,
        "system_error_thread_ids": system_error_thread_ids,
        "busy_reasons": busy_reasons,
    })
}

async fn query_session_status(session: &mut AppServerSessionState) -> Result<Value> {
    let loaded = session
        .client
        .request::<ThreadLoadedListResponse>("thread/loaded/list", json!({}))
        .await?;
    let mut loaded_thread_ids = loaded.data;
    if !loaded_thread_ids
        .iter()
        .any(|thread_id| thread_id == &session.thread_id)
    {
        loaded_thread_ids.push(session.thread_id.clone());
    }
    loaded_thread_ids.sort();
    loaded_thread_ids.dedup();

    let mut thread_statuses = Vec::new();
    let active_turn_id = session.turn_aliases.active_crp_turn_id.clone();

    for thread_id in &loaded_thread_ids {
        let response = session
            .client
            .request::<ThreadReadResponse>(
                "thread/read",
                json!({
                    "threadId": thread_id,
                    "includeTurns": false,
                }),
            )
            .await?;
        thread_statuses.push((response.thread.id, response.thread.status));
    }
    Ok(build_session_status_details(
        &session.thread_id,
        active_turn_id,
        thread_statuses,
    ))
}

async fn probe_models(config: CrpSessionConfig, _options: &RuntimeOptions) -> Result<ModelsProbe> {
    let workdir = config
        .spawn_cwd
        .clone()
        .or_else(|| config.cwd.clone())
        .unwrap_or(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut client = AppServerClient::start(&workdir).await?;
    let response = client
        .request::<ModelListResponse>("model/list", json!({ "includeHidden": false }))
        .await?;
    client.shutdown().await;
    let models = build_model_infos(&response.data);
    let current_model_id = build_current_model_id(Some(&config), &response.data, None, None);
    Ok(ModelsProbe {
        models,
        current_model_id,
        catalog_source: Some("live_remote".to_string()),
    })
}

fn session_effort(config: &CrpSessionConfig) -> Option<String> {
    config
        .model
        .as_deref()
        .and_then(|model| split_model_and_effort(model).1)
        .or_else(|| {
            config
                .reasoning_effort
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

async fn handle_app_server_event(
    session_state: &mut AppServerSessionState,
    inbound: AppServerInbound,
    router: &CrpEventRouter,
) {
    match inbound {
        AppServerInbound::Notification { method, params } => {
            match translate_notification(session_state, &method, params) {
                Ok(events) => {
                    for (channel, event) in events {
                        dispatch_event(router, channel, event);
                    }
                }
                Err(err) => warn!(%method, %err, "failed to translate app-server notification"),
            }
        }
        AppServerInbound::Request { id, method, params } => {
            handle_server_request(session_state, router, id, &method, params).await;
        }
    }
}

async fn handle_server_request(
    session_state: &mut AppServerSessionState,
    router: &CrpEventRouter,
    id: i64,
    method: &str,
    params: Value,
) {
    let turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .map(|turn_id| session_state.turn_aliases.ensure_crp_turn_id(turn_id));
    emit_unsupported_server_request_notice(
        router,
        &session_state.tracker.session_id,
        turn_id,
        "unsupported_server_request",
        method,
    );
    if let Err(err) = session_state
        .client
        .reject_request(
            id,
            format!("app-server request `{method}` is not supported by codex-crp"),
        )
        .await
    {
        warn!(?err, "failed to reject app-server request");
    }
}
