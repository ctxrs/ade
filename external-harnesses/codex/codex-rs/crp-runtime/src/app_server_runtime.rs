use super::CrpChannel;
use super::CrpCommand;
use super::CrpCommandInfo;
use super::CrpEvent;
use super::CrpEventRouter;
use super::CrpModelInfo;
use super::CrpSessionConfig;
use super::CrpTurnError;
use super::CrpTurnStatus;
use super::CrpWriter;
use super::RuntimeCommand;
use super::TurnTracker;
use super::build_crp_model_infos;
use super::build_current_model_id;
use super::build_session_command_infos;
use super::command_names;
use super::dispatch_event;
use super::find_rollout_path_fallback;
use super::find_thread_path_by_id_str;
use super::load_config_from_crp;
use super::map_codex_event;
use super::mcp_servers_to_cli_overrides;
use super::read_commands;
use super::rollout_filename_matches_id;
use super::run_writer;
use super::split_model_and_effort;

use anyhow::Context;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_client::InProcessServerEvent;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::JSONRPCNotification;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ReviewStartParams;
use codex_app_server_protocol::ReviewStartResponse;
use codex_app_server_protocol::ReviewTarget as ApiReviewTarget;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadRollbackParams;
use codex_app_server_protocol::ThreadRollbackResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_arg0::Arg0DispatchPaths;
use codex_core::config::Config;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_core::models_manager::manager::ModelCatalogSource;
use codex_feedback::CodexFeedback;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use toml::Value as TomlValue;
use tracing::error;
use tracing::warn;

struct RequestIdSequencer {
    next: i64,
}

impl RequestIdSequencer {
    fn new() -> Self {
        Self { next: 1 }
    }

    fn next(&mut self) -> RequestId {
        let id = self.next;
        self.next += 1;
        RequestId::Integer(id)
    }
}

struct TurnAliasState {
    app_to_crp: HashMap<String, String>,
    crp_to_app: HashMap<String, String>,
    pending_compact_turns: VecDeque<String>,
    active_app_turn_id: Option<String>,
    active_crp_turn_id: Option<String>,
    latest_token_usage: Option<ThreadTokenUsage>,
    token_usage_by_app_turn: HashMap<String, ThreadTokenUsage>,
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
        if let Some(pending_compact) = self.pending_compact_turns.pop_front() {
            self.bind_turn_alias(app_turn_id.to_string(), Some(pending_compact.clone()));
            return pending_compact;
        }
        self.bind_turn_alias(app_turn_id.to_string(), None)
    }

    fn note_terminal_turn(&mut self, app_turn_id: &str) {
        if self.active_app_turn_id.as_deref() == Some(app_turn_id) {
            self.active_app_turn_id = None;
        }
        if let Some(crp_turn_id) = self.app_to_crp.get(app_turn_id).cloned()
            && self.active_crp_turn_id.as_deref() == Some(crp_turn_id.as_str())
        {
            self.active_crp_turn_id = None;
        }
    }

    fn note_token_usage(&mut self, app_turn_id: &str, token_usage: ThreadTokenUsage) {
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
    client: Option<InProcessAppServerClient>,
    request_ids: RequestIdSequencer,
    thread_id: String,
    default_cwd: PathBuf,
    default_model: String,
    default_effort: Option<ReasoningEffort>,
    default_summary: Option<codex_protocol::config_types::ReasoningSummary>,
    default_service_tier: Option<codex_protocol::config_types::ServiceTier>,
    default_approval_policy: AskForApproval,
    default_sandbox_policy: SandboxPolicy,
    opened_commands: Vec<CrpCommandInfo>,
    opened_slash_commands: Vec<String>,
    turn_aliases: TurnAliasState,
}

impl AppServerSessionState {
    async fn shutdown(self) {
        if let Some(client) = self.client
            && let Err(err) = client.shutdown().await
        {
            warn!(?err, "app-server-backed codex-crp shutdown failed");
        }
    }
}

enum RuntimeInput {
    Command(RuntimeCommand),
    ServerEvent(InProcessServerEvent),
}

async fn next_runtime_input(
    session: &mut Option<AppServerSessionState>,
    cmd_rx: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
) -> Option<RuntimeInput> {
    let Some(session_state) = session.as_mut() else {
        return cmd_rx.recv().await.map(RuntimeInput::Command);
    };
    let Some(client) = session_state.client.as_mut() else {
        return cmd_rx.recv().await.map(RuntimeInput::Command);
    };

    tokio::select! {
        Some(cmd) = cmd_rx.recv() => Some(RuntimeInput::Command(cmd)),
        maybe_event = client.next_event() => maybe_event.map(RuntimeInput::ServerEvent),
        else => None,
    }
}

pub(super) async fn run_main(
    cli_kv_overrides: &[(String, TomlValue)],
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<()> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(read_commands(cmd_tx));

    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (data_tx, data_rx) = mpsc::channel(super::DATA_PLANE_BUFFER_CAPACITY);
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
            Some(RuntimeInput::Command(cmd)) => {
                match cmd {
                    RuntimeCommand::ParseError { message } => {
                        error!(%message, "failed to parse CRP command");
                    }
                    RuntimeCommand::Parsed(CrpCommand::ToolResult { .. }) => {
                        warn!("tool.result ignored: app-server-backed runtime owns tool execution");
                    }
                    RuntimeCommand::Parsed(command) => {
                        handle_command(
                            command,
                            &mut session,
                            &router,
                            cli_kv_overrides,
                            arg0_paths.clone(),
                        )
                        .await?;
                    }
                }
            }
            Some(RuntimeInput::ServerEvent(server_event)) => {
                if let Some(session_state) = session.as_mut() {
                    handle_server_event(session_state, server_event, &router).await;
                }
            }
            None => break,
        }
    }

    if let Some(session_state) = session {
        session_state.shutdown().await;
    }

    Ok(())
}

