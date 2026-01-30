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
use codex_core::protocol::ExecCommandSource;
use codex_core::protocol::ExecOutputStream;
use codex_core::protocol::FileChange;
use codex_core::protocol::Op;
use codex_core::protocol::SandboxPolicy;
use codex_core::protocol::Submission;
use codex_core::protocol::TurnAbortReason;
use codex_core::find_thread_path_by_id_str;
use codex_core::SESSIONS_SUBDIR;
use codex_core::ARCHIVED_SESSIONS_SUBDIR;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::TurnItem;
use codex_protocol::openai_models::{ModelPreset, ReasoningEffort};
use codex_protocol::parse_command::ParsedCommand;
use codex_protocol::protocol::SessionSource;
use codex_protocol::user_input::UserInput;
use mcp_types::CallToolResult;
use mcp_types::ContentBlock;
use mcp_types::TextContent;
use serde_json::json;
use std::fs::OpenOptions;
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
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

    /// Offline translator mode: read Codex internal event dump JSONL (from CODEX_CRP_DUMP_CODEX_EVENTS_PATH)
    /// and emit translated CRP JSONL to the given output path.
    ///
    /// This mode does not contact any provider; it's purely a JSONL -> JSONL mapping runner.
    #[clap(long)]
    replay_codex_events: Option<PathBuf>,

    /// Output path for `--replay-codex-events` (defaults to `<input>.crp.jsonl`).
    #[clap(long)]
    replay_out: Option<PathBuf>,
}

enum RuntimeCommand {
    Parsed(CrpCommand),
    ParseError { message: String },
}

const DATA_PLANE_BUFFER_CAPACITY: usize = 256;
const TOOL_OUTPUT_CHUNK_BYTES: usize = 8 * 1024;

// Optional debug dump of raw Codex internal events (EventMsg) before any CRP mapping.
//
// Enable by setting `CODEX_CRP_DUMP_CODEX_EVENTS_PATH=/path/to/file.jsonl`.
// Each line is JSON: { "i": <monotonic>, "event": <codex_core::protocol::Event> }.
static CODEX_EVENT_DUMP: OnceLock<Mutex<std::io::BufWriter<std::fs::File>>> = OnceLock::new();
static CODEX_EVENT_DUMP_SEQ: AtomicU64 = AtomicU64::new(1);
const CODEX_EVENT_DUMP_ENV: &str = "CODEX_CRP_DUMP_CODEX_EVENTS_PATH";

// Optional debug dump of CRP events emitted by this runtime (after mapping, before stdout).
//
// Enable by setting `CODEX_CRP_DUMP_CRP_EVENTS_PATH=/path/to/file.jsonl`.
// Each line is JSON: <CrpEventEnvelope>.
static CRP_EVENT_DUMP: OnceLock<Mutex<std::io::BufWriter<std::fs::File>>> = OnceLock::new();
const CRP_EVENT_DUMP_ENV: &str = "CODEX_CRP_DUMP_CRP_EVENTS_PATH";

fn maybe_dump_codex_event(event: &Event) {
    // Fast-path when not enabled.
    let Ok(path) = std::env::var(CODEX_EVENT_DUMP_ENV) else {
        return;
    };

    let writer = CODEX_EVENT_DUMP.get_or_init(|| {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("failed to open CODEX_CRP_DUMP_CODEX_EVENTS_PATH");
        Mutex::new(std::io::BufWriter::new(file))
    });

    let Ok(mut w) = writer.lock() else {
        return;
    };

    let seq = CODEX_EVENT_DUMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let obj = json!({
        "i": seq,
        "event": event,
    });
    if serde_json::to_writer(&mut *w, &obj).is_ok() {
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }
}

