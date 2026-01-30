#![deny(clippy::print_stdout)]

mod protocol;

use crate::protocol::CrpChannel;
use crate::protocol::CrpCommand;
use crate::protocol::CrpCommandEnvelope;
use crate::protocol::CrpEvent;
use crate::protocol::CrpEventEnvelope;
use crate::protocol::CrpModelInfo;
use crate::protocol::CrpMcpServerConfig;
use crate::protocol::CrpSessionConfig;
use crate::protocol::CrpToolOutputStream;
use crate::protocol::CrpToolStatus;
use crate::protocol::CrpTurnError;
use crate::protocol::CrpTurnStatus;
use async_trait::async_trait;
use base64::Engine;
use clap::Parser;
use codex_arg0::arg0_dispatch_or_else;
use codex_common::CliConfigOverrides;
use codex_core::AuthManager;
use codex_core::CodexThread;
use codex_core::FunctionCallError;
use codex_core::NewThread;
use codex_core::ThreadManager;
use codex_core::ToolHandler;
use codex_core::ToolInvocation;
use codex_core::ToolKind;
use codex_core::ToolOutput;
use codex_core::ToolPayload;
use codex_core::auth::enforce_login_restrictions;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::default_client::set_default_originator;
use codex_core::protocol::AskForApproval;
use codex_core::protocol::Event;
use codex_core::protocol::EventMsg;
use codex_core::protocol::ExecOutputStream;
use codex_core::protocol::Op;
use codex_core::protocol::SandboxPolicy;
use codex_core::protocol::Submission;
use codex_core::protocol::TurnAbortReason;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::TurnItem;
use codex_protocol::openai_models::{ModelPreset, ReasoningEffort};
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use mcp_types::CallToolResult;
use mcp_types::ContentBlock;
use mcp_types::TextContent;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use toml::Value as TomlValue;
use tracing::error;
use tracing::warn;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    #[clap(flatten)]
    config_overrides: CliConfigOverrides,
}

enum RuntimeCommand {
    Parsed(CrpCommand),
    ParseError { message: String },
}

const DATA_PLANE_BUFFER_CAPACITY: usize = 256;
const TOOL_OUTPUT_CHUNK_BYTES: usize = 8 * 1024;

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

    async fn send(&mut self, channel: CrpChannel, event: CrpEvent) -> anyhow::Result<()> {
        let envelope = CrpEventEnvelope {
            v: 1,
            seq: self.next_seq(),
            channel,
            event,
        };
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

    fn send_control(&self, event: CrpEvent) -> Result<(), CrpEvent> {
        self.control_tx.send(event).map_err(|err| err.0)
    }

    fn send_data(&self, event: CrpEvent) -> Result<(), CrpEvent> {
        match self.data_tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(err) => Err(match err {
                mpsc::error::TrySendError::Full(event) => event,
                mpsc::error::TrySendError::Closed(event) => event,
            }),
        }
    }
}

struct TurnTracker {
    session_id: String,
    turns: HashMap<String, TurnState>,
}

impl TurnTracker {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            turns: HashMap::new(),
        }
    }
}

struct SessionState {
    tracker: TurnTracker,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    default_cwd: PathBuf,
    default_model: String,
    default_effort: Option<ReasoningEffort>,
    default_summary: codex_protocol::config_types::ReasoningSummary,
    default_approval_policy: AskForApproval,
    default_sandbox_policy: SandboxPolicy,
}

struct TurnState {
    run_id: String,
    turn_id: String,
    message_id: Option<String>,
    reasoning_item_id: Option<String>,
    emitted_final: bool,
    completed: bool,
}

impl TurnState {
    fn new(turn_id: String) -> Self {
        Self {
            run_id: format!("run_{turn_id}"),
            turn_id,
            message_id: None,
            reasoning_item_id: None,
            emitted_final: false,
            completed: false,
        }
    }
}

struct ToolBridgeRequest {
    session_id: String,
    run_id: String,
    turn_id: String,
    tool_call_id: String,
    tool_name: String,
    input: Option<serde_json::Value>,
    respond_to: oneshot::Sender<ToolBridgeResult>,
}

#[derive(Clone, Debug)]
struct ToolBridgeResult {
    status: CrpToolStatus,
    output: Option<serde_json::Value>,
    error: Option<String>,
}

struct PendingToolRequest {
    session_id: String,
    run_id: String,
    turn_id: String,
    tool_name: String,
    respond_to: oneshot::Sender<ToolBridgeResult>,
}

#[allow(dead_code)]
struct ExternalToolHandler {
    session_id: Arc<RwLock<String>>,
    request_tx: mpsc::UnboundedSender<ToolBridgeRequest>,
}

#[allow(dead_code)]
impl ExternalToolHandler {
    fn new(
        session_id: Arc<RwLock<String>>,
        request_tx: mpsc::UnboundedSender<ToolBridgeRequest>,
    ) -> Self {
        Self {
            session_id,
            request_tx,
        }
    }
}

#[async_trait]
impl ToolHandler for ExternalToolHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, _payload: &ToolPayload) -> bool {
        true
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let turn_id = invocation.turn_id().to_string();
        let ToolInvocation {
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;

        let session_id = {
            let guard = self.session_id.read().await;
            guard.clone()
        };
        let run_id = format!("run_{turn_id}");
        let tool_name = tool_name_for_payload(&tool_name, &payload);
        let input = tool_payload_to_value(&payload);

        let (respond_to, receiver) = oneshot::channel();
        let request = ToolBridgeRequest {
            session_id,
            run_id,
            turn_id,
            tool_call_id: call_id,
            tool_name,
            input,
            respond_to,
        };

        self.request_tx
            .send(request)
            .map_err(|_| FunctionCallError::Fatal("tool bridge unavailable".to_string()))?;

        let result = receiver
            .await
            .map_err(|_| FunctionCallError::Fatal("tool bridge response dropped".to_string()))?;

        tool_output_from_result(result, &payload)
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    arg0_dispatch_or_else(|codex_linux_sandbox_exe| async move {
        run_main(cli, codex_linux_sandbox_exe).await
    })
}

async fn run_main(cli: Cli, codex_linux_sandbox_exe: Option<PathBuf>) -> anyhow::Result<()> {
    if let Err(err) = set_default_originator("codex_crp".to_string()) {
        warn!(?err, "Failed to set codex CRP originator override");
    }

    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("error"))
        .unwrap_or_else(|_| EnvFilter::new("error"));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(env_filter);
    let _ = tracing_subscriber::registry().with(fmt_layer).try_init();

