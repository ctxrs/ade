use crate::app_server::{
    AgentMessageDeltaNotification, AppServerClient, AppServerInbound,
    CommandExecutionOutputDeltaNotification, ItemLifecycleNotification, ModelListResponse,
    ReasoningSummaryPartAddedNotification, ReasoningSummaryTextDeltaNotification,
    ReasoningTextDeltaNotification, ThreadItem, ThreadStartLikeResponse,
    ThreadTokenUsageUpdatedNotification, TurnCompletedNotification, TurnStartResponse,
    TurnStartedNotification,
};
use crate::builtins::{
    build_app_server_config_overrides, build_current_model_id, build_model_infos,
    build_session_command_infos, command_names, merge_config_overrides,
    normalize_ctx_system_prompt_append, resolve_codex_home, split_model_and_effort,
};
use crate::protocol::{
    CrpChannel, CrpCommand, CrpCommandEnvelope, CrpCommandInfo, CrpEvent, CrpEventEnvelope,
    CrpModelInfo, CrpSessionConfig, CrpToolStatus, CrpTurnError, CrpTurnStatus, CRP_VERSION,
};
use crate::RuntimeOptions;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::mpsc;
use tracing::{error, warn};

const CRP_EVENT_DUMP_ENV: &str = "CODEX_CRP_DUMP_CRP_EVENTS_PATH";
const DATA_PLANE_BUFFER_CAPACITY: usize = 256;

static CRP_EVENT_DUMP: OnceLock<Mutex<std::io::BufWriter<std::fs::File>>> = OnceLock::new();

enum RuntimeCommand {
    Parsed(Box<CrpCommand>),
    ParseError { message: String },
}

struct CrpWriter {
    seq: u64,
    out: BufWriter<tokio::io::Stdout>,
}

impl CrpWriter {
    fn new() -> Self {
        Self {
            seq: 0,
            out: BufWriter::new(tokio::io::stdout()),
        }
    }

    async fn send(&mut self, channel: CrpChannel, event: CrpEvent) -> Result<()> {
        let envelope = CrpEventEnvelope {
            v: CRP_VERSION,
            seq: self.next_seq(),
            channel,
            event,
        };
        maybe_dump_crp_event(&envelope);
        let bytes = serde_json::to_vec(&envelope)?;
        self.out.write_all(&bytes).await?;
        self.out.write_all(b"\n").await?;
        self.out.flush().await?;
        Ok(())
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}

struct CrpEventRouter {
    control_tx: mpsc::UnboundedSender<CrpEvent>,
    data_tx: mpsc::Sender<CrpEvent>,
}

impl CrpEventRouter {
    fn new(control_tx: mpsc::UnboundedSender<CrpEvent>, data_tx: mpsc::Sender<CrpEvent>) -> Self {
        Self {
            control_tx,
            data_tx,
        }
    }

    fn send_control(&self, event: CrpEvent) -> Result<(), ()> {
        self.control_tx.send(event).map_err(|_| ())
    }

    fn send_data(&self, event: CrpEvent) -> Result<(), Option<String>> {
        let session_id = event_session_id(&event).map(ToString::to_string);
        match self.data_tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(event))
            | Err(mpsc::error::TrySendError::Closed(event)) => {
                let _ = event;
                Err(session_id)
            }
        }
    }
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
        .cwd
        .clone()
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