async fn handle_command(
    command: CrpCommand,
    session: &mut Option<AppServerSessionState>,
    router: &CrpEventRouter,
    cli_kv_overrides: &[(String, TomlValue)],
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<()> {
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

            let config = config.unwrap_or(CrpSessionConfig {
                cwd: None,
                model: None,
                reasoning_effort: None,
                model_provider: None,
                approval_policy: None,
                sandbox_mode: None,
                reasoning_trace_enabled: None,
                personality: None,
                mcp_servers: None,
            });

            let provider_session_id = provider_session_id.or_else(|| {
                std::env::var("CTX_PROVIDER_SESSION_REF")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });

            let mut state =
                open_session(config, cli_kv_overrides, arg0_paths, provider_session_id).await?;

            let provider_session_id = state.thread_id.clone();
            let session_id = session_id.unwrap_or_else(|| provider_session_id.clone());
            state.tracker.session_id = session_id.clone();

            if router
                .send_control(CrpEvent::SessionOpened {
                    session_id,
                    provider_session_id: Some(provider_session_id),
                    commands: Some(state.opened_commands.clone()),
                    slash_commands: Some(state.opened_slash_commands.clone()),
                })
                .is_err()
            {
                warn!("failed to send session.opened event");
            }

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
            let Some(session_state) = session.as_mut() else {
                warn!("session.prompt ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.prompt ignored: session_id mismatch");
                return Ok(());
            }

            let items = match (items, prompt) {
                (Some(items), _) => items,
                (None, Some(prompt)) => vec![UserInput::Text {
                    text: prompt,
                    text_elements: Vec::new(),
                }],
                (None, None) => {
                    warn!("session.prompt ignored: missing prompt or items");
                    return Ok(());
                }
            };

            let cwd = cwd.unwrap_or_else(|| session_state.default_cwd.clone());
            let model = model.unwrap_or_else(|| session_state.default_model.clone());
            let (model, effort_override) = split_model_and_effort(&model);
            let effort = effort_override
                .or(reasoning_effort)
                .or(session_state.default_effort.clone());

            let request = ClientRequest::TurnStart {
                request_id: session_state.request_ids.next(),
                params: TurnStartParams {
                    thread_id: session_state.thread_id.clone(),
                    input: items.into_iter().map(Into::into).collect(),
                    cwd: Some(cwd),
                    approval_policy: Some(session_state.default_approval_policy.clone().into()),
                    sandbox_policy: Some(session_state.default_sandbox_policy.clone().into()),
                    model: Some(model),
                    service_tier: session_state.default_service_tier.clone().map(Some),
                    effort,
                    summary: session_state.default_summary.clone(),
                    personality: None,
                    output_schema: None,
                    collaboration_mode: None,
                },
            };
            let requested_turn_id = turn_id.clone();
            match send_request_with_response::<TurnStartResponse>(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request,
                "turn/start",
            )
            .await
            {
                Ok(response) => {
                    let _ = session_state
                        .turn_aliases
                        .bind_turn_alias(response.turn.id.clone(), requested_turn_id);
                }
                Err(err) => emit_turn_request_error(
                    router,
                    &session_state.tracker.session_id,
                    turn_id,
                    "turn_start_failed",
                    err,
                ),
            }
        }
        CrpCommand::SessionCompact {
            session_id,
            turn_id,
        } => {
            let Some(session_state) = session.as_mut() else {
                warn!("session.compact ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.compact ignored: session_id mismatch");
                return Ok(());
            }

            if let Some(turn_id) = turn_id.clone() {
                session_state
                    .turn_aliases
                    .pending_compact_turns
                    .push_back(turn_id);
            }

            let request = ClientRequest::ThreadCompactStart {
                request_id: session_state.request_ids.next(),
                params: codex_app_server_protocol::ThreadCompactStartParams {
                    thread_id: session_state.thread_id.clone(),
                },
            };
            if let Err(err) =
                send_request_with_response::<codex_app_server_protocol::ThreadCompactStartResponse>(
                    session_state
                        .client
                        .as_ref()
                        .expect("session client should exist"),
                    request,
                    "thread/compact/start",
                )
                .await
            {
                emit_turn_request_error(
                    router,
                    &session_state.tracker.session_id,
                    turn_id,
                    "thread_compact_start_failed",
                    err,
                );
            }
        }
        CrpCommand::SessionUndo {
            session_id,
            turn_id,
        } => {
            let Some(session_state) = session.as_mut() else {
                warn!("session.undo ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.undo ignored: session_id mismatch");
                return Ok(());
            }

            let request = ClientRequest::ThreadRollback {
                request_id: session_state.request_ids.next(),
                params: ThreadRollbackParams {
                    thread_id: session_state.thread_id.clone(),
                    num_turns: 1,
                },
            };
            match send_request_with_response::<ThreadRollbackResponse>(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request,
                "thread/rollback",
            )
            .await
            {
                Ok(_) => {
                    if let Some(turn_id) = turn_id {
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::TurnCompleted {
                                session_id: session_state.tracker.session_id.clone(),
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
                    &session_state.tracker.session_id,
                    turn_id,
                    "thread_rollback_failed",
                    err,
                ),
            }
        }
        CrpCommand::SessionReview {
            session_id,
            turn_id,
            instructions,
        } => {
            let Some(session_state) = session.as_mut() else {
                warn!("session.review ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.review ignored: session_id mismatch");
                return Ok(());
            }

            let instructions = instructions.and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            });
            let target = match instructions {
                Some(instructions) => ApiReviewTarget::Custom { instructions },
                None => ApiReviewTarget::UncommittedChanges,
            };
            let request = ClientRequest::ReviewStart {
                request_id: session_state.request_ids.next(),
                params: ReviewStartParams {
                    thread_id: session_state.thread_id.clone(),
                    target,
                    delivery: None,
                },
            };
            let requested_turn_id = turn_id.clone();
            match send_request_with_response::<ReviewStartResponse>(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request,
                "review/start",
            )
            .await
            {
                Ok(response) => {
                    let _ = session_state
                        .turn_aliases
                        .bind_turn_alias(response.turn.id.clone(), requested_turn_id);
                }
                Err(err) => emit_turn_request_error(
                    router,
                    &session_state.tracker.session_id,
                    turn_id,
                    "review_start_failed",
                    err,
                ),
            }
        }
        CrpCommand::SessionSetModel {
            session_id,
            model_id,
        } => {
            let Some(session_state) = session.as_mut() else {
                warn!("session.set_model ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.set_model ignored: session_id mismatch");
                return Ok(());
            }

            let Some(requested_model_id) = model_id
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                warn!("session.set_model ignored: missing model_id");
                return Ok(());
            };

            let (requested_model, requested_effort) = split_model_and_effort(&requested_model_id);
            let request = ClientRequest::ModelList {
                request_id: session_state.request_ids.next(),
                params: codex_app_server_protocol::ModelListParams {
                    cursor: None,
                    limit: None,
                    include_hidden: Some(false),
                },
            };

            match send_request_with_response::<ModelListResponse>(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request,
                "model/list",
            )
            .await
            {
                Ok(response) => {
                    let supported = response
                        .data
                        .iter()
                        .any(|model| model.id == requested_model);
                    if supported {
                        session_state.default_model = requested_model;
                        session_state.default_effort = requested_effort;
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::SessionNotice {
                                session_id: session_state.tracker.session_id.clone(),
                                turn_id: None,
                                code: "session_model_updated".to_string(),
                                severity: Some("info".to_string()),
                                message: Some(format!(
                                    "session model updated to {requested_model_id}"
                                )),
                                details: Some(json!({ "model_id": requested_model_id })),
                                transient: Some(false),
                            },
                        );
                    } else {
                        dispatch_event(
                            router,
                            CrpChannel::Control,
                            CrpEvent::SessionNotice {
                                session_id: session_state.tracker.session_id.clone(),
                                turn_id: None,
                                code: "session_model_update_failed".to_string(),
                                severity: Some("error".to_string()),
                                message: Some(format!(
                                    "failed to update session model to {requested_model_id}: model not found"
                                )),
                                details: Some(json!({ "model_id": requested_model_id })),
                                transient: Some(false),
                            },
                        );
                    }
                }
                Err(err) => {
                    dispatch_event(
                        router,
                        CrpChannel::Control,
                        CrpEvent::SessionNotice {
                            session_id: session_state.tracker.session_id.clone(),
                            turn_id: None,
                            code: "session_model_update_failed".to_string(),
                            severity: Some("error".to_string()),
                            message: Some(format!(
                                "failed to update session model to {requested_model_id}: {err}"
                            )),
                            details: Some(json!({ "model_id": requested_model_id })),
                            transient: Some(false),
                        },
                    );
                }
            }
        }
        CrpCommand::ModelsList { config } => {
            let config = config.unwrap_or(CrpSessionConfig {
                cwd: None,
                model: None,
                reasoning_effort: None,
                model_provider: None,
                approval_policy: None,
                sandbox_mode: None,
                reasoning_trace_enabled: None,
                personality: None,
                mcp_servers: None,
            });
            let config = load_config_from_crp(config, cli_kv_overrides, arg0_paths).await?;
            let (models, current_model_id, catalog_source) = probe_models(&config).await;
            if router
                .send_control(CrpEvent::ModelsList {
                    models,
                    current_model_id,
                    catalog_source,
                })
                .is_err()
            {
                warn!("failed to send models.list event");
            }
        }
        CrpCommand::SessionCancel {
            session_id,
            turn_id,
        } => {
            let Some(session_state) = session.as_mut() else {
                warn!("session.cancel ignored: no active session");
                return Ok(());
            };
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.cancel ignored: session_id mismatch");
                return Ok(());
            }

            let app_turn_id = session_state
                .turn_aliases
                .app_turn_id_for_crp(turn_id.as_deref())
                .or_else(|| session_state.turn_aliases.active_app_turn_id.clone());

            let Some(app_turn_id) = app_turn_id else {
                warn!("session.cancel ignored: no active turn to interrupt");
                return Ok(());
            };

            let request = ClientRequest::TurnInterrupt {
                request_id: session_state.request_ids.next(),
                params: TurnInterruptParams {
                    thread_id: session_state.thread_id.clone(),
                    turn_id: app_turn_id,
                },
            };
            if let Err(err) = send_request_with_response::<TurnInterruptResponse>(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request,
                "turn/interrupt",
            )
            .await
            {
                dispatch_event(
                    router,
                    CrpChannel::Control,
                    CrpEvent::SessionNotice {
                        session_id: session_state.tracker.session_id.clone(),
                        turn_id,
                        code: "turn_interrupt_failed".to_string(),
                        severity: Some("error".to_string()),
                        message: Some(err),
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

async fn open_session(
    session_config: CrpSessionConfig,
    cli_kv_overrides: &[(String, TomlValue)],
    arg0_paths: Arg0DispatchPaths,
    provider_session_id: Option<String>,
) -> anyhow::Result<AppServerSessionState> {
    let config =
        load_config_from_crp(session_config.clone(), cli_kv_overrides, arg0_paths.clone()).await?;
    let run_cli_overrides = combined_cli_overrides(cli_kv_overrides, session_config);
    let config_warnings: Vec<ConfigWarningNotification> = config
        .startup_warnings
        .iter()
        .map(|warning| ConfigWarningNotification {
            summary: warning.clone(),
            details: None,
            path: None,
            range: None,
        })
        .collect();

    let client = InProcessAppServerClient::start(InProcessClientStartArgs {
        arg0_paths,
        config: Arc::new(config.clone()),
        cli_overrides: run_cli_overrides,
        loader_overrides: LoaderOverrides::default(),
        cloud_requirements: CloudRequirementsLoader::default(),
        feedback: CodexFeedback::new(),
        config_warnings,
        session_source: SessionSource::Exec,
        enable_codex_api_key_env: true,
        client_name: "codex-crp".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    .context("failed to initialize in-process app-server client")?;

    let mut request_ids = RequestIdSequencer::new();
    let (thread_id, default_model, default_effort) = if let Some(provider_session_id) =
        provider_session_id
    {
        match resume_thread(
            &client,
            &config,
            request_ids.next(),
            provider_session_id.clone(),
        )
        .await
        {
            Ok(response) => (
                response.thread.id,
                response.model,
                response.reasoning_effort,
            ),
            Err(err) => {
                warn!(provider_session_id = %provider_session_id, %err, "thread/resume failed; starting a new thread");
                let response = start_thread(&client, &config, request_ids.next())
                    .await
                    .map_err(anyhow::Error::msg)?;
                (
                    response.thread.id,
                    response.model,
                    response.reasoning_effort,
                )
            }
        }
    } else {
        let response = start_thread(&client, &config, request_ids.next())
            .await
            .map_err(anyhow::Error::msg)?;
        (
            response.thread.id,
            response.model,
            response.reasoning_effort,
        )
    };

    let opened_commands = build_session_command_infos(&config).await;
    let opened_slash_commands = command_names(&opened_commands);

    Ok(AppServerSessionState {
        tracker: TurnTracker::new(String::new()),
        client: Some(client),
        request_ids,
        thread_id,
        default_cwd: config.cwd.to_path_buf(),
        default_model,
        default_effort,
        default_summary: config.model_reasoning_summary.clone(),
        default_service_tier: config.service_tier.clone(),
        default_approval_policy: config.permissions.approval_policy.value(),
        default_sandbox_policy: config.permissions.sandbox_policy.get().clone(),
        opened_commands,
        opened_slash_commands,
        turn_aliases: TurnAliasState::new(),
    })
}

async fn start_thread(
    client: &InProcessAppServerClient,
    config: &Config,
    request_id: RequestId,
) -> Result<ThreadStartResponse, String> {
    send_request_with_response(
        client,
        ClientRequest::ThreadStart {
            request_id,
            params: ThreadStartParams {
                model: config.model.clone(),
                model_provider: Some(config.model_provider_id.clone()),
                service_tier: config.service_tier.clone().map(Some),
                cwd: Some(config.cwd.to_string_lossy().to_string()),
                approval_policy: Some(config.permissions.approval_policy.value().into()),
                sandbox: sandbox_mode_from_policy(config.permissions.sandbox_policy.get()),
                config: None,
                service_name: None,
                base_instructions: config.base_instructions.clone(),
                developer_instructions: config.developer_instructions.clone(),
                personality: config.personality.clone(),
                ephemeral: Some(config.ephemeral),
                dynamic_tools: None,
                mock_experimental_field: None,
                experimental_raw_events: false,
                persist_extended_history: false,
            },
        },
        "thread/start",
    )
    .await
}

async fn resume_thread(
    client: &InProcessAppServerClient,
    config: &Config,
    request_id: RequestId,
    provider_session_id: String,
) -> Result<ThreadResumeResponse, String> {
    let path =
        match find_thread_path_by_id_str(&config.codex_home, provider_session_id.as_str()).await {
            Ok(Some(path)) if rollout_filename_matches_id(&path, provider_session_id.as_str()) => {
                Some(path)
            }
            Ok(Some(_)) | Ok(None) | Err(_) => {
                find_rollout_path_fallback(&config.codex_home, provider_session_id.as_str()).await
            }
        };

    send_request_with_response(
        client,
        ClientRequest::ThreadResume {
            request_id,
            params: ThreadResumeParams {
                thread_id: provider_session_id,
                history: None,
                path,
                model: config.model.clone(),
                model_provider: Some(config.model_provider_id.clone()),
                service_tier: config.service_tier.clone().map(Some),
                cwd: Some(config.cwd.to_string_lossy().to_string()),
                approval_policy: Some(config.permissions.approval_policy.value().into()),
                sandbox: sandbox_mode_from_policy(config.permissions.sandbox_policy.get()),
                config: None,
                base_instructions: config.base_instructions.clone(),
                developer_instructions: config.developer_instructions.clone(),
                personality: config.personality.clone(),
                persist_extended_history: false,
            },
        },
        "thread/resume",
    )
    .await
}

async fn handle_server_event(
    session_state: &mut AppServerSessionState,
    server_event: InProcessServerEvent,
    router: &CrpEventRouter,
) {
    match server_event {
        InProcessServerEvent::ServerRequest(request) => {
            handle_server_request(session_state, request, router).await;
        }
        InProcessServerEvent::ServerNotification(notification) => {
            for (channel, event) in translate_server_notification(session_state, notification) {
                dispatch_event(router, channel, event);
            }
        }
        InProcessServerEvent::LegacyNotification(notification) => {
            match translate_legacy_notification(session_state, notification) {
                Ok(events) => {
                    for (channel, event) in events {
                        dispatch_event(router, channel, event);
                    }
                }
                Err(err) => {
                    warn!(%err, "failed to translate app-server legacy notification");
                }
            }
        }
        InProcessServerEvent::Lagged { skipped } => {
            dispatch_event(
                router,
                CrpChannel::Control,
                CrpEvent::SessionGap {
                    session_id: session_state.tracker.session_id.clone(),
                    reason: Some(format!("app_server_lagged:{skipped}")),
                },
            );
        }
    }
}

async fn handle_server_request(
    session_state: &mut AppServerSessionState,
    request: ServerRequest,
    router: &CrpEventRouter,
) {
    let method = server_request_method_name(&request);
    let thread_id = session_state.thread_id.clone();
    let handle_result = match request {
        ServerRequest::McpServerElicitationRequest { request_id, .. } => {
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                "ctx codex-crp does not support app-server elicitation requests yet".to_string(),
            )
            .await
        }
        ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
            emit_unsupported_server_request_notice(
                router,
                &session_state.tracker.session_id,
                Some(params.turn_id.clone()),
                "command_execution_approval_unsupported",
                &method,
            );
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                format!("command execution approval is not supported for thread `{thread_id}`"),
            )
            .await
        }
        ServerRequest::FileChangeRequestApproval { request_id, params } => {
            emit_unsupported_server_request_notice(
                router,
                &session_state.tracker.session_id,
                Some(params.turn_id.clone()),
                "file_change_approval_unsupported",
                &method,
            );
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                format!("file change approval is not supported for thread `{thread_id}`"),
            )
            .await
        }
        ServerRequest::ToolRequestUserInput { request_id, params } => {
            emit_unsupported_server_request_notice(
                router,
                &session_state.tracker.session_id,
                Some(params.turn_id.clone()),
                "request_user_input_unsupported",
                &method,
            );
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                format!("request_user_input is not supported for thread `{thread_id}`"),
            )
            .await
        }
        ServerRequest::DynamicToolCall {
            request_id,
            params: _,
        } => {
            emit_unsupported_server_request_notice(
                router,
                &session_state.tracker.session_id,
                None,
                "dynamic_tool_call_unsupported",
                &method,
            );
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                format!("dynamic tool calls are not supported for thread `{thread_id}`"),
            )
            .await
        }
        ServerRequest::PermissionsRequestApproval { request_id, params } => {
            emit_unsupported_server_request_notice(
                router,
                &session_state.tracker.session_id,
                Some(params.turn_id.clone()),
                "permissions_request_unsupported",
                &method,
            );
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                format!("permission requests are not supported for thread `{thread_id}`"),
            )
            .await
        }
        ServerRequest::ChatgptAuthTokensRefresh { request_id, .. } => {
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                "chatgpt auth refresh is not supported by codex-crp".to_string(),
            )
            .await
        }
        ServerRequest::ApplyPatchApproval { request_id, .. } => {
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                "apply_patch approval is not supported by codex-crp".to_string(),
            )
            .await
        }
        ServerRequest::ExecCommandApproval { request_id, .. } => {
            reject_server_request(
                session_state
                    .client
                    .as_ref()
                    .expect("session client should exist"),
                request_id,
                &method,
                "exec command approval is not supported by codex-crp".to_string(),
            )
            .await
        }
    };

    if let Err(err) = handle_result {
        warn!(%err, "failed to resolve app-server request");
        dispatch_event(
            router,
            CrpChannel::Control,
            CrpEvent::SessionNotice {
                session_id: session_state.tracker.session_id.clone(),
                turn_id: session_state.turn_aliases.active_crp_turn_id.clone(),
                code: "server_request_resolution_failed".to_string(),
                severity: Some("error".to_string()),
                message: Some(err),
                details: Some(json!({ "request_method": method })),
                transient: Some(false),
            },
        );
    }
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