    let cli_kv_overrides = cli
        .config_overrides
        .parse_overrides()
        .map_err(|err| anyhow::anyhow!(err))?;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(read_commands(cmd_tx));

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (tool_request_tx, mut tool_request_rx) = mpsc::unbounded_channel();

    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (data_tx, data_rx) = mpsc::channel(DATA_PLANE_BUFFER_CAPACITY);
    let router = CrpEventRouter::new(control_tx, data_tx);

    let writer = CrpWriter::new();
    tokio::spawn(async move {
        if let Err(err) = run_writer(writer, control_rx, data_rx).await {
            error!(?err, "crp writer task failed");
        }
    });
    let mut session: Option<SessionState> = None;
    let mut pending_tool_requests = HashMap::new();

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    RuntimeCommand::ParseError { message } => {
                        error!(%message, "Failed to parse CRP command");
                    }
                    RuntimeCommand::Parsed(command) => {
                        match command {
                            CrpCommand::ToolResult {
                                session_id,
                                turn_id,
                                tool_call_id,
                                status,
                                output,
                                error,
                            } => {
                                handle_tool_result(
                                    ToolBridgeResult { status, output, error },
                                    ToolResultCommand {
                                        session_id,
                                        turn_id,
                                        tool_call_id,
                                    },
                                    &router,
                                    &mut pending_tool_requests,
                                );
                            }
                            other => {
                                handle_command(
                                    other,
                                    &mut session,
                                    &router,
                                    &event_tx,
                                    &cli_kv_overrides,
                                    codex_linux_sandbox_exe.clone(),
                                    &tool_request_tx,
                                ).await?;
                            }
                        }
                    }
                }
            }
            Some(event) = event_rx.recv() => {
                if let Some(session_state) = session.as_mut() {
                    let events = map_codex_event(&mut session_state.tracker, event);
                    for (channel, ev) in events {
                        dispatch_event(&router, channel, ev);
                    }
                }
            }
            Some(request) = tool_request_rx.recv() => {
                handle_tool_request(request, &router, &mut pending_tool_requests);
            }
            else => {
                break;
            }
        }
    }

    Ok(())
}