async fn probe_models(config: CrpSessionConfig, _options: &RuntimeOptions) -> Result<ModelsProbe> {
    let workdir = config
        .cwd
        .clone()
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

fn translate_notification(
    session_state: &mut AppServerSessionState,
    method: &str,
    params: Value,
) -> Result<Vec<(CrpChannel, CrpEvent)>> {
    match method {
        "thread/tokenUsage/updated" => {
            let payload: ThreadTokenUsageUpdatedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            session_state
                .turn_aliases
                .note_token_usage(payload.turn_id.as_str(), payload.token_usage);
            Ok(Vec::new())
        }
        "turn/started" => {
            let payload: TurnStartedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn_id = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn.id.as_str());
            session_state.tracker.ensure_turn(payload.turn.id.as_str());
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::TurnStarted {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id,
                },
            )])
        }
        "turn/completed" => {
            let payload: TurnCompletedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            session_state
                .turn_aliases
                .note_terminal_turn(payload.turn.id.as_str());
            let turn_id = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn.id.as_str());
            let (status, error) = match payload.turn.status.as_str() {
                "completed" => (CrpTurnStatus::Success, None),
                "interrupted" => (CrpTurnStatus::Interrupted, None),
                "failed" => {
                    let error = payload.turn.error.map(|err| CrpTurnError {
                        message: err.message,
                        kind: Some("app_server_error".to_string()),
                        details: merge_error_details(err.codex_error_info, err.additional_details),
                    });
                    (CrpTurnStatus::Error, error)
                }
                _ => (CrpTurnStatus::Canceled, None),
            };
            let context_window = if matches!(status, CrpTurnStatus::Success) {
                session_state
                    .turn_aliases
                    .take_context_window_for_app_turn(payload.turn.id.as_str())
            } else {
                None
            };
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::TurnCompleted {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id,
                    status,
                    context_window,
                    error,
                },
            )])
        }
        "item/agentMessage/delta" => {
            let payload: AgentMessageDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.message_id = Some(payload.item_id.clone());
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::MessageDelta {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    message_id: payload.item_id,
                    delta: payload.delta,
                },
            )])
        }
        "item/reasoning/summaryPartAdded" => {
            let payload: ReasoningSummaryPartAddedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.reasoning_summaries
                .entry((payload.item_id, payload.summary_index))
                .or_default();
            Ok(Vec::new())
        }
        "item/reasoning/summaryTextDelta" => {
            let payload: ReasoningSummaryTextDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let summary_text = {
                let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
                let state = turn
                    .reasoning_summaries
                    .entry((payload.item_id.clone(), payload.summary_index))
                    .or_default();
                state.text.push_str(&payload.delta);
                state.text.clone()
            };
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::ReasoningSummary {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    summary_index: payload.summary_index,
                    text: summary_text,
                    item_id: Some(payload.item_id),
                },
            )])
        }
        "item/reasoning/textDelta" => {
            let payload: ReasoningTextDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.reasoning_text_seen.insert(payload.item_id.clone());
            let _ = payload.content_index;
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::ReasoningTrace {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    chunk: payload.delta,
                    encoding: None,
                    summary_index: 0,
                    item_id: Some(payload.item_id),
                },
            )])
        }
        "item/commandExecution/outputDelta" => {
            let payload: CommandExecutionOutputDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::ToolOutputDelta {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    tool_call_id: payload.item_id,
                    stream: None,
                    chunk: payload.delta,
                },
            )])
        }
        "item/started" | "item/completed" => {
            let payload: ItemLifecycleNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            translate_item_lifecycle(session_state, method == "item/completed", payload)
        }
        "error" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn translate_item_lifecycle(
    session_state: &mut AppServerSessionState,
    completed: bool,
    payload: ItemLifecycleNotification,
) -> Result<Vec<(CrpChannel, CrpEvent)>> {
    let session_id = session_state.tracker.session_id.clone();
    let crp_turn_id = session_state
        .turn_aliases
        .ensure_crp_turn_id(payload.turn_id.as_str());
    let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());

    let events = match payload.item {
        ThreadItem::AgentMessage { id, text, .. } if completed => {
            turn.message_id = Some(id.clone());
            turn.emitted_final = true;
            vec![(
                CrpChannel::Control,
                CrpEvent::MessageFinal {
                    session_id,
                    turn_id: crp_turn_id,
                    message_id: id,
                    content: text,
                },
            )]
        }
        ThreadItem::Reasoning {
            id,
            summary,
            content,
        } if completed => {
            let mut out = Vec::new();
            for (summary_index, text) in summary.into_iter().enumerate() {
                let summary_index = summary_index as i64;
                let state = turn
                    .reasoning_summaries
                    .entry((id.clone(), summary_index))
                    .or_default();
                if state.text.is_empty() {
                    state.text = text.clone();
                    out.push((
                        CrpChannel::Control,
                        CrpEvent::ReasoningSummary {
                            session_id: session_id.clone(),
                            turn_id: crp_turn_id.clone(),
                            summary_index,
                            text,
                            item_id: Some(id.clone()),
                        },
                    ));
                }
            }
            if !turn.reasoning_text_seen.contains(&id) {
                for block in content {
                    if block.is_empty() {
                        continue;
                    }
                    out.push((
                        CrpChannel::Data,
                        CrpEvent::ReasoningTraceFinal {
                            session_id: session_id.clone(),
                            turn_id: crp_turn_id.clone(),
                            content: block,
                            encoding: None,
                            summary_index: 0,
                            item_id: Some(id.clone()),
                        },
                    ));
                }
            }
            out
        }
        ThreadItem::CommandExecution {
            id,
            command,
            cwd,
            status,
            command_actions,
            aggregated_output,
            exit_code,
            duration_ms,
            ..
        } => {
            let input_preview = json!({
                "command": command,
                "cwd": cwd,
                "command_actions": command_actions,
            });
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    "declined" => (CrpToolStatus::Error, Some("command_declined".to_string())),
                    _ => (
                        CrpToolStatus::Error,
                        exit_code.map(|code| format!("exit_code: {code}")),
                    ),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "exec".to_string(),
                        tool_label: Some("Ran".to_string()),
                        status,
                        output: Some(json!({
                            "aggregated_output": aggregated_output,
                            "exit_code": exit_code,
                            "duration_ms": duration_ms,
                        })),
                        error,
                        input_preview: Some(input_preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "exec".to_string(),
                        tool_label: Some("Running".to_string()),
                        input: Some(input_preview.clone()),
                        input_preview: Some(input_preview),
                    },
                )]
            }
        }
        ThreadItem::FileChange {
            id,
            changes,
            status,
        } => {
            let preview = patch_input_preview(&changes);
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    "declined" => (
                        CrpToolStatus::Error,
                        Some("apply_patch_declined".to_string()),
                    ),
                    _ => (CrpToolStatus::Error, Some("apply_patch_failed".to_string())),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "apply_patch".to_string(),
                        tool_label: Some("Edited".to_string()),
                        status,
                        output: Some(json!({ "changes": changes })),
                        error,
                        input_preview: Some(preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "apply_patch".to_string(),
                        tool_label: Some("Edited".to_string()),
                        input: Some(json!({ "preview": preview.clone() })),
                        input_preview: Some(preview),
                    },
                )]
            }
        }
        ThreadItem::McpToolCall {
            id,
            server,
            tool,
            status,
            arguments,
            result,
            error,
            duration_ms,
        } => {
            let tool_name = format!("mcp.{server}.{tool}");
            let input_preview = json!({ "server": server, "tool": tool });
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    _ => (CrpToolStatus::Error, error.map(|error| error.message)),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name,
                        tool_label: Some("MCP".to_string()),
                        status,
                        output: Some(json!({
                            "duration_ms": duration_ms,
                            "result": result,
                        })),
                        error,
                        input_preview: Some(input_preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name,
                        tool_label: Some("MCP".to_string()),
                        input: Some(json!({
                            "server": input_preview["server"],
                            "tool": input_preview["tool"],
                            "arguments": arguments,
                        })),
                        input_preview: Some(input_preview),
                    },
                )]
            }
        }
        ThreadItem::WebSearch { id, query, .. } => {
            if completed {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "web_search".to_string(),
                        tool_label: Some("Searched".to_string()),
                        status: CrpToolStatus::Success,
                        output: Some(json!({ "query": query })),
                        error: None,
                        input_preview: Some(json!({ "query": query })),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "web_search".to_string(),
                        tool_label: Some("Search".to_string()),
                        input: None,
                        input_preview: None,
                    },
                )]
            }
        }
        ThreadItem::ImageView { id, path } => {
            let payload = json!({ "path": path });
            if completed {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "view_image".to_string(),
                        tool_label: Some("Viewed".to_string()),
                        status: CrpToolStatus::Success,
                        output: Some(payload),
                        error: None,
                        input_preview: None,
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "view_image".to_string(),
                        tool_label: Some("View".to_string()),
                        input: Some(payload.clone()),
                        input_preview: Some(payload),
                    },
                )]
            }
        }
        ThreadItem::ContextCompaction { .. } if completed => vec![
            (
                CrpChannel::Control,
                CrpEvent::SessionNotice {
                    session_id: session_id.clone(),
                    turn_id: Some(crp_turn_id.clone()),
                    code: "context.compacted".to_string(),
                    severity: Some("info".to_string()),
                    message: Some("Context compacted. Earlier turns were summarized.".to_string()),
                    details: None,
                    transient: None,
                },
            ),
            (
                CrpChannel::Control,
                CrpEvent::SessionGap {
                    session_id: session_id.clone(),
                    reason: Some("context_compacted".to_string()),
                },
            ),
        ],
        ThreadItem::EnteredReviewMode { .. }
        | ThreadItem::ExitedReviewMode { .. }
        | ThreadItem::ContextCompaction { .. }
        | ThreadItem::Unknown
        | ThreadItem::AgentMessage { .. }
        | ThreadItem::Reasoning { .. } => Vec::new(),
    };
    Ok(events)
}

