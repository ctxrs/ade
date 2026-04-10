use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub const CRP_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct CrpCommandEnvelope {
    #[allow(dead_code)]
    #[serde(default)]
    pub v: Option<u32>,
    #[serde(flatten)]
    pub command: CrpCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
pub enum CrpCommand {
    #[serde(rename = "session.open")]
    SessionOpen {
        session_id: Option<String>,
        #[serde(default)]
        provider_session_id: Option<String>,
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        session_id: Option<String>,
        turn_id: Option<String>,
        prompt: Option<String>,
        items: Option<Vec<Value>>,
        model: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.compact")]
    SessionCompact {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "session.undo")]
    SessionUndo {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "session.review")]
    SessionReview {
        session_id: Option<String>,
        turn_id: Option<String>,
        instructions: Option<String>,
    },
    #[serde(rename = "session.set_model")]
    SessionSetModel {
        session_id: Option<String>,
        model_id: Option<String>,
    },
    #[serde(rename = "session.status")]
    SessionStatus { session_id: Option<String> },
    #[serde(rename = "session.cancel")]
    SessionCancel {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        #[serde(default)]
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "tool.result")]
    ToolResult {
        session_id: Option<String>,
        turn_id: Option<String>,
        tool_call_id: String,
        status: CrpToolStatus,
        output: Option<Value>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CrpSessionConfig {
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub spawn_cwd: Option<PathBuf>,
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    pub model_provider: Option<String>,
    pub approval_policy: Option<String>,
    pub sandbox_mode: Option<String>,
    pub reasoning_trace_enabled: Option<bool>,
    pub personality: Option<String>,
    pub mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CrpMcpServerConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub env_vars: Option<Vec<String>>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub http_headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub env_http_headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub disabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub tool_timeout_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrpModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrpCommandInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CrpEventEnvelope {
    pub v: u32,
    pub seq: u64,
    pub channel: CrpChannel,
    #[serde(flatten)]
    pub event: CrpEvent,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrpChannel {
    Control,
    Data,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum CrpEvent {
    #[serde(rename = "session.opened")]
    SessionOpened {
        session_id: String,
        provider_session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        commands: Option<Vec<CrpCommandInfo>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        slash_commands: Option<Vec<String>>,
    },
    #[serde(rename = "session.gap")]
    SessionGap {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "session.notice")]
    SessionNotice {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        severity: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        transient: Option<bool>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted { session_id: String, turn_id: String },
    #[serde(rename = "message.delta")]
    MessageDelta {
        session_id: String,
        turn_id: String,
        message_id: String,
        delta: String,
    },
    #[serde(rename = "message.final")]
    MessageFinal {
        session_id: String,
        turn_id: String,
        message_id: String,
        content: String,
    },
    #[serde(rename = "reasoning.summary")]
    ReasoningSummary {
        session_id: String,
        turn_id: String,
        #[serde(default)]
        summary_index: i64,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace")]
    ReasoningTrace {
        session_id: String,
        turn_id: String,
        chunk: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace.final")]
    ReasoningTraceFinal {
        session_id: String,
        turn_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    #[serde(rename = "turn.context_window.updated")]
    TurnContextWindowUpdated {
        session_id: String,
        turn_id: String,
        context_window: Value,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        session_id: String,
        turn_id: String,
        status: CrpTurnStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_window: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<CrpTurnError>,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_preview: Option<Value>,
    },
    #[serde(rename = "tool.output_delta")]
    ToolOutputDelta {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream: Option<CrpToolOutputStream>,
        chunk: String,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_label: Option<String>,
        status: CrpToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_preview: Option<Value>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        models: Vec<CrpModelInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_model_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        catalog_source: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpTurnStatus {
    Success,
    Error,
    Canceled,
    Interrupted,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrpTurnError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpToolStatus {
    Success,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpToolOutputStream {
    Stdout,
    Stderr,
    Stdin,
}