async fn read_commands(tx: mpsc::UnboundedSender<RuntimeCommand>) {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<CrpCommandEnvelope>(trimmed) {
                    Ok(envelope) => {
                        if let Some(v) = envelope.v
                            && v != 1
                        {
                            warn!(%v, "Unexpected CRP version");
                        }
                        if tx.send(RuntimeCommand::Parsed(envelope.command)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let message = format!("invalid command: {err}");
                        if tx.send(RuntimeCommand::ParseError { message }).is_err() {
                            break;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                let message = format!("failed to read stdin: {err}");
                let _ = tx.send(RuntimeCommand::ParseError { message });
                break;
            }
        }
    }
}

async fn handle_command(
    command: CrpCommand,
    session: &mut Option<SessionState>,
    router: &CrpEventRouter,
    event_tx: &mpsc::UnboundedSender<Event>,
    cli_kv_overrides: &[(String, toml::Value)],
    codex_linux_sandbox_exe: Option<PathBuf>,
    _tool_request_tx: &mpsc::UnboundedSender<ToolBridgeRequest>,
) -> anyhow::Result<()> {
    match command {
        CrpCommand::SessionOpen { session_id, config } => {
            if session.is_some() {
                warn!("session.open ignored: session already active");
                return Ok(());
            }

            let config = config.unwrap_or(CrpSessionConfig {
                cwd: None,
                model: None,
                model_provider: None,
                approval_policy: None,
                sandbox_mode: None,
                reasoning_trace_enabled: None,
                mcp_servers: None,
            });

            let state = open_session(config, cli_kv_overrides, codex_linux_sandbox_exe).await?;
            let provider_session_id = state.thread_id.to_string();
            let session_id = session_id.unwrap_or_else(|| provider_session_id.clone());

            if router
                .send_control(CrpEvent::SessionOpened {
                    session_id: session_id.clone(),
                    provider_session_id: Some(provider_session_id),
                })
                .is_err()
            {
                warn!("failed to send session.opened event");
            }

            let thread = Arc::clone(&state.thread);
            let tx = event_tx.clone();
            tokio::spawn(async move {
                loop {
                    match thread.next_event().await {
                        Ok(event) => {
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            error!(?err, "codex event stream failed");
                            break;
                        }
                    }
                }
            });

            let mut state = state;
            state.tracker.session_id = session_id;
            *session = Some(state);
        }
        CrpCommand::SessionPrompt {
            session_id,
            turn_id,
            prompt,
            items,
            model,
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
                (None, Some(prompt)) => vec![UserInput::Text { text: prompt }],
                (None, None) => {
                    warn!("session.prompt ignored: missing prompt or items");
                    return Ok(());
                }
            };

            let cwd = cwd.unwrap_or_else(|| session_state.default_cwd.clone());
            let model = model.unwrap_or_else(|| session_state.default_model.clone());
            let (model, effort_override) = split_model_and_effort(&model);
            let effort = effort_override.or(session_state.default_effort);

            let op = Op::UserTurn {
                items,
                cwd,
                approval_policy: session_state.default_approval_policy,
                sandbox_policy: session_state.default_sandbox_policy.clone(),
                model,
                effort,
                summary: session_state.default_summary,
                final_output_json_schema: None,
            };

            let sub_id = if let Some(turn_id) = turn_id {
                session_state
                    .thread
                    .submit_with_id(Submission {
                        id: turn_id.clone(),
                        op,
                    })
                    .await?;
                turn_id
            } else {
                session_state.thread.submit(op).await?
            };

            session_state
                .tracker
                .turns
                .entry(sub_id.clone())
                .or_insert_with(|| TurnState::new(sub_id));
        }
        CrpCommand::ModelsList { config } => {
            let config = config.unwrap_or(CrpSessionConfig {
                cwd: None,
                model: None,
                model_provider: None,
                approval_policy: None,
                sandbox_mode: None,
                reasoning_trace_enabled: None,
                mcp_servers: None,
            });

            let config =
                load_config_from_crp(config, cli_kv_overrides, codex_linux_sandbox_exe).await?;
            let auth_manager = AuthManager::shared(
                config.codex_home.clone(),
                true,
                config.cli_auth_credentials_store_mode,
            );
            let thread_manager =
                ThreadManager::new(config.codex_home.clone(), auth_manager, SessionSource::Exec);
            let presets = thread_manager.list_models(&config).await;
            let models = build_crp_model_infos(&presets);
            let current_model_id = build_current_model_id(&config, &presets);

            if router
                .send_control(CrpEvent::ModelsList {
                    models,
                    current_model_id,
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
            let Some(session_state) = session.as_ref() else {
                warn!("session.cancel ignored: no active session");
                return Ok(());
            };
            if let Some(turn_id) = turn_id {
                warn!(%turn_id, "session.cancel ignores turn_id for now");
            }
            if let Some(expected) = session_id.as_deref()
                && expected != session_state.tracker.session_id
            {
                warn!(%expected, "session.cancel ignored: session_id mismatch");
                return Ok(());
            }

            session_state.thread.submit(Op::Interrupt).await?;
        }
        CrpCommand::ToolResult { .. } => {
            warn!("tool.result ignored: handled in runtime loop");
        }
    }

    Ok(())
}

async fn load_config_from_crp(
    session_config: CrpSessionConfig,
    cli_kv_overrides: &[(String, toml::Value)],
    codex_linux_sandbox_exe: Option<PathBuf>,
) -> anyhow::Result<Config> {
    let (model_override, effort_override) = session_config
        .model
        .as_deref()
        .map(split_model_and_effort)
        .map(|(model, effort)| (Some(model), effort))
        .unwrap_or((None, None));
    let overrides = ConfigOverrides {
        model: model_override,
        review_model: None,
        config_profile: None,
        approval_policy: session_config.approval_policy,
        sandbox_mode: session_config.sandbox_mode,
        cwd: session_config.cwd,
        model_provider: session_config.model_provider,
        codex_linux_sandbox_exe,
        base_instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        include_apply_patch_tool: None,
        show_raw_agent_reasoning: session_config.reasoning_trace_enabled,
        tools_web_search_request: None,
        additional_writable_roots: Vec::new(),
    };

    let mut cli_overrides = cli_kv_overrides.to_vec();
    if let Some(mcp_servers) = session_config.mcp_servers {
        cli_overrides.extend(mcp_servers_to_cli_overrides(mcp_servers));
    }

    let mut config =
        Config::load_with_cli_overrides_and_harness_overrides(cli_overrides, overrides).await?;
    if let Some(effort) = effort_override {
        config.model_reasoning_effort = Some(effort);
    }

    if let Err(err) = enforce_login_restrictions(&config) {
        return Err(anyhow::anyhow!(err));
    }

    Ok(config)
}

fn split_model_and_effort(model: &str) -> (String, Option<ReasoningEffort>) {
    let Some((base, effort_str)) = model.rsplit_once('/') else {
        return (model.to_string(), None);
    };
    if base.is_empty() {
        return (model.to_string(), None);
    }
    let effort = match effort_str {
        "none" => Some(ReasoningEffort::None),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::XHigh),
        _ => None,
    };
    if effort.is_some() {
        (base.to_string(), effort)
    } else {
        (model.to_string(), None)
    }
}

async fn open_session(
    session_config: CrpSessionConfig,
    cli_kv_overrides: &[(String, toml::Value)],
    codex_linux_sandbox_exe: Option<PathBuf>,
) -> anyhow::Result<SessionState> {
    let config =
        load_config_from_crp(session_config, cli_kv_overrides, codex_linux_sandbox_exe).await?;

    let auth_manager = AuthManager::shared(
        config.codex_home.clone(),
        true,
        config.cli_auth_credentials_store_mode,
    );
    let thread_manager =
        ThreadManager::new(config.codex_home.clone(), auth_manager, SessionSource::Exec);
    let default_model = thread_manager
        .get_models_manager()
        .get_model(&config.model, &config)
        .await;

    let NewThread {
        thread_id,
        thread,
        session_configured: _,
    } = thread_manager.start_thread(config.clone()).await?;

    Ok(SessionState {
        tracker: TurnTracker::new(String::new()),
        thread_id,
        thread,
        default_cwd: config.cwd.to_path_buf(),
        default_model,
        default_effort: config.model_reasoning_effort,
        default_summary: config.model_reasoning_summary,
        default_approval_policy: config.approval_policy.value(),
        default_sandbox_policy: config.sandbox_policy.get().clone(),
    })
}

fn build_crp_model_infos(presets: &[ModelPreset]) -> Vec<CrpModelInfo> {
    let mut out = Vec::new();
    for preset in presets {
        if !preset.show_in_picker {
            continue;
        }
        if preset.supported_reasoning_efforts.len() >= 2 {
            let mut seen = HashSet::new();
            for effort in &preset.supported_reasoning_efforts {
                let effort_id = effort.effort.to_string();
                if !seen.insert(effort_id.clone()) {
                    continue;
                }
                let id = format!("{}/{}", preset.id, effort_id);
                let name = format!("{} ({})", preset.display_name, effort_id);
                out.push(CrpModelInfo {
                    id,
                    name: Some(name),
                });
            }
        } else {
            out.push(CrpModelInfo {
                id: preset.id.clone(),
                name: Some(preset.display_name.clone()),
            });
        }
    }
    out
}

fn build_current_model_id(config: &Config, presets: &[ModelPreset]) -> Option<String> {
    let Some(model_raw) = config.model.as_deref() else {
        return None;
    };
    let model = model_raw.trim();
    if model.is_empty() {
        return None;
    }
    if model.contains('/') {
        return Some(model.to_string());
    }
    let preset = presets
        .iter()
        .find(|p| p.id == model || p.model == model);
    if let Some(preset) = preset {
        if preset.supported_reasoning_efforts.len() >= 2 {
            let desired = config
                .model_reasoning_effort
                .unwrap_or(preset.default_reasoning_effort);
            let supported = preset
                .supported_reasoning_efforts
                .iter()
                .any(|p| p.effort == desired);
            let effort = if supported {
                desired
            } else {
                preset.default_reasoning_effort
            };
            return Some(format!("{}/{}", model, effort.to_string()));
        }
    }
    Some(model.to_string())
}

fn mcp_servers_to_cli_overrides(
    mcp_servers: HashMap<String, CrpMcpServerConfig>,
) -> Vec<(String, TomlValue)> {
    mcp_servers
        .into_iter()
        .filter_map(|(name, config)| match mcp_server_to_toml(config) {
            Some(value) => Some((format!("mcp_servers.{name}"), value)),
            None => {
                warn!(%name, "mcp server missing transport config");
                None
            }
        })
        .collect()
}

fn mcp_server_to_toml(config: CrpMcpServerConfig) -> Option<TomlValue> {
    let mut table = toml::value::Table::new();

    if let Some(timeout) = config.tool_timeout_sec {
        table.insert("tool_timeout_sec".to_string(), TomlValue::Float(timeout));
    }

    if let Some(enabled_tools) = config.enabled_tools {
        table.insert(
            "enabled_tools".to_string(),
            TomlValue::Array(enabled_tools.into_iter().map(TomlValue::String).collect()),
        );
    }

    if let Some(disabled_tools) = config.disabled_tools {
        table.insert(
            "disabled_tools".to_string(),
            TomlValue::Array(disabled_tools.into_iter().map(TomlValue::String).collect()),
        );
    }

    if let Some(command) = config.command {
        table.insert("command".to_string(), TomlValue::String(command));
        if let Some(args) = config.args
            && !args.is_empty()
        {
            table.insert(
                "args".to_string(),
                TomlValue::Array(args.into_iter().map(TomlValue::String).collect()),
            );
        }
        if let Some(env) = config.env
            && !env.is_empty()
        {
            table.insert(
                "env".to_string(),
                TomlValue::Table(string_map_to_toml_table(env)),
            );
        }
        if let Some(env_vars) = config.env_vars
            && !env_vars.is_empty()
        {
            table.insert(
                "env_vars".to_string(),
                TomlValue::Array(env_vars.into_iter().map(TomlValue::String).collect()),
            );
        }
        if let Some(cwd) = config.cwd {
            table.insert(
                "cwd".to_string(),
                TomlValue::String(cwd.to_string_lossy().to_string()),
            );
        }
    } else if let Some(url) = config.url {
        table.insert("url".to_string(), TomlValue::String(url));

        let http_headers = config.http_headers.unwrap_or_default();

        if !http_headers.is_empty() {
            table.insert(
                "http_headers".to_string(),
                TomlValue::Table(string_map_to_toml_table(http_headers)),
            );
        }

        if let Some(env_http_headers) = config.env_http_headers
            && !env_http_headers.is_empty()
        {
            table.insert(
                "env_http_headers".to_string(),
                TomlValue::Table(string_map_to_toml_table(env_http_headers)),
            );
        }
    } else {
        return None;
    }

    Some(TomlValue::Table(table))
}

fn string_map_to_toml_table(values: HashMap<String, String>) -> toml::value::Table {
    let mut table = toml::value::Table::new();
    for (key, value) in values {
        table.insert(key, TomlValue::String(value));
    }
    table
}

async fn run_writer(
    mut writer: CrpWriter,
    mut control_rx: mpsc::UnboundedReceiver<CrpEvent>,
    mut data_rx: mpsc::Receiver<CrpEvent>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            biased;
            Some(event) = control_rx.recv() => {
                writer.send(CrpChannel::Control, event).await?;
            }
            Some(event) = data_rx.recv() => {
                writer.send(CrpChannel::Data, event).await?;
            }
            else => break,
        }
    }
    Ok(())
}