fn merge_error_details(
    codex_error_info: Option<Value>,
    additional_details: Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = codex_error_info {
        parts.push(value.to_string());
    }
    if let Some(details) = additional_details {
        let trimmed = details.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn patch_input_preview(changes: &[crate::app_server::FileUpdateChange]) -> Value {
    let mut paths: Vec<String> = changes.iter().map(|change| change.path.clone()).collect();
    paths.sort();
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in changes {
        for line in change.diff.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }
            if line.starts_with('+') {
                added += 1;
            } else if line.starts_with('-') {
                removed += 1;
            }
        }
    }
    json!({
        "paths": paths,
        "diff_stats": {
            "added": added,
            "removed": removed,
            "files": changes.len(),
        }
    })
}

fn canonical_context_window_from_thread_usage(
    token_usage: &crate::app_server::ThreadTokenUsage,
) -> Option<Value> {
    let context_window_tokens = token_usage.model_context_window?;
    if context_window_tokens == 0 {
        return None;
    }
    let total_tokens = token_usage.total.total_tokens;
    let input_tokens = token_usage.total.input_tokens;
    let output_tokens = token_usage.total.output_tokens;
    let reasoning_output_tokens = token_usage.total.reasoning_output_tokens;
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(total_tokens);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": total_tokens,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
        "total_input_tokens": input_tokens,
        "total_output_tokens": output_tokens.saturating_add(reasoning_output_tokens),
    }))
}