fn translate_server_notification(
    session_state: &mut AppServerSessionState,
    notification: ServerNotification,
) -> Vec<(CrpChannel, CrpEvent)> {
    match notification {
        ServerNotification::ThreadTokenUsageUpdated(payload) => {
            session_state
                .turn_aliases
                .note_token_usage(payload.turn_id.as_str(), payload.token_usage);
            Vec::new()
        }
        ServerNotification::TurnStarted(payload) => {
            let _ = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn.id.as_str());
            Vec::new()
        }
        ServerNotification::TurnCompleted(payload) => {
            session_state
                .turn_aliases
                .note_terminal_turn(payload.turn.id.as_str());
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn translate_legacy_notification(
    session_state: &mut AppServerSessionState,
    notification: JSONRPCNotification,
) -> Result<Vec<(CrpChannel, CrpEvent)>, String> {
    let decoded = decode_legacy_notification(notification)?;
    if let Some(conversation_id) = decoded.conversation_id.as_deref()
        && conversation_id != session_state.thread_id
    {
        return Ok(Vec::new());
    }
    if matches!(decoded.event.msg, EventMsg::SessionConfigured(_)) {
        return Ok(Vec::new());
    }
    let mut translated = Vec::new();
    for (channel, event) in map_codex_event(&mut session_state.tracker, decoded.event) {
        translated.push((
            channel,
            rewrite_turn_ids(&mut session_state.turn_aliases, event),
        ));
    }
    Ok(translated)
}

fn rewrite_turn_ids(turn_aliases: &mut TurnAliasState, event: CrpEvent) -> CrpEvent {
    match event {
        CrpEvent::TurnStarted {
            session_id,
            turn_id,
        } => {
            let crp_turn_id = turn_aliases.ensure_crp_turn_id(turn_id.as_str());
            CrpEvent::TurnStarted {
                session_id,
                turn_id: crp_turn_id,
            }
        }
        CrpEvent::MessageDelta {
            session_id,
            turn_id,
            message_id,
            delta,
        } => CrpEvent::MessageDelta {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            message_id,
            delta,
        },
        CrpEvent::MessageFinal {
            session_id,
            turn_id,
            message_id,
            content,
        } => CrpEvent::MessageFinal {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            message_id,
            content,
        },
        CrpEvent::ReasoningSummary {
            session_id,
            turn_id,
            summary_index,
            text,
            item_id,
        } => CrpEvent::ReasoningSummary {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            summary_index,
            text,
            item_id,
        },
        CrpEvent::ReasoningTrace {
            session_id,
            turn_id,
            chunk,
            encoding,
            summary_index,
            item_id,
        } => CrpEvent::ReasoningTrace {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            chunk,
            encoding,
            summary_index,
            item_id,
        },
        CrpEvent::ReasoningTraceFinal {
            session_id,
            turn_id,
            content,
            encoding,
            summary_index,
            item_id,
        } => CrpEvent::ReasoningTraceFinal {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            content,
            encoding,
            summary_index,
            item_id,
        },
        CrpEvent::TurnCompleted {
            session_id,
            turn_id,
            status,
            context_window,
            error,
        } => {
            let context_window = context_window.or_else(|| {
                if matches!(status, CrpTurnStatus::Success) {
                    turn_aliases.take_context_window_for_app_turn(turn_id.as_str())
                } else {
                    None
                }
            });
            turn_aliases.note_terminal_turn(turn_id.as_str());
            CrpEvent::TurnCompleted {
                session_id,
                turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
                status,
                context_window,
                error,
            }
        }
        CrpEvent::ToolStarted {
            session_id,
            turn_id,
            tool_call_id,
            tool_name,
            tool_label,
            input,
            input_preview,
        } => CrpEvent::ToolStarted {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            tool_call_id,
            tool_name,
            tool_label,
            input,
            input_preview,
        },
        CrpEvent::ToolRequest {
            session_id,
            turn_id,
            tool_call_id,
            tool_name,
            input,
        } => CrpEvent::ToolRequest {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            tool_call_id,
            tool_name,
            input,
        },
        CrpEvent::ToolOutputDelta {
            session_id,
            turn_id,
            tool_call_id,
            stream,
            chunk,
        } => CrpEvent::ToolOutputDelta {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            tool_call_id,
            stream,
            chunk,
        },
        CrpEvent::ToolCompleted {
            session_id,
            turn_id,
            tool_call_id,
            tool_name,
            tool_label,
            status,
            output,
            error,
            input_preview,
        } => CrpEvent::ToolCompleted {
            session_id,
            turn_id: turn_aliases.ensure_crp_turn_id(turn_id.as_str()),
            tool_call_id,
            tool_name,
            tool_label,
            status,
            output,
            error,
            input_preview,
        },
        CrpEvent::SessionNotice {
            session_id,
            turn_id,
            code,
            severity,
            message,
            details,
            transient,
        } => CrpEvent::SessionNotice {
            session_id,
            turn_id: turn_id.map(|turn_id| turn_aliases.ensure_crp_turn_id(turn_id.as_str())),
            code,
            severity,
            message,
            details,
            transient,
        },
        other => other,
    }
}

fn canonical_context_window_from_thread_usage(token_usage: &ThreadTokenUsage) -> Option<Value> {
    let context_window_tokens = u64::try_from(token_usage.model_context_window?).ok()?;
    if context_window_tokens == 0 {
        return None;
    }
    let total_tokens = u64::try_from(token_usage.total.total_tokens).ok()?;
    let input_tokens = u64::try_from(token_usage.total.input_tokens).ok()?;
    let output_tokens = u64::try_from(token_usage.total.output_tokens).ok()?;
    let reasoning_output_tokens = u64::try_from(token_usage.total.reasoning_output_tokens).ok()?;
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

fn combined_cli_overrides(
    cli_kv_overrides: &[(String, TomlValue)],
    session_config: CrpSessionConfig,
) -> Vec<(String, TomlValue)> {
    let mut cli_overrides = cli_kv_overrides.to_vec();
    if let Some(mcp_servers) = session_config.mcp_servers {
        cli_overrides.extend(mcp_servers_to_cli_overrides(mcp_servers));
    }
    cli_overrides
}

async fn probe_models(config: &Config) -> (Vec<CrpModelInfo>, Option<String>, Option<String>) {
    let auth_manager = codex_core::AuthManager::shared(
        config.codex_home.clone(),
        true,
        config.cli_auth_credentials_store_mode,
    );
    let thread_manager = codex_core::ThreadManager::new(
        config,
        auth_manager,
        SessionSource::Exec,
        super::collaboration_modes_config(config),
    );
    let (presets, catalog_source) = thread_manager
        .list_models_with_catalog_source(
            codex_core::models_manager::manager::RefreshStrategy::Online,
        )
        .await;
    let models = build_crp_model_infos(&presets);
    let current_model_id = build_current_model_id(config, &presets);
    let catalog_source = match catalog_source {
        ModelCatalogSource::LiveRemote => Some("live_remote".to_string()),
        ModelCatalogSource::LocalCache => Some("local_cache".to_string()),
        ModelCatalogSource::LocalBundle => Some("local_bundle".to_string()),
        ModelCatalogSource::CustomCatalog => Some("custom_catalog".to_string()),
    };
    (models, current_model_id, catalog_source)
}

async fn send_request_with_response<T>(
    client: &InProcessAppServerClient,
    request: ClientRequest,
    method: &str,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    client.request_typed(request).await.map_err(|err| {
        if method.is_empty() {
            err.to_string()
        } else {
            format!("{method}: {err}")
        }
    })
}

async fn reject_server_request(
    client: &InProcessAppServerClient,
    request_id: RequestId,
    method: &str,
    reason: String,
) -> Result<(), String> {
    client
        .reject_server_request(
            request_id,
            JSONRPCErrorError {
                code: -32000,
                message: reason,
                data: None,
            },
        )
        .await
        .map_err(|err| format!("failed to reject `{method}` server request: {err}"))
}

fn server_request_method_name(request: &ServerRequest) -> String {
    serde_json::to_value(request)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_string())
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

fn sandbox_mode_from_policy(
    sandbox_policy: &codex_protocol::protocol::SandboxPolicy,
) -> Option<codex_app_server_protocol::SandboxMode> {
    match sandbox_policy {
        codex_protocol::protocol::SandboxPolicy::DangerFullAccess => {
            Some(codex_app_server_protocol::SandboxMode::DangerFullAccess)
        }
        codex_protocol::protocol::SandboxPolicy::ReadOnly { .. } => {
            Some(codex_app_server_protocol::SandboxMode::ReadOnly)
        }
        codex_protocol::protocol::SandboxPolicy::WorkspaceWrite { .. } => {
            Some(codex_app_server_protocol::SandboxMode::WorkspaceWrite)
        }
        codex_protocol::protocol::SandboxPolicy::ExternalSandbox { .. } => None,
    }
}

struct DecodedLegacyNotification {
    conversation_id: Option<String>,
    event: Event,
}

fn normalize_legacy_notification_method(method: &str) -> &str {
    method.strip_prefix("codex/event/").unwrap_or(method)
}

fn decode_legacy_notification(
    notification: JSONRPCNotification,
) -> Result<DecodedLegacyNotification, String> {
    let value = notification
        .params
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let method = notification.method;
    let normalized_method = normalize_legacy_notification_method(&method).to_string();
    let Value::Object(mut object) = value else {
        return Err(format!(
            "legacy notification `{method}` params were not an object"
        ));
    };
    let conversation_id = object
        .remove("conversationId")
        .and_then(|value| value.as_str().map(str::to_owned));
    let Some(msg_value) = object.get_mut("msg") else {
        return Err(format!(
            "legacy notification `{method}` missing msg payload"
        ));
    };
    let Value::Object(msg_object) = msg_value else {
        return Err(format!(
            "legacy notification `{method}` msg payload was not an object"
        ));
    };
    msg_object.insert("type".to_string(), Value::String(normalized_method));
    let event: Event = serde_json::from_value(Value::Object(object))
        .map_err(|err| format!("failed to decode legacy event `{method}`: {err}"))?;
    Ok(DecodedLegacyNotification {
        conversation_id,
        event,
    })
}

#[cfg(test)]
    mod app_server_fixture_tests {
        use super::*;
        use pretty_assertions::assert_eq;
        use std::collections::BTreeMap;
        use std::fs;
        use tokio::task::yield_now;

    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum FixtureStep {
        BindTurn {
            app_turn_id: String,
            crp_turn_id: String,
        },
        LegacyNotification {
            notification: JSONRPCNotification,
        },
        ServerNotification {
            notification: ServerNotification,
        },
        Lagged {
            skipped: usize,
        },
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct SnapshotEvent {
        channel: String,
        event: Value,
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct SnapshotState {
        active_app_turn_id: Option<String>,
        active_crp_turn_id: Option<String>,
        latest_token_usage: Option<ThreadTokenUsage>,
        turn_aliases: BTreeMap<String, String>,
    }

    #[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
    struct SnapshotOutput {
        events: Vec<SnapshotEvent>,
        state: SnapshotState,
    }

    fn testdata_path(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(file)
    }

    fn fixture_session_state() -> AppServerSessionState {
        AppServerSessionState {
            tracker: TurnTracker::new("fixture-session".to_string()),
            client: None,
            request_ids: RequestIdSequencer::new(),
            thread_id: "thr_fixture".to_string(),
            default_cwd: PathBuf::from("/tmp"),
            default_model: "gpt-5".to_string(),
            default_effort: Some(ReasoningEffort::Medium),
            default_summary: None,
            default_service_tier: None,
            default_approval_policy: AskForApproval::UnlessTrusted,
            default_sandbox_policy: SandboxPolicy::DangerFullAccess,
            opened_commands: Vec::new(),
            opened_slash_commands: Vec::new(),
            turn_aliases: TurnAliasState::new(),
        }
    }

    fn load_fixture_steps(file: &str) -> Vec<FixtureStep> {
        let input = fs::read_to_string(testdata_path(file)).expect("fixture should exist");
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<FixtureStep>(line).expect("fixture line should parse")
            })
            .collect()
    }

    fn replay_fixture(file: &str) -> SnapshotOutput {
        let mut session_state = fixture_session_state();
        let mut output = Vec::new();

        for step in load_fixture_steps(file) {
            let events = match step {
                FixtureStep::BindTurn {
                    app_turn_id,
                    crp_turn_id,
                } => {
                    session_state
                        .turn_aliases
                        .bind_turn_alias(app_turn_id, Some(crp_turn_id));
                    Vec::new()
                }
                FixtureStep::LegacyNotification { notification } => {
                    translate_legacy_notification(&mut session_state, notification)
                        .expect("legacy notification should translate")
                }
                FixtureStep::ServerNotification { notification } => {
                    translate_server_notification(&mut session_state, notification)
                }
                FixtureStep::Lagged { skipped } => vec![(
                    CrpChannel::Control,
                    CrpEvent::SessionGap {
                        session_id: session_state.tracker.session_id.clone(),
                        reason: Some(format!("app_server_lagged:{skipped}")),
                    },
                )],
            };

            for (channel, event) in events {
                output.push(SnapshotEvent {
                    channel: match channel {
                        CrpChannel::Control => "control".to_string(),
                        CrpChannel::Data => "data".to_string(),
                    },
                    event: serde_json::to_value(event).expect("event should serialize"),
                });
            }
        }

        SnapshotOutput {
            events: output,
            state: SnapshotState {
                active_app_turn_id: session_state.turn_aliases.active_app_turn_id.clone(),
                active_crp_turn_id: session_state.turn_aliases.active_crp_turn_id.clone(),
                latest_token_usage: session_state.turn_aliases.latest_token_usage.clone(),
                turn_aliases: session_state
                    .turn_aliases
                    .app_to_crp
                    .iter()
                    .map(|(app, crp)| (app.clone(), crp.clone()))
                    .collect(),
            },
        }
    }

    fn assert_snapshot(input: &str, expected: &str) {
        let actual = replay_fixture(input);
        let expected_value: SnapshotOutput = serde_json::from_str(
            &fs::read_to_string(testdata_path(expected)).expect("expected snapshot should exist"),
        )
        .expect("expected snapshot should parse");
        assert_eq!(actual, expected_value);
    }

    #[test]
    fn basic_message_fixture_matches_snapshot() {
        assert_snapshot(
            "app_server_basic_message.input.jsonl",
            "app_server_basic_message.expected.json",
        );
    }

    #[test]
    fn reasoning_and_parallel_tools_fixture_matches_snapshot() {
        assert_snapshot(
            "app_server_reasoning_parallel_tools.input.jsonl",
            "app_server_reasoning_parallel_tools.expected.json",
        );
    }

    #[test]
    fn error_and_token_usage_fixture_matches_snapshot() {
        assert_snapshot(
            "app_server_error_and_usage.input.jsonl",
            "app_server_error_and_usage.expected.json",
        );
    }

    #[tokio::test]
    async fn waits_for_first_command_before_session_exists() {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let mut session = None;
        let task = tokio::spawn(async move { next_runtime_input(&mut session, &mut cmd_rx).await });

        for _ in 0..3 {
            yield_now().await;
        }
        assert!(
            !task.is_finished(),
            "runtime input loop should stay pending while stdin remains open"
        );

        cmd_tx
            .send(RuntimeCommand::Parsed(CrpCommand::ModelsList { config: None }))
            .expect("command channel should remain open");

        let result = task.await.expect("task should complete");
        assert!(matches!(
            result,
            Some(RuntimeInput::Command(RuntimeCommand::Parsed(
                CrpCommand::ModelsList { .. }
            )))
        ));
    }
}