fn dispatch_event(router: &CrpEventRouter, channel: CrpChannel, event: CrpEvent) {
    match channel {
        CrpChannel::Control => {
            if router.send_control(event).is_err() {
                warn!("failed to dispatch control event");
            }
        }
        CrpChannel::Data => {
            if let Err(event) = router.send_data(event)
                && let Some(session_id) = event_session_id(&event)
            {
                let _ = router.send_control(CrpEvent::SessionGap {
                    session_id: session_id.to_string(),
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
        | CrpEvent::ToolOutputDelta { session_id, .. } => Some(session_id),
        _ => None,
    }
}

fn handle_tool_request(
    request: ToolBridgeRequest,
    router: &CrpEventRouter,
    pending: &mut HashMap<String, PendingToolRequest>,
) {
    let ToolBridgeRequest {
        session_id,
        run_id,
        turn_id,
        tool_call_id,
        tool_name,
        input,
        respond_to,
    } = request;

    let _ = router.send_control(CrpEvent::ToolRequest {
        session_id: session_id.clone(),
        run_id: run_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.clone(),
        input: input.clone(),
    });
    let _ = router.send_control(CrpEvent::ToolStarted {
        session_id: session_id.clone(),
        run_id: run_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.clone(),
        input,
    });

    if pending
        .insert(
            tool_call_id.clone(),
            PendingToolRequest {
                session_id,
                run_id,
                turn_id,
                tool_name,
                respond_to,
            },
        )
        .is_some()
    {
        warn!(%tool_call_id, "overwriting pending tool request");
    }
}

struct ToolResultCommand {
    session_id: Option<String>,
    turn_id: Option<String>,
    tool_call_id: String,
}

fn handle_tool_result(
    result: ToolBridgeResult,
    command: ToolResultCommand,
    router: &CrpEventRouter,
    pending: &mut HashMap<String, PendingToolRequest>,
) {
    let Some(pending_request) = pending.remove(command.tool_call_id.as_str()) else {
        warn!(tool_call_id = %command.tool_call_id, "tool.result without pending request");
        return;
    };

    if let Some(session_id) = command.session_id.as_deref()
        && session_id != pending_request.session_id
    {
        warn!(
            expected = %pending_request.session_id,
            received = %session_id,
            "tool.result session_id mismatch"
        );
    }

    if let Some(turn_id) = command.turn_id.as_deref()
        && turn_id != pending_request.turn_id
    {
        warn!(
            expected = %pending_request.turn_id,
            received = %turn_id,
            "tool.result turn_id mismatch"
        );
    }

    if let Some(output) = tool_result_output_text(&result) {
        for chunk in chunk_output(&output, TOOL_OUTPUT_CHUNK_BYTES) {
            if chunk.is_empty() {
                continue;
            }
            dispatch_event(
                router,
                CrpChannel::Data,
                CrpEvent::ToolOutputDelta {
                    session_id: pending_request.session_id.clone(),
                    run_id: pending_request.run_id.clone(),
                    turn_id: pending_request.turn_id.clone(),
                    tool_call_id: command.tool_call_id.clone(),
                    stream: None,
                    chunk,
                },
            );
        }
    }

    dispatch_event(
        router,
        CrpChannel::Control,
        CrpEvent::ToolCompleted {
            session_id: pending_request.session_id.clone(),
            run_id: pending_request.run_id.clone(),
            turn_id: pending_request.turn_id.clone(),
            tool_call_id: command.tool_call_id.clone(),
            tool_name: pending_request.tool_name,
            status: result.status.clone(),
            output: result.output.clone(),
            error: result.error.clone(),
        },
    );

    let _ = pending_request.respond_to.send(result);
}

#[allow(dead_code)]
fn tool_name_for_payload(tool_name: &str, payload: &ToolPayload) -> String {
    match payload {
        ToolPayload::Mcp { server, tool, .. } => format!("mcp.{server}.{tool}"),
        _ => tool_name.to_string(),
    }
}

#[allow(dead_code)]
fn tool_payload_to_value(payload: &ToolPayload) -> Option<serde_json::Value> {
    match payload {
        ToolPayload::Function { arguments } => serde_json::from_str(arguments)
            .ok()
            .or_else(|| Some(json!({ "raw": arguments }))),
        ToolPayload::Custom { input } => Some(json!({ "input": input })),
        ToolPayload::LocalShell { params } => Some(json!({
            "command": params.command.clone(),
            "workdir": params.workdir.clone(),
            "timeout_ms": params.timeout_ms,
            "sandbox_permissions": params.sandbox_permissions,
            "justification": params.justification.clone(),
        })),
        ToolPayload::Mcp {
            server,
            tool,
            raw_arguments,
        } => {
            let arguments = serde_json::from_str(raw_arguments)
                .ok()
                .unwrap_or_else(|| json!({ "raw": raw_arguments }));
            Some(json!({
                "server": server,
                "tool": tool,
                "arguments": arguments,
            }))
        }
    }
}

#[allow(dead_code)]
fn tool_output_from_result(
    result: ToolBridgeResult,
    payload: &ToolPayload,
) -> Result<ToolOutput, FunctionCallError> {
    let ToolBridgeResult {
        status,
        output,
        error,
    } = result;
    let success = matches!(status, CrpToolStatus::Success);
    let output_text = output_value_to_string(output.as_ref());
    match payload {
        ToolPayload::Mcp { .. } => {
            if success {
                let call_tool_result = call_tool_result_from_value(output);
                Ok(ToolOutput::Mcp {
                    result: Ok(call_tool_result),
                })
            } else {
                let message = error
                    .or_else(|| output_text.clone())
                    .unwrap_or_else(|| "tool error".to_string());
                Ok(ToolOutput::Mcp {
                    result: Err(message),
                })
            }
        }
        _ => {
            let content = output_text.or_else(|| error.clone()).unwrap_or_default();
            Ok(ToolOutput::Function {
                content,
                content_items: None,
                success: Some(success),
            })
        }
    }
}

#[allow(dead_code)]
fn call_tool_result_from_value(value: Option<serde_json::Value>) -> CallToolResult {
    if let Some(value) = value {
        if let Ok(result) = serde_json::from_value::<CallToolResult>(value.clone()) {
            return result;
        }
        let text = match value {
            serde_json::Value::String(text) => text,
            _ => value.to_string(),
        };
        return CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent {
                annotations: None,
                text,
                r#type: "text".to_string(),
            })],
            is_error: None,
            structured_content: None,
        };
    }

    CallToolResult {
        content: Vec::new(),
        is_error: None,
        structured_content: None,
    }
}

fn tool_result_output_text(result: &ToolBridgeResult) -> Option<String> {
    output_value_to_string(result.output.as_ref()).or_else(|| result.error.clone())
}

fn output_value_to_string(output: Option<&serde_json::Value>) -> Option<String> {
    output.map(|value| match value {
        serde_json::Value::String(text) => text.clone(),
        _ => value.to_string(),
    })
}

fn chunk_output(output: &str, max_bytes: usize) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }
    if output.len() <= max_bytes {
        return vec![output.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let bytes = output.len();

    while start < bytes {
        let mut end = std::cmp::min(start + max_bytes, bytes);
        while end > start && !output.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            break;
        }
        chunks.push(output[start..end].to_string());
        start = end;
    }

    chunks
}