fn emit_unsupported_server_request_notice(
    router: &CrpEventRouter,
    session_id: &str,
    turn_id: Option<String>,
    code: &str,
    method: &str,
) {
    dispatch_event(
        router,
        CrpChannel::Control,
        CrpEvent::SessionNotice {
            session_id: session_id.to_string(),
            turn_id,
            code: code.to_string(),
            severity: Some("warning".to_string()),
            message: Some(format!(
                "app-server request `{method}` is not supported by codex-crp"
            )),
            details: Some(json!({ "request_method": method })),
            transient: Some(false),
        },
    );
}

fn emit_turn_request_error(
    router: &CrpEventRouter,
    session_id: &str,
    turn_id: Option<String>,
    kind: &str,
    message: String,
) {
    let Some(turn_id) = turn_id else {
        warn!(%kind, %message, "request failed without turn_id; unable to emit turn.completed");
        return;
    };
    dispatch_event(
        router,
        CrpChannel::Control,
        CrpEvent::TurnCompleted {
            session_id: session_id.to_string(),
            turn_id,
            status: CrpTurnStatus::Error,
            context_window: None,
            error: Some(CrpTurnError {
                message,
                kind: Some(kind.to_string()),
                details: None,
            }),
        },
    );
}

fn dispatch_event(router: &CrpEventRouter, channel: CrpChannel, event: CrpEvent) {
    match channel {
        CrpChannel::Control => {
            if router.send_control(event).is_err() {
                warn!("failed to dispatch control event");
            }
        }
        CrpChannel::Data => {
            if let Err(Some(session_id)) = router.send_data(event) {
                let _ = router.send_control(CrpEvent::SessionGap {
                    session_id,
                    reason: Some("data_plane_overflow".to_string()),
                });
            }
        }
    }
}

fn event_session_id(event: &CrpEvent) -> Option<&str> {
    match event {
        CrpEvent::MessageDelta { session_id, .. }
        | CrpEvent::ReasoningTrace { session_id, .. }
        | CrpEvent::ReasoningTraceFinal { session_id, .. }
        | CrpEvent::ToolOutputDelta { session_id, .. } => Some(session_id),
        _ => None,
    }
}

async fn read_commands(tx: mpsc::UnboundedSender<RuntimeCommand>) {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let command = match serde_json::from_str::<CrpCommandEnvelope>(trimmed) {
            Ok(envelope) => RuntimeCommand::Parsed(Box::new(envelope.command)),
            Err(err) => RuntimeCommand::ParseError {
                message: err.to_string(),
            },
        };
        if tx.send(command).is_err() {
            break;
        }
    }
}

async fn run_writer(
    mut writer: CrpWriter,
    mut control_rx: mpsc::UnboundedReceiver<CrpEvent>,
    mut data_rx: mpsc::Receiver<CrpEvent>,
) -> Result<()> {
    loop {
        tokio::select! {
            biased;
            Some(event) = control_rx.recv() => writer.send(CrpChannel::Control, event).await?,
            Some(event) = data_rx.recv() => writer.send(CrpChannel::Data, event).await?,
            else => break,
        }
    }
    Ok(())
}

