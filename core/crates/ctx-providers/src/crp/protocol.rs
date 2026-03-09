use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(super) struct CrpCommandEnvelope {
    pub(super) v: u32,
    #[serde(flatten)]
    pub(super) command: CrpCommand,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
pub(super) enum CrpCommand {
    #[serde(rename = "session.open")]
    SessionOpen {
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        session_id: Option<String>,
        turn_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        items: Option<Vec<Value>>,
        prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_effort: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
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
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
    #[serde(rename = "session.authenticate")]
    SessionAuthenticate {
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        method_id: Option<String>,
    },
    #[serde(rename = "session.cancel")]
    SessionCancel {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<CrpSessionConfig>,
    },
}

#[derive(Debug, Serialize)]
pub(super) struct CrpSessionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning_trace_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) personality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Serialize)]
pub(super) struct CrpMcpServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_timeout_sec: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrpModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CrpModelsProbe {
    pub models: Vec<CrpModelInfo>,
    pub current_model_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct CrpEventEnvelope {
    #[allow(dead_code)]
    #[serde(default)]
    pub(super) v: Option<u32>,
    pub(super) seq: u64,
    #[allow(dead_code)]
    pub(super) channel: CrpChannel,
    #[serde(flatten)]
    pub(super) event: CrpEvent,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CrpChannel {
    Control,
    Data,
}

#[allow(dead_code)]
// EXCEPTION: session.opened carries provider-specific metadata blobs; boxing this
// protocol parser enum right before release would add churn without product value.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub(super) enum CrpEvent {
    #[serde(rename = "session.opened")]
    SessionOpened {
        session_id: String,
        provider_session_id: Option<String>,
        #[serde(default)]
        commands: Option<Value>,
        #[serde(default)]
        slash_commands: Option<Vec<String>>,
        #[serde(default)]
        models: Option<Value>,
        #[serde(default)]
        current_model_id: Option<String>,
        #[serde(default)]
        agents: Option<Value>,
        #[serde(default)]
        output_style: Option<String>,
        #[serde(default)]
        available_output_styles: Option<Vec<String>>,
        #[serde(default)]
        skills: Option<Vec<String>>,
        #[serde(default)]
        plugins: Option<Value>,
        #[serde(default)]
        tools: Option<Vec<String>>,
        #[serde(default)]
        permission_mode: Option<String>,
        #[serde(default)]
        mcp_servers: Option<Value>,
        #[serde(default)]
        account: Option<Value>,
        #[serde(default)]
        fast_mode_state: Option<String>,
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
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace")]
    ReasoningTrace {
        session_id: String,
        turn_id: String,
        chunk: String,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace.final")]
    ReasoningTraceFinal {
        session_id: String,
        turn_id: String,
        content: String,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        tool_label: Option<String>,
        #[serde(default)]
        input: Option<Value>,
        #[serde(default)]
        input_preview: Option<Value>,
    },
    #[serde(rename = "tool.output.delta")]
    ToolOutputDelta {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        #[serde(default)]
        chunk: String,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        tool_label: Option<String>,
        status: CrpToolStatus,
        #[serde(default)]
        output: Option<Value>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        input_preview: Option<Value>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        models: Vec<CrpModelInfo>,
        #[serde(default)]
        current_model_id: Option<String>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        session_id: String,
        turn_id: String,
        status: CrpTurnStatus,
        #[serde(default)]
        error: Option<CrpTurnError>,
    },
    #[serde(rename = "session.gap")]
    SessionGap {
        session_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "session.notice")]
    SessionNotice {
        session_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        code: String,
        #[serde(default)]
        severity: Option<String>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        details: Option<Value>,
        #[serde(default)]
        transient: Option<bool>,
    },
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub(super) enum CrpTurnStatus {
    Success,
    Error,
    Canceled,
    Interrupted,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct CrpTurnError {
    pub(super) message: String,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) details: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub(super) enum CrpToolStatus {
    Success,
    Error,
}