fn map_codex_event(tracker: &mut TurnTracker, event: Event) -> Vec<(CrpChannel, CrpEvent)> {
    let session_id = tracker.session_id.clone();
    match event.msg {
        EventMsg::TurnStarted(_) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Control,
                CrpEvent::TurnStarted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                },
            )]
        }
        EventMsg::AgentMessageContentDelta(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            turn.message_id = Some(ev.item_id.clone());
            vec![(
                CrpChannel::Data,
                CrpEvent::MessageDelta {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    message_id: ev.item_id,
                    delta: ev.delta,
                },
            )]
        }
        EventMsg::AgentMessageDelta(_) => Vec::new(),
        EventMsg::ReasoningContentDelta(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let item_id = if ev.item_id.is_empty() {
                None
            } else {
                Some(ev.item_id.clone())
            };
            turn.reasoning_item_id = item_id.clone();
            vec![(
                CrpChannel::Control,
                CrpEvent::ReasoningSummary {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    text: ev.delta,
                    item_id,
                },
            )]
        }
        EventMsg::ReasoningRawContentDelta(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            if turn.reasoning_item_id.is_none() && !ev.item_id.is_empty() {
                turn.reasoning_item_id = Some(ev.item_id.clone());
            }
            vec![(
                CrpChannel::Data,
                CrpEvent::ReasoningTrace {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    chunk: ev.delta,
                    encoding: None,
                },
            )]
        }
        EventMsg::ItemCompleted(ev) => match ev.item {
            TurnItem::AgentMessage(item) => {
                let turn = ensure_turn(tracker, &event.id);
                let message_id = item.id.clone();
                let content = agent_message_text(&item);
                turn.message_id = Some(message_id.clone());
                turn.emitted_final = true;
                vec![(
                    CrpChannel::Control,
                    CrpEvent::MessageFinal {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        message_id,
                        content,
                    },
                )]
            }
            _ => Vec::new(),
        },
        EventMsg::AgentMessage(_) => Vec::new(),
        EventMsg::AgentReasoningSectionBreak(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            turn.reasoning_item_id = if ev.item_id.is_empty() {
                None
            } else {
                Some(ev.item_id)
            };
            Vec::new()
        }
        EventMsg::AgentReasoningDelta(_) => Vec::new(),
        EventMsg::AgentReasoning(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Control,
                CrpEvent::ReasoningSummary {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    text: ev.text,
                    item_id: turn.reasoning_item_id.clone(),
                },
            )]
        }
        EventMsg::AgentReasoningRawContentDelta(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Data,
                CrpEvent::ReasoningTrace {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    chunk: ev.delta,
                    encoding: None,
                },
            )]
        }
        EventMsg::AgentReasoningRawContent(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Data,
                CrpEvent::ReasoningTrace {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    chunk: ev.text,
                    encoding: None,
                },
            )]
        }
        EventMsg::ExecCommandBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let input = json!({
                "command": ev.command,
                "cwd": ev.cwd,
                "source": ev.source,
                "interaction_input": ev.interaction_input,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "exec".to_string(),
                    input: Some(input),
                },
            )]
        }
        EventMsg::ExecCommandOutputDelta(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let stream = match ev.stream {
                ExecOutputStream::Stdout => CrpToolOutputStream::Stdout,
                ExecOutputStream::Stderr => CrpToolOutputStream::Stderr,
            };
            let chunk = base64::engine::general_purpose::STANDARD.encode(&ev.chunk);
            vec![(
                CrpChannel::Data,
                CrpEvent::ToolOutputDelta {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    stream: Some(stream),
                    chunk,
                },
            )]
        }
        EventMsg::TerminalInteraction(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let chunk = base64::engine::general_purpose::STANDARD.encode(ev.stdin.as_bytes());
            vec![(
                CrpChannel::Data,
                CrpEvent::ToolOutputDelta {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    stream: Some(CrpToolOutputStream::Stdin),
                    chunk,
                },
            )]
        }
        EventMsg::ExecCommandEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let exit_code = ev.exit_code;
            let status = if exit_code == 0 {
                CrpToolStatus::Success
            } else {
                CrpToolStatus::Error
            };
            let error = if exit_code == 0 {
                None
            } else {
                Some(format!("exit_code: {exit_code}"))
            };
            let output = json!({
                "stdout": ev.stdout,
                "stderr": ev.stderr,
                "aggregated_output": ev.aggregated_output,
                "formatted_output": ev.formatted_output,
                "exit_code": ev.exit_code,
                "duration_ms": ev.duration.as_millis(),
                "command": ev.command,
                "cwd": ev.cwd,
                "source": ev.source,
                "process_id": ev.process_id,
                "interaction_input": ev.interaction_input,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "exec".to_string(),
                    status,
                    output: Some(output),
                    error,
                },
            )]
        }
        EventMsg::PatchApplyBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let input = json!({
                "auto_approved": ev.auto_approved,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "apply_patch".to_string(),
                    input: Some(input),
                },
            )]
        }
        EventMsg::PatchApplyEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let status = if ev.success {
                CrpToolStatus::Success
            } else {
                CrpToolStatus::Error
            };
            let error = if ev.success {
                None
            } else {
                Some("apply_patch_failed".to_string())
            };
            let output = json!({
                "stdout": ev.stdout,
                "stderr": ev.stderr,
                "success": ev.success,
                "changes": serde_json::to_value(&ev.changes).ok(),
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "apply_patch".to_string(),
                    status,
                    output: Some(output),
                    error,
                },
            )]
        }
        EventMsg::WebSearchBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "web_search".to_string(),
                    input: None,
                },
            )]
        }
        EventMsg::WebSearchEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let output = json!({
                "query": ev.query,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "web_search".to_string(),
                    status: CrpToolStatus::Success,
                    output: Some(output),
                    error: None,
                },
            )]
        }
        EventMsg::McpToolCallBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let invocation = ev.invocation;
            let server = invocation.server;
            let tool = invocation.tool;
            let tool_name = format!("mcp.{server}.{tool}");
            let input = json!({
                "server": server,
                "tool": tool,
                "arguments": invocation.arguments,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name,
                    input: Some(input),
                },
            )]
        }
        EventMsg::McpToolCallEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let invocation = ev.invocation;
            let server = invocation.server;
            let tool = invocation.tool;
            let tool_name = format!("mcp.{server}.{tool}");
            let duration_ms = ev.duration.as_millis();
            let (status, error, output) = match ev.result {
                Ok(result) => (
                    CrpToolStatus::Success,
                    None,
                    Some(json!({
                        "duration_ms": duration_ms,
                        "result": result,
                    })),
                ),
                Err(message) => (
                    CrpToolStatus::Error,
                    Some(message),
                    Some(json!({
                        "duration_ms": duration_ms,
                    })),
                ),
            };
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name,
                    status,
                    output,
                    error,
                },
            )]
        }
        EventMsg::ViewImageToolCall(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let payload = json!({
                "path": ev.path,
            });
            vec![
                (
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id: session_id.clone(),
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        tool_call_id: ev.call_id.clone(),
                        tool_name: "view_image".to_string(),
                        input: Some(payload.clone()),
                    },
                ),
                (
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        tool_call_id: ev.call_id,
                        tool_name: "view_image".to_string(),
                        status: CrpToolStatus::Success,
                        output: Some(payload),
                        error: None,
                    },
                ),
            ]
        }
        EventMsg::TurnComplete(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let mut events = Vec::new();
            if let Some(last_message) = ev.last_agent_message
                && !turn.emitted_final
            {
                let message_id = ensure_message_id(turn);
                turn.emitted_final = true;
                events.push((
                    CrpChannel::Control,
                    CrpEvent::MessageFinal {
                        session_id: session_id.clone(),
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        message_id,
                        content: last_message,
                    },
                ));
            }
            if mark_completed(turn) {
                events.push((
                    CrpChannel::Control,
                    CrpEvent::TurnCompleted {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        status: CrpTurnStatus::Success,
                        error: None,
                    },
                ));
            }
            events
        }
        EventMsg::TurnAborted(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            if mark_completed(turn) {
                let status = match ev.reason {
                    TurnAbortReason::Interrupted => CrpTurnStatus::Interrupted,
                    TurnAbortReason::Replaced | TurnAbortReason::ReviewEnded => {
                        CrpTurnStatus::Canceled
                    }
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::TurnCompleted {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        status,
                        error: None,
                    },
                )]
            } else {
                Vec::new()
            }
        }
        EventMsg::Error(err) => {
            let turn = ensure_turn(tracker, &event.id);
            if mark_completed(turn) {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::TurnCompleted {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        status: CrpTurnStatus::Error,
                        error: Some(CrpTurnError {
                            message: err.to_string(),
                            kind: Some("error".to_string()),
                        }),
                    },
                )]
            } else {
                Vec::new()
            }
        }
        EventMsg::StreamError(err) => {
            let turn = ensure_turn(tracker, &event.id);
            if mark_completed(turn) {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::TurnCompleted {
                        session_id,
                        run_id: turn.run_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        status: CrpTurnStatus::Error,
                        error: Some(CrpTurnError {
                            message: err.to_string(),
                            kind: Some("stream_error".to_string()),
                        }),
                    },
                )]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn ensure_turn<'a>(tracker: &'a mut TurnTracker, turn_id: &str) -> &'a mut TurnState {
    tracker
        .turns
        .entry(turn_id.to_string())
        .or_insert_with(|| TurnState::new(turn_id.to_string()))
}

