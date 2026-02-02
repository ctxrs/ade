use codex_protocol::config_types::SandboxMode;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::user_input::UserInput;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct CrpCommandEnvelope {
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
        items: Option<Vec<UserInput>>,
        model: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<ReasoningEffort>,
        cwd: Option<PathBuf>,
    },
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

#[derive(Debug, Deserialize)]
pub struct CrpSessionConfig {
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub model_provider: Option<String>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox_mode: Option<SandboxMode>,
    pub reasoning_trace_enabled: Option<bool>,
    pub mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Deserialize)]
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


#[derive(Debug, Serialize)]
pub struct CrpEventEnvelope {
    pub v: u32,
    pub seq: u64,
    pub channel: CrpChannel,
    #[serde(flatten)]
    pub event: CrpEvent,
}

#[derive(Debug, Serialize, PartialEq)]
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
    },
    #[serde(rename = "session.gap")]
    SessionGap {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted {
        session_id: String,
        turn_id: String,
    },
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
    /// Reasoning summary (control plane only).
    #[serde(rename = "reasoning.summary")]
    ReasoningSummary {
        session_id: String,
        turn_id: String,
        /// Codex can stream multiple reasoning summary blocks; this identifies which block.
        /// Consumers can use it to pick the latest block title for status displays.
        #[serde(default)]
        summary_index: i64,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    /// Raw reasoning trace (data plane only).
    #[serde(rename = "reasoning.trace")]
    ReasoningTrace {
        session_id: String,
        turn_id: String,
        chunk: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        encoding: Option<String>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        session_id: String,
        turn_id: String,
        status: CrpTurnStatus,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_label: Option<String>,
        input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_preview: Option<Value>,
    },
    #[serde(rename = "tool.request")]
    ToolRequest {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        input: Option<Value>,
    },
    #[serde(rename = "tool.output.delta")]
    ToolOutputDelta {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
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
        output: Option<Value>,
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_preview: Option<Value>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        models: Vec<CrpModelInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_model_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpTurnStatus {
    Success,
    Error,
    Canceled,
    Interrupted,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpToolStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpToolOutputStream {
    Stdout,
    Stderr,
    Stdin,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_types_use_dotted_names() {
        let json = r#"{"type":"session.open","session_id":"s1"}"#;
        let cmd: CrpCommand = serde_json::from_str(json).unwrap();
        match cmd {
            CrpCommand::SessionOpen { session_id, .. } => {
                assert_eq!(session_id.as_deref(), Some("s1"));
            }
            _ => panic!("expected session.open command"),
        }
    }

    #[test]
    fn command_types_include_tool_result() {
        let json = r#"{"type":"tool.result","tool_call_id":"tool-1","status":"success","output":"ok"}"#;
        let cmd: CrpCommand = serde_json::from_str(json).unwrap();
        match cmd {
            CrpCommand::ToolResult {
                tool_call_id,
                status: CrpToolStatus::Success,
                ..
            } => {
                assert_eq!(tool_call_id, "tool-1");
            }
            _ => panic!("expected tool.result command"),
        }
    }

    #[test]
    fn event_types_use_dotted_names() {
        let event = CrpEvent::MessageFinal {
            session_id: "session".to_string(),
            turn_id: "turn".to_string(),
            message_id: "message".to_string(),
            content: "hello".to_string(),
        };
        let value = serde_json::to_value(event).unwrap();
        let kind = value.get("type").and_then(|value| value.as_str());
        assert_eq!(kind, Some("message.final"));
    }

    #[test]
    fn tool_event_types_use_dotted_names() {
        let event = CrpEvent::ToolCompleted {
            session_id: "session".to_string(),
            turn_id: "turn".to_string(),
            tool_call_id: "tool".to_string(),
            tool_name: "exec".to_string(),
            tool_label: None,
            status: CrpToolStatus::Success,
            output: None,
            error: None,
            input_preview: None,
        };
        let value = serde_json::to_value(event).unwrap();
        let kind = value.get("type").and_then(|value| value.as_str());
        assert_eq!(kind, Some("tool.completed"));
    }

    #[test]
    fn tool_request_event_uses_dotted_name() {
        let event = CrpEvent::ToolRequest {
            session_id: "session".to_string(),
            turn_id: "turn".to_string(),
            tool_call_id: "tool".to_string(),
            tool_name: "exec".to_string(),
            input: None,
        };
        let value = serde_json::to_value(event).unwrap();
        let kind = value.get("type").and_then(|value| value.as_str());
        assert_eq!(kind, Some("tool.request"));
    }

    #[test]
    fn reasoning_event_types_use_dotted_names() {
        let summary = CrpEvent::ReasoningSummary {
            session_id: "session".to_string(),
            turn_id: "turn".to_string(),
            summary_index: 0,
            text: "summary".to_string(),
            item_id: None,
        };
        let value = serde_json::to_value(summary).unwrap();
        let kind = value.get("type").and_then(|value| value.as_str());
        assert_eq!(kind, Some("reasoning.summary"));

        let trace = CrpEvent::ReasoningTrace {
            session_id: "session".to_string(),
            turn_id: "turn".to_string(),
            chunk: "trace".to_string(),
            encoding: None,
        };
        let value = serde_json::to_value(trace).unwrap();
        let kind = value.get("type").and_then(|value| value.as_str());
        assert_eq!(kind, Some("reasoning.trace"));
    }
}