fn maybe_dump_crp_event(envelope: &CrpEventEnvelope) {
    let Ok(path) = std::env::var(CRP_EVENT_DUMP_ENV) else {
        return;
    };

    let writer = CRP_EVENT_DUMP.get_or_init(|| {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("failed to open CODEX_CRP_DUMP_CRP_EVENTS_PATH");
        Mutex::new(std::io::BufWriter::new(file))
    });

    let Ok(mut writer) = writer.lock() else {
        return;
    };
    if serde_json::to_writer(&mut *writer, envelope).is_ok() {
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;

    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum FixtureStep {
        BindTurn {
            app_turn_id: String,
            crp_turn_id: String,
        },
        Notification {
            method: String,
            params: Value,
        },
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct SnapshotEvent {
        channel: String,
        event: Value,
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct SnapshotOutput {
        events: Vec<SnapshotEvent>,
        aliases: HashMap<String, String>,
        latest_token_usage: Option<crate::app_server::ThreadTokenUsage>,
    }

    fn testdata_path(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(file)
    }

    fn replay_fixture(file: &str) -> SnapshotOutput {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let runtime_guard = runtime.enter();
        let mut state = AppServerSessionState {
            tracker: TurnTracker::new("fixture-session".to_string()),
            client: AppServerClient::test_stub(),
            thread_id: "thr_fixture".to_string(),
            default_cwd: PathBuf::from("/tmp"),
            default_model: "gpt-5.4".to_string(),
            default_effort: Some("medium".to_string()),
            opened_commands: Vec::new(),
            opened_slash_commands: Vec::new(),
            turn_aliases: TurnAliasState::new(),
        };
        let input = fs::read_to_string(testdata_path(file)).expect("fixture should exist");
        let mut events = Vec::new();
        for line in input.lines().filter(|line| !line.trim().is_empty()) {
            let step: FixtureStep = serde_json::from_str(line).expect("fixture line should parse");
            let translated = match step {
                FixtureStep::BindTurn {
                    app_turn_id,
                    crp_turn_id,
                } => {
                    state
                        .turn_aliases
                        .bind_turn_alias(app_turn_id, Some(crp_turn_id));
                    Vec::new()
                }
                FixtureStep::Notification { method, params } => {
                    translate_notification(&mut state, &method, params)
                        .expect("notification should translate")
                }
            };
            for (channel, event) in translated {
                events.push(SnapshotEvent {
                    channel: match channel {
                        CrpChannel::Control => "control".to_string(),
                        CrpChannel::Data => "data".to_string(),
                    },
                    event: serde_json::to_value(event).expect("event should serialize"),
                });
            }
        }

        let output = SnapshotOutput {
            events,
            aliases: state.turn_aliases.app_to_crp.clone(),
            latest_token_usage: state.turn_aliases.latest_token_usage.clone(),
        };
        drop(state);
        drop(runtime_guard);
        drop(runtime);
        output
    }

    fn assert_snapshot(input: &str, expected: &str) {
        let actual = replay_fixture(input);
        let expected: SnapshotOutput = serde_json::from_str(
            &fs::read_to_string(testdata_path(expected)).expect("expected snapshot should exist"),
        )
        .expect("expected snapshot should parse");
        assert_eq!(actual, expected);
    }

    #[test]
    fn basic_message_snapshot_matches() {
        assert_snapshot(
            "app_server_basic_message.input.jsonl",
            "app_server_basic_message.expected.json",
        );
    }

    #[test]
    fn reasoning_and_tools_snapshot_matches() {
        assert_snapshot(
            "app_server_reasoning_and_tools.input.jsonl",
            "app_server_reasoning_and_tools.expected.json",
        );
    }

    #[test]
    fn error_and_usage_snapshot_matches() {
        assert_snapshot(
            "app_server_error_and_usage.input.jsonl",
            "app_server_error_and_usage.expected.json",
        );
    }

    #[test]
    fn canonical_context_window_matches_expected_shape() {
        let usage = crate::app_server::ThreadTokenUsage {
            total: crate::app_server::TokenUsageBreakdown {
                total_tokens: 4200,
                input_tokens: 3000,
                cached_input_tokens: 0,
                output_tokens: 900,
                reasoning_output_tokens: 300,
            },
            last: crate::app_server::TokenUsageBreakdown {
                total_tokens: 4200,
                input_tokens: 3000,
                cached_input_tokens: 0,
                output_tokens: 900,
                reasoning_output_tokens: 300,
            },
            model_context_window: Some(128000),
        };
        let metrics = canonical_context_window_from_thread_usage(&usage).expect("metrics");
        assert_eq!(metrics["context_tokens_estimate"], json!(4200));
        assert_eq!(metrics["context_window_tokens"], json!(128000));
    }
}