fn ensure_message_id(turn: &mut TurnState) -> String {
    if let Some(message_id) = turn.message_id.as_ref() {
        return message_id.clone();
    }
    let turn_id = &turn.turn_id;
    let message_id = format!("message_{turn_id}");
    turn.message_id = Some(message_id.clone());
    message_id
}

fn mark_completed(turn: &mut TurnState) -> bool {
    if turn.completed {
        return false;
    }
    turn.completed = true;
    true
}

fn agent_message_text(item: &AgentMessageItem) -> String {
    item.content
        .iter()
        .map(|content| match content {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use codex_core::protocol::AgentMessageContentDeltaEvent;
    use codex_core::protocol::ExecCommandBeginEvent;
    use codex_core::protocol::ExecCommandEndEvent;
    use codex_core::protocol::ExecCommandOutputDeltaEvent;
    use codex_core::protocol::ExecCommandSource;
    use codex_core::protocol::ReasoningContentDeltaEvent;
    use codex_core::protocol::ReasoningRawContentDeltaEvent;
    use codex_core::protocol::TurnCompleteEvent;
    use codex_core::protocol::TurnStartedEvent;
    use codex_protocol::parse_command::ParsedCommand;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn crp_mapping_smoke_orders_events() {
        let mut tracker = TurnTracker::new("session-1".to_string());
        let turn_id = "turn-1".to_string();
        let call_id = "tool-1".to_string();
        let cwd = PathBuf::from("/tmp");

        let events = vec![
            Event {
                id: turn_id.clone(),
                msg: EventMsg::TurnStarted(TurnStartedEvent {
                    model_context_window: None,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                    call_id: call_id.clone(),
                    process_id: None,
                    turn_id: turn_id.clone(),
                    command: vec!["echo".to_string(), "hi".to_string()],
                    cwd: cwd.clone(),
                    parsed_cmd: Vec::<ParsedCommand>::new(),
                    source: ExecCommandSource::Agent,
                    interaction_input: None,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                    call_id: call_id.clone(),
                    stream: ExecOutputStream::Stdout,
                    chunk: b"hi".to_vec(),
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                    call_id,
                    process_id: None,
                    turn_id: turn_id.clone(),
                    command: vec!["echo".to_string(), "hi".to_string()],
                    cwd,
                    parsed_cmd: Vec::<ParsedCommand>::new(),
                    source: ExecCommandSource::Agent,
                    interaction_input: None,
                    stdout: "hi
"
                    .to_string(),
                    stderr: String::new(),
                    aggregated_output: "hi
"
                    .to_string(),
                    exit_code: 0,
                    duration: std::time::Duration::from_millis(5),
                    formatted_output: "hi
"
                    .to_string(),
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                    thread_id: "thread".to_string(),
                    turn_id: turn_id.clone(),
                    item_id: "msg-1".to_string(),
                    delta: "ok".to_string(),
                }),
            },
            Event {
                id: turn_id,
                msg: EventMsg::TurnComplete(TurnCompleteEvent {
                    last_agent_message: Some("done".to_string()),
                }),
            },
        ];

        let mut mapped = Vec::new();
        for event in events {
            mapped.extend(map_codex_event(&mut tracker, event));
        }

        let kinds: Vec<&'static str> = mapped.iter().map(|(_, event)| event_kind(event)).collect();
        assert_eq!(
            kinds,
            vec![
                "turn.started",
                "tool.started",
                "tool.output.delta",
                "tool.completed",
                "message.delta",
                "message.final",
                "turn.completed",
            ]
        );

        let message_delta_id = mapped
            .iter()
            .find_map(|(_, event)| match event {
                CrpEvent::MessageDelta { message_id, .. } => Some(message_id.clone()),
                _ => None,
            })
            .expect("expected message delta event");
        let message_final_id = mapped
            .iter()
            .find_map(|(_, event)| match event {
                CrpEvent::MessageFinal { message_id, .. } => Some(message_id.clone()),
                _ => None,
            })
            .expect("expected message final event");
        assert_eq!(message_delta_id, message_final_id);

        let tool_started_id = mapped
            .iter()
            .find_map(|(_, event)| match event {
                CrpEvent::ToolStarted { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .expect("expected tool.started event");
        let tool_completed_id = mapped
            .iter()
            .find_map(|(_, event)| match event {
                CrpEvent::ToolCompleted { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .expect("expected tool.completed event");
        assert_eq!(tool_started_id, tool_completed_id);

        let delta_chunk = mapped
            .iter()
            .find_map(|(_, event)| match event {
                CrpEvent::ToolOutputDelta { chunk, .. } => Some(chunk.clone()),
                _ => None,
            })
            .expect("expected tool.output.delta event");
        let expected_chunk = base64::engine::general_purpose::STANDARD.encode(b"hi");
        assert_eq!(delta_chunk, expected_chunk);
    }

    #[test]
    fn crp_mapping_emits_reasoning_events() {
        let mut tracker = TurnTracker::new("session-1".to_string());
        let turn_id = "turn-1".to_string();

        let events = vec![
            Event {
                id: turn_id.clone(),
                msg: EventMsg::TurnStarted(TurnStartedEvent {
                    model_context_window: None,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ReasoningContentDelta(ReasoningContentDeltaEvent {
                    thread_id: "thread".to_string(),
                    turn_id: turn_id.clone(),
                    item_id: "reasoning-1".to_string(),
                    delta: "summary".to_string(),
                    summary_index: 0,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ReasoningRawContentDelta(ReasoningRawContentDeltaEvent {
                    thread_id: "thread".to_string(),
                    turn_id: turn_id.clone(),
                    item_id: "reasoning-1".to_string(),
                    delta: "raw".to_string(),
                    content_index: 0,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ReasoningRawContentDelta(ReasoningRawContentDeltaEvent {
                    thread_id: "thread".to_string(),
                    turn_id,
                    item_id: "reasoning-1".to_string(),
                    delta: "raw-final".to_string(),
                    content_index: 1,
                }),
            },
        ];

        let mut mapped = Vec::new();
        for event in events {
            mapped.extend(map_codex_event(&mut tracker, event));
        }

        let summary = mapped.iter().find_map(|(channel, event)| match event {
            CrpEvent::ReasoningSummary { text, item_id, .. } => {
                Some((channel, text.clone(), item_id.clone()))
            }
            _ => None,
        });
        let Some((channel, text, item_id)) = summary else {
            panic!("missing reasoning summary");
        };
        assert_eq!(channel, &CrpChannel::Control);
        assert_eq!(text, "summary".to_string());
        assert_eq!(item_id.as_deref(), Some("reasoning-1"));

        let trace_chunks: Vec<_> = mapped
            .iter()
            .filter_map(|(channel, event)| match event {
                CrpEvent::ReasoningTrace { chunk, .. } => Some((channel, chunk.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(trace_chunks.len(), 2);
        assert_eq!(trace_chunks[0].0, &CrpChannel::Data);
        assert_eq!(trace_chunks[0].1, "raw".to_string());
        assert_eq!(trace_chunks[1].0, &CrpChannel::Data);
        assert_eq!(trace_chunks[1].1, "raw-final".to_string());
    }

    #[test]
    fn data_plane_overflow_emits_session_gap() {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        let (data_tx, _data_rx) = mpsc::channel(1);
        let router = CrpEventRouter::new(control_tx, data_tx);

        let data_event = CrpEvent::MessageDelta {
            session_id: "session-1".to_string(),
            run_id: "run-1".to_string(),
            turn_id: "turn-1".to_string(),
            message_id: "msg-1".to_string(),
            delta: "hello".to_string(),
        };

        dispatch_event(&router, CrpChannel::Data, data_event.clone());
        dispatch_event(&router, CrpChannel::Data, data_event);

        let gap = control_rx.try_recv().ok();
        match gap {
            Some(CrpEvent::SessionGap { session_id, .. }) => {
                assert_eq!(session_id, "session-1");
            }
            other => panic!("expected session.gap event, got {other:?}"),
        }
    }

    fn event_kind(event: &CrpEvent) -> &'static str {
        match event {
            CrpEvent::TurnStarted { .. } => "turn.started",
            CrpEvent::ToolStarted { .. } => "tool.started",
            CrpEvent::ToolOutputDelta { .. } => "tool.output.delta",
            CrpEvent::ToolCompleted { .. } => "tool.completed",
            CrpEvent::MessageDelta { .. } => "message.delta",
            CrpEvent::MessageFinal { .. } => "message.final",
            CrpEvent::TurnCompleted { .. } => "turn.completed",
            _ => "other",
        }
    }
}