fn maybe_dump_crp_event(envelope: &CrpEventEnvelope) {
    // Fast-path when not enabled.
    let Ok(path) = std::env::var(CRP_EVENT_DUMP_ENV) else {
        return;
    };

    let writer = CRP_EVENT_DUMP.get_or_init(|| {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("failed to open CODEX_CRP_DUMP_CRP_EVENTS_PATH");
        Mutex::new(std::io::BufWriter::new(file))
    });

    let Ok(mut w) = writer.lock() else {
        return;
    };

    if serde_json::to_writer(&mut *w, envelope).is_ok() {
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }
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

    async fn send(&mut self, channel: CrpChannel, event: CrpEvent) -> anyhow::Result<()> {
        let envelope = CrpEventEnvelope {
            v: 1,
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
    turn_id: String,
    message_id: Option<String>,
    reasoning_item_id: Option<String>,
    current_summary_index: i64,
    // Codex can emit multiple reasoning items per turn, and each item can have multiple summary blocks.
    // Key by (item_id, summary_index) to avoid cross-item bleed that causes title/body duplication.
    reasoning_summaries: HashMap<(String, i64), ReasoningSummaryState>,
    emitted_final: bool,
    completed: bool,
}

impl TurnState {
    fn new(turn_id: String) -> Self {
        Self {
            turn_id,
            message_id: None,
            reasoning_item_id: None,
            current_summary_index: 0,
            reasoning_summaries: HashMap::new(),
            emitted_final: false,
            completed: false,
        }
    }
}

#[derive(Default, Debug)]
struct ReasoningSummaryState {
    buffer: String,
    title: Option<String>,
    title_emitted: bool,
    body_offset: usize,
    // Number of bytes emitted from the body region (buffer[body_offset..]).
    body_sent: usize,
}

impl ReasoningSummaryState {
    fn looks_like_snapshot(delta: &str) -> bool {
        // Codex sometimes emits "snapshot" deltas that contain the entire block:
        //   **Title**\n\nBody...
        // These should replace the buffer (not append), otherwise we duplicate title/body.
        delta.starts_with("**") && delta.contains("\n\n") && delta.matches("**").count() >= 2 && delta.len() >= 16
    }

    fn looks_like_title_only(delta: &str) -> bool {
        // Codex also sometimes emits repeated "title only" deltas like:
        //   **Reading files**
        // If we've already emitted that title, these must not enter the trace stream.
        let t = delta.trim();
        t.starts_with("**") && t.ends_with("**") && !t.contains("\n\n") && t.matches("**").count() >= 2
    }

    fn push_delta(&mut self, delta: &str) -> (Option<String>, Option<String>) {
        if delta.is_empty() {
            return (None, None);
        }

        // If we've already emitted a title and Codex repeats a title-only delta, treat it as a
        // no-op update (or a title update), never as trace content.
        if self.title_emitted && Self::looks_like_title_only(delta) {
            // Extract inner title text (best-effort).
            let t = delta.trim();
            if let Some((title, _)) = extract_summary_title_and_body_offset(t) {
                if self.title.as_deref() == Some(title.as_str()) {
                    // Repeated title; ignore.
                    return (None, None);
                }
                // Title changed without body; update status title and reset body tracking.
                self.buffer.clear();
                self.buffer.push_str(t);
                self.title = Some(title.clone());
                self.title_emitted = true;
                // The body is empty for title-only blocks.
                if let Some((_title, body_offset)) = extract_summary_title_and_body_offset(&self.buffer) {
                    self.body_offset = body_offset;
                }
                self.body_sent = 0;
                return (Some(title), None);
            }
        }

        let replaced_with_snapshot = if Self::looks_like_snapshot(delta) {
            // Replace (best-known full text), rather than append.
            self.buffer.clear();
            self.buffer.push_str(delta);
            true
        } else {
            self.buffer.push_str(delta);
            false
        };

        if !self.title_emitted {
            if let Some((title, body_offset)) = extract_summary_title_and_body_offset(&self.buffer) {
                self.title_emitted = true;
                self.title = Some(title.clone());
                self.body_offset = body_offset;
                self.body_sent = 0;
                let body = if self.buffer.len() > body_offset {
                    let body = self.buffer[body_offset..].to_string();
                    self.body_sent = body.len();
                    Some(body)
                } else {
                    None
                };
                let title = if title.trim().is_empty() {
                    None
                } else {
                    Some(title)
                };
                return (title, body);
            }
            return (None, None);
        }

        // If Codex sent a snapshot after the title was already emitted, recompute the body offset
        // to keep the "post-title" boundary aligned with the latest full text. Otherwise the
        // title block (and its newlines) can leak into the thought stream.
        if replaced_with_snapshot {
            if let Some((title, body_offset)) = extract_summary_title_and_body_offset(&self.buffer) {
                // If a snapshot changed the title, publish the new status title and reset body tracking.
                if self.title.as_deref() != Some(title.as_str()) {
                    self.title = Some(title.clone());
                    self.body_sent = 0;
                    self.body_offset = body_offset;
                    return (Some(title), None);
                }
                self.body_offset = body_offset;
            }
        }

        // Codex typically formats summary blocks like: "**Title**\n\nBody...". When the body
        // arrives in a later delta, the leading whitespace/newlines might appear only then.
        // Trim that leading whitespace exactly once (before the first emitted body chunk) so
        // blank lines don't leak into the thought stream.
        if self.body_sent == 0 && self.buffer.len() > self.body_offset {
            let sub = &self.buffer[self.body_offset..];
            let trimmed = sub.trim_start();
            let skipped = sub.len().saturating_sub(trimmed.len());
            if skipped > 0 {
                self.body_offset += skipped;
            }
        }

        // Stream only the new suffix of the body region (post-title).
        if self.buffer.len() > self.body_offset {
            let body = &self.buffer[self.body_offset..];
            if self.body_sent > body.len() {
                // If a snapshot replaced the buffer with a shorter version, clamp.
                self.body_sent = body.len();
            }
            if body.len() > self.body_sent {
                let chunk = body[self.body_sent..].to_string();
                self.body_sent = body.len();
                return (None, Some(chunk));
            }
        }

        (None, None)
    }
}

#[cfg(test)]
mod reasoning_summary_state_tests {
    use super::*;

    #[test]
    fn title_only_duplicates_do_not_emit_trace() {
        let mut s = ReasoningSummaryState::default();

        // First title-only delta emits a summary title, no trace.
        let (title, trace) = s.push_delta("**Applying patch to add hello_world.txt**");
        assert_eq!(title.as_deref(), Some("Applying patch to add hello_world.txt"));
        assert!(trace.is_none());

        // Repeated title-only deltas must not leak into trace.
        let (title2, trace2) = s.push_delta("**Applying patch to add hello_world.txt**");
        assert!(title2.is_none());
        assert!(trace2.is_none());
    }

    #[test]
    fn snapshot_with_body_emits_body_once_and_dedupes() {
        let mut s = ReasoningSummaryState::default();

        let (title, trace) =
            s.push_delta("**Reading instruction files**

I'm checking the .ctx directory.");
        assert_eq!(title.as_deref(), Some("Reading instruction files"));
        assert_eq!(trace.as_deref(), Some("I'm checking the .ctx directory."));

        // Duplicate full snapshot should not re-emit the body.
        let (title2, trace2) =
            s.push_delta("**Reading instruction files**

I'm checking the .ctx directory.");
        assert!(title2.is_none());
        assert!(trace2.is_none());
    }

    #[test]
    fn title_then_body_delta_emits_body_without_title() {
        let mut s = ReasoningSummaryState::default();

        let (title, trace) = s.push_delta("**Exploring context files**");
        assert_eq!(title.as_deref(), Some("Exploring context files"));
        assert!(trace.is_none());

        let (_title2, trace2) = s.push_delta("

I'm checking the .ctx directory for agent-basics.");
        assert_eq!(
            trace2.as_deref(),
            Some("I'm checking the .ctx directory for agent-basics.")
        );
    }
}

#[cfg(test)]
mod replay_golden_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn testdata_path(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(file)
    }

    fn replay_fixture(input_name: &str) -> Vec<serde_json::Value> {
        let input_path = testdata_path(input_name);
        let input = fs::read_to_string(&input_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", input_path.display()));

        let mut tracker = TurnTracker::new("replay_session".to_string());

        let mut seq: u64 = 0;
        let mut out = Vec::new();

        // Seed a SessionOpened event so snapshots are self-contained (mirrors offline replay mode).
        seq += 1;
        let opened = CrpEventEnvelope {
            v: 1,
            seq,
            channel: CrpChannel::Control,
            event: CrpEvent::SessionOpened {
                session_id: "replay_session".to_string(),
                provider_session_id: None,
            },
        };
        out.push(serde_json::to_value(&opened).unwrap());

        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let v: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = v
                .get("event")
                .and_then(|e| e.get("msg"))
                .and_then(|m| m.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            match msg_type {
                // Large / legacy / redundant events shouldn't influence the CRP output.
                "raw_response_item" => continue,
                "agent_message_delta" | "agent_message" => continue,
                "agent_reasoning_delta" | "agent_reasoning" => continue,
                _ => {}
            }

            let Some(ev_val) = v.get("event") else { continue };
            let event: Event = match serde_json::from_value(ev_val.clone()) {
                Ok(ev) => ev,
                Err(_) => continue,
            };

            for (channel, event) in map_codex_event(&mut tracker, event) {
                seq += 1;
                let env = CrpEventEnvelope {
                    v: 1,
                    seq,
                    channel,
                    event,
                };
                out.push(serde_json::to_value(&env).unwrap());
            }
        }

        out
    }

    fn assert_fixture(input: &str, expected: &str) {
        let got = replay_fixture(input);
        let expected_path = testdata_path(expected);
        let expected_contents = fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", expected_path.display()));
        let expected_values: Vec<serde_json::Value> = expected_contents
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(serde_json::from_str(t).unwrap())
                }
            })
            .collect();

        assert_eq!(got, expected_values);
    }

    #[test]
    fn wal_title_only_does_not_emit_trace() {
        assert_fixture(
            "wal_title_only_no_trace.input.jsonl",
            "wal_title_only_no_trace.expected.jsonl",
        );
    }

    #[test]
    fn wal_title_then_body_emits_body_only() {
        assert_fixture(
            "wal_title_then_body_trace_no_title.input.jsonl",
            "wal_title_then_body_trace_no_title.expected.jsonl",
        );
    }
}

fn extract_summary_title_and_body_offset(text: &str) -> Option<(String, usize)> {
    let start = text.find("**")?;
    let after_start = start + 2;
    let end_rel = text[after_start..].find("**")?;
    let end = after_start + end_rel;
    let title = text[after_start..end].trim().to_string();
    let mut body_offset = end + 2;

    // Codex formats summaries like: "**Title**\n\nBody...".
    // We want title-only status and body-only trace; drop leading whitespace in the body so the
    // first emitted trace chunk doesn't start with blank lines.
    if body_offset < text.len() {
        let sub = &text[body_offset..];
        let trimmed = sub.trim_start();
        let skipped = sub.len().saturating_sub(trimmed.len());
        body_offset += skipped;
    }

    Some((title, body_offset))
}

struct ToolBridgeRequest {
    session_id: String,
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
        let tool_name = tool_name_for_payload(&tool_name, &payload);
        let input = tool_payload_to_value(&payload);

        let (respond_to, receiver) = oneshot::channel();
        let request = ToolBridgeRequest {
            session_id,
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
    // Important: `codex-core` expects binaries embedding it (including this runtime) to support
    // a virtual `apply_patch` CLI via the arg0 dispatch mechanism:
    //   <bin> --codex-run-as-apply-patch
    //
    // If we parse CLI args before calling `arg0_dispatch_or_else`, clap will reject the
    // `--codex-run-as-apply-patch` flag and `apply_patch` will be broken.
    arg0_dispatch_or_else(|codex_linux_sandbox_exe| async move {
        let cli = Cli::parse();
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

    if let Some(input_path) = cli.replay_codex_events.clone() {
        return run_replay_codex_events(
            input_path,
            cli.replay_out.clone(),
        )
        .await;
    }

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

async fn run_replay_codex_events(
    input_path: PathBuf,
    out_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    use crate::protocol::CrpEventEnvelope;
    use tokio::io::AsyncWriteExt;

    let out_path = out_path.unwrap_or_else(|| {
        let mut p = input_path.clone();
        let file_name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "codex_events.jsonl".to_string());
        p.set_file_name(format!("{file_name}.crp.jsonl"));
        p
    });

    let mut tracker = TurnTracker::new("replay_session".to_string());

    let input = tokio::fs::File::open(&input_path).await?;
    let mut reader = BufReader::new(input).lines();

    let output = tokio::fs::File::create(&out_path).await?;
    let mut out = BufWriter::new(output);

    // Seed a SessionOpened event so the output is self-contained.
    let mut seq: u64 = 0;
    seq += 1;
    let opened = CrpEventEnvelope {
        v: 1,
        seq,
        channel: CrpChannel::Control,
        event: CrpEvent::SessionOpened {
            session_id: "replay_session".to_string(),
            provider_session_id: None,
        },
    };
    out.write_all(serde_json::to_vec(&opened)?.as_slice()).await?;
    out.write_all(b"\n").await?;

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Dump format: { "i": <n>, "event": { "id": "...", "msg": { "type": "..." } } }
        let msg_type = v
            .get("event")
            .and_then(|e| e.get("msg"))
            .and_then(|m| m.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Ignore large / legacy / redundant events for the offline translator.
        match msg_type {
            "raw_response_item" => continue,
            "agent_message_delta" | "agent_message" => continue,
            "agent_reasoning_delta" | "agent_reasoning" => continue,
            _ => {}
        }

        let Some(ev_val) = v.get("event") else { continue };
        let event: Event = match serde_json::from_value(ev_val.clone()) {
            Ok(ev) => ev,
            Err(_) => continue,
        };

        for (channel, event) in map_codex_event(&mut tracker, event) {
            seq += 1;
            let env = CrpEventEnvelope {
                v: 1,
                seq,
                channel,
                event,
            };
            out.write_all(serde_json::to_vec(&env)?.as_slice()).await?;
            out.write_all(b"\n").await?;
        }
    }

    out.flush().await?;
    eprintln!("replay complete: wrote {}", out_path.display());
    Ok(())
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
                model_provider: None,
                approval_policy: None,
                sandbox_mode: None,
                reasoning_trace_enabled: None,
                mcp_servers: None,
            });

            let provider_session_id = provider_session_id.or_else(|| {
                std::env::var("CTX_PROVIDER_SESSION_REF")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });

            let state = open_session(
                config,
                cli_kv_overrides,
                codex_linux_sandbox_exe,
                provider_session_id.clone(),
            )
            .await?;
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
                collaboration_mode: None,
                personality: None,
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
            let presets = thread_manager
                .list_models(
                    &config,
                    codex_core::models_manager::manager::RefreshStrategy::OnlineIfUncached,
                )
                .await;
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
        model_personality: None,
        compact_prompt: None,
        include_apply_patch_tool: None,
        show_raw_agent_reasoning: session_config.reasoning_trace_enabled,
        tools_web_search_request: None,
        ephemeral: None,
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
    provider_session_id: Option<String>,
) -> anyhow::Result<SessionState> {
    let config =
        load_config_from_crp(session_config, cli_kv_overrides, codex_linux_sandbox_exe).await?;

    let auth_manager = AuthManager::shared(
        config.codex_home.clone(),
        true,
        config.cli_auth_credentials_store_mode,
    );
    let thread_manager = ThreadManager::new(
        config.codex_home.clone(),
        Arc::clone(&auth_manager),
        SessionSource::Exec,
    );
    let default_model = thread_manager
        .get_models_manager()
        .get_default_model(
            &config.model,
            &config,
            codex_core::models_manager::manager::RefreshStrategy::OnlineIfUncached,
        )
        .await;

    let resume_path = if let Some(id) = provider_session_id.as_deref() {
        match find_thread_path_by_id_str(&config.codex_home, id).await {
            Ok(Some(path)) => {
                if rollout_filename_matches_id(&path, id) {
                    Some(path)
                } else {
                    let fallback = find_rollout_path_fallback(&config.codex_home, id).await;
                    if fallback.is_none() {
                        warn!(%id, "provider_session_id not found; starting new session");
                    }
                    fallback
                }
            }
            Ok(None) => {
                let fallback = find_rollout_path_fallback(&config.codex_home, id).await;
                if fallback.is_none() {
                    warn!(%id, "provider_session_id not found; starting new session");
                }
                fallback
            }
            Err(err) => {
                warn!(%id, ?err, "failed to look up provider_session_id; trying fallback");
                let fallback = find_rollout_path_fallback(&config.codex_home, id).await;
                if fallback.is_none() {
                    warn!(%id, "provider_session_id not found; starting new session");
                }
                fallback
            }
        }
    } else {
        None
    };

    let NewThread {
        thread_id,
        thread,
        session_configured: _,
    } = if let Some(path) = resume_path {
        thread_manager
            .resume_thread_from_rollout(config.clone(), path, auth_manager)
            .await?
    } else {
        thread_manager.start_thread(config.clone()).await?
    };

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

async fn find_rollout_path_fallback(codex_home: &PathBuf, id: &str) -> Option<PathBuf> {
    let roots = [SESSIONS_SUBDIR, ARCHIVED_SESSIONS_SUBDIR];
    for subdir in roots {
        let root = codex_home.join(subdir);
        if let Some(found) = find_rollout_path_in_dir(&root, id).await {
            return Some(found);
        }
    }
    None
}

fn rollout_filename_matches_id(path: &PathBuf, id: &str) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    name.contains("rollout-") && name.contains(id)
}

async fn find_rollout_path_in_dir(root: &PathBuf, id: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.contains("rollout-") && name.contains(id) {
                    return Some(path);
                }
            }
        }
    }
    None
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
        turn_id,
        tool_call_id,
        tool_name,
        input,
        respond_to,
    } = request;

    let _ = router.send_control(CrpEvent::ToolRequest {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.clone(),
        input: input.clone(),
    });
    let _ = router.send_control(CrpEvent::ToolStarted {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.clone(),
        tool_label: None,
        input,
        input_preview: None,
    });

    if pending
        .insert(
            tool_call_id.clone(),
            PendingToolRequest {
                session_id,
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
            turn_id: pending_request.turn_id.clone(),
            tool_call_id: command.tool_call_id.clone(),
            tool_name: pending_request.tool_name,
            tool_label: None,
            status: result.status.clone(),
            output: result.output.clone(),
            error: result.error.clone(),
            input_preview: None,
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

fn is_exploring_parsed_cmd(parsed_cmd: &[ParsedCommand]) -> bool {
    !parsed_cmd.is_empty()
        && parsed_cmd.iter().all(|parsed| {
            matches!(
                parsed,
                ParsedCommand::Read { .. }
                    | ParsedCommand::ListFiles { .. }
                    | ParsedCommand::Search { .. }
            )
        })
}

fn tool_label_for_exec(parsed_cmd: &[ParsedCommand], completed: bool) -> String {
    if is_exploring_parsed_cmd(parsed_cmd) {
        if completed {
            "Explored".to_string()
        } else {
            "Exploring".to_string()
        }
    } else if completed {
        "Ran".to_string()
    } else {
        "Running".to_string()
    }
}

fn exec_input_preview(
    command: &[String],
    parsed_cmd: &[ParsedCommand],
    cwd: &PathBuf,
    source: ExecCommandSource,
    interaction_input: Option<&String>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("command".to_string(), json!(command));
    obj.insert("cwd".to_string(), json!(cwd));
    obj.insert("source".to_string(), json!(source));
    if !parsed_cmd.is_empty() {
        obj.insert("parsed_cmd".to_string(), json!(parsed_cmd));
    }
    if let Some(input) = interaction_input {
        obj.insert("interaction_input".to_string(), json!(input));
    }
    serde_json::Value::Object(obj)
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn diff_stats_for_changes(changes: &HashMap<PathBuf, FileChange>) -> (usize, usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in changes.values() {
        match change {
            FileChange::Add { content } => {
                added += count_lines(content);
            }
            FileChange::Delete { content } => {
                removed += count_lines(content);
            }
            FileChange::Update { unified_diff, .. } => {
                for line in unified_diff.lines() {
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
        }
    }
    (added, removed, changes.len())
}

fn change_paths(changes: &HashMap<PathBuf, FileChange>) -> Vec<String> {
    let mut paths: Vec<String> = changes
        .keys()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    paths.sort();
    paths
}

fn patch_input_preview(changes: &HashMap<PathBuf, FileChange>) -> serde_json::Value {
    let paths = change_paths(changes);
    let (added, removed, files) = diff_stats_for_changes(changes);
    let mut obj = serde_json::Map::new();
    if let Some(first) = paths.first() {
        if paths.len() == 1 {
            obj.insert("path".to_string(), json!(first));
        } else {
            obj.insert("paths".to_string(), json!(paths));
            obj.insert("paths_total".to_string(), json!(paths.len()));
        }
    }
    obj.insert(
        "diff_stats".to_string(),
        json!({
            "added": added,
            "removed": removed,
            "files": files,
        }),
    );
    serde_json::Value::Object(obj)
}

fn tool_label_for_patch(changes: &HashMap<PathBuf, FileChange>) -> String {
    if changes.len() == 1 {
        if let Some(change) = changes.values().next() {
            return match change {
                FileChange::Add { .. } => "Added".to_string(),
                FileChange::Delete { .. } => "Deleted".to_string(),
                FileChange::Update { .. } => "Edited".to_string(),
            };
        }
    }
    "Edited".to_string()
}

fn map_codex_event(tracker: &mut TurnTracker, event: Event) -> Vec<(CrpChannel, CrpEvent)> {
    maybe_dump_codex_event(&event);
    let session_id = tracker.session_id.clone();
    match event.msg {
        EventMsg::TurnStarted(_) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Control,
                CrpEvent::TurnStarted {
                    session_id,
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
            turn.current_summary_index = ev.summary_index;
            let key_item_id = item_id
                .clone()
                .unwrap_or_else(|| "unknown_reasoning_item".to_string());
            let state = turn
                .reasoning_summaries
                .entry((key_item_id, ev.summary_index))
                .or_default();
            let (title, body_chunk) = state.push_delta(&ev.delta);
            let mut out = Vec::new();
            if let Some(title) = title {
                out.push((
                    CrpChannel::Control,
                    CrpEvent::ReasoningSummary {
                        session_id: session_id.clone(),
                        turn_id: turn.turn_id.clone(),
                        summary_index: ev.summary_index,
                        text: title,
                        item_id,
                    },
                ));
            }
            if let Some(chunk) = body_chunk {
                out.push((
                    CrpChannel::Data,
                    CrpEvent::ReasoningTrace {
                        session_id,
                        turn_id: turn.turn_id.clone(),
                        chunk,
                        encoding: None,
                    },
                ));
            }
            out
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
            turn.current_summary_index = ev.summary_index;
            Vec::new()
        }
        EventMsg::AgentReasoningDelta(_) => Vec::new(),
        // Latest Codex emits structured reasoning via `ReasoningContentDelta` (and section breaks).
        // The legacy `AgentReasoning*` family can arrive concurrently and is not stable across
        // Codex versions; it can duplicate content and introduce formatting artifacts (wrapped
        // newlines, title blocks leaking into the thought stream). Ignore it entirely.
        EventMsg::AgentReasoning(_) => Vec::new(),
        EventMsg::AgentReasoningRawContentDelta(_) => Vec::new(),
        EventMsg::AgentReasoningRawContent(_) => Vec::new(),
        EventMsg::ExecCommandBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let input_preview = exec_input_preview(
                &ev.command,
                &ev.parsed_cmd,
                &ev.cwd,
                ev.source,
                ev.interaction_input.as_ref(),
            );
            let tool_label = tool_label_for_exec(&ev.parsed_cmd, false);
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "exec".to_string(),
                    tool_label: Some(tool_label),
                    input: Some(input_preview.clone()),
                    input_preview: Some(input_preview),
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
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    stream: Some(CrpToolOutputStream::Stdin),
                    chunk,
                },
            )]
        }
        EventMsg::ExecCommandEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let input_preview = exec_input_preview(
                &ev.command,
                &ev.parsed_cmd,
                &ev.cwd,
                ev.source,
                ev.interaction_input.as_ref(),
            );
            let tool_label = tool_label_for_exec(&ev.parsed_cmd, true);
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
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "exec".to_string(),
                    tool_label: Some(tool_label),
                    status,
                    output: Some(output),
                    error,
                    input_preview: Some(input_preview),
                },
            )]
        }
        EventMsg::PatchApplyBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let preview = patch_input_preview(&ev.changes);
            let tool_label = tool_label_for_patch(&ev.changes);
            let input = json!({
                "auto_approved": ev.auto_approved,
                "preview": preview,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "apply_patch".to_string(),
                    tool_label: Some(tool_label),
                    input: Some(input),
                    input_preview: Some(patch_input_preview(&ev.changes)),
                },
            )]
        }
        EventMsg::PatchApplyEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let preview = patch_input_preview(&ev.changes);
            let tool_label = tool_label_for_patch(&ev.changes);
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
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "apply_patch".to_string(),
                    tool_label: Some(tool_label),
                    status,
                    output: Some(output),
                    error,
                    input_preview: Some(preview),
                },
            )]
        }
        EventMsg::WebSearchBegin(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "web_search".to_string(),
                    tool_label: Some("Search".to_string()),
                    input: None,
                    input_preview: None,
                },
            )]
        }
        EventMsg::WebSearchEnd(ev) => {
            let turn = ensure_turn(tracker, &event.id);
            let input_preview = json!({ "query": ev.query });
            let output = json!({
                "query": ev.query,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name: "web_search".to_string(),
                    tool_label: Some("Searched".to_string()),
                    status: CrpToolStatus::Success,
                    output: Some(output),
                    error: None,
                    input_preview: Some(input_preview),
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
            let input_preview = json!({
                "server": server,
                "tool": tool,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolStarted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name,
                    tool_label: Some("Call".to_string()),
                    input: Some(input),
                    input_preview: Some(input_preview),
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
            let input_preview = json!({
                "server": server,
                "tool": tool,
            });
            vec![(
                CrpChannel::Control,
                CrpEvent::ToolCompleted {
                    session_id,
                    turn_id: turn.turn_id.clone(),
                    tool_call_id: ev.call_id,
                    tool_name,
                    tool_label: Some("Called".to_string()),
                    status,
                    output,
                    error,
                    input_preview: Some(input_preview),
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
                        turn_id: turn.turn_id.clone(),
                        tool_call_id: ev.call_id.clone(),
                        tool_name: "view_image".to_string(),
                        tool_label: Some("View".to_string()),
                        input: Some(payload.clone()),
                        input_preview: Some(payload.clone()),
                    },
                ),
                (
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: turn.turn_id.clone(),
                        tool_call_id: ev.call_id,
                        tool_name: "view_image".to_string(),
                        tool_label: Some("Viewed".to_string()),
                        status: CrpToolStatus::Success,
                        output: Some(payload),
                        error: None,
                        input_preview: None,
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
                        turn_id: turn.turn_id.clone(),
                        status: CrpTurnStatus::Success,
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
                        turn_id: turn.turn_id.clone(),
                        status,
                    },
                )]
            } else {
                Vec::new()
            }
        }
        EventMsg::Error(_) | EventMsg::StreamError(_) => {
            let turn = ensure_turn(tracker, &event.id);
            if mark_completed(turn) {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::TurnCompleted {
                        session_id,
                        turn_id: turn.turn_id.clone(),
                        status: CrpTurnStatus::Error,
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
                    delta: "**Reading".to_string(),
                    summary_index: 0,
                }),
            },
            Event {
                id: turn_id.clone(),
                msg: EventMsg::ReasoningContentDelta(ReasoningContentDeltaEvent {
                    thread_id: "thread".to_string(),
                    turn_id: turn_id.clone(),
                    item_id: "reasoning-1".to_string(),
                    delta: " foo**\n\nThinking about bar".to_string(),
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

        let summaries: Vec<_> = mapped
            .iter()
            .filter_map(|(channel, event)| match event {
                CrpEvent::ReasoningSummary { text, item_id, .. } => {
                    Some((channel, text.clone(), item_id.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(summaries.len(), 1);
        let summary = summaries.into_iter().next();
        let Some((channel, text, item_id)) = summary else {
            panic!("missing reasoning summary");
        };
        assert_eq!(channel, &CrpChannel::Control);
        assert_eq!(text, "Reading foo".to_string());
        assert_eq!(item_id.as_deref(), Some("reasoning-1"));

        let trace_chunks: Vec<_> = mapped
            .iter()
            .filter_map(|(channel, event)| match event {
                CrpEvent::ReasoningTrace { chunk, .. } => Some((channel, chunk.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(trace_chunks.len(), 3);
        assert_eq!(trace_chunks[0].0, &CrpChannel::Data);
        assert_eq!(trace_chunks[0].1, "Thinking about bar".to_string());
        assert_eq!(trace_chunks[1].0, &CrpChannel::Data);
        assert_eq!(trace_chunks[1].1, "raw".to_string());
        assert_eq!(trace_chunks[2].0, &CrpChannel::Data);
        assert_eq!(trace_chunks[2].1, "raw-final".to_string());
    }

    #[test]
    fn data_plane_overflow_emits_session_gap() {
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        let (data_tx, _data_rx) = mpsc::channel(1);
        let router = CrpEventRouter::new(control_tx, data_tx);

        let data_event = CrpEvent::MessageDelta {
            session_id: "session-1".to_string(),
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
