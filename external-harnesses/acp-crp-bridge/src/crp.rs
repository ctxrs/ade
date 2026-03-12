use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufWriter};

const CRP_VERSION: u32 = 1;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum CrpCommand {
    #[serde(rename = "session.open")]
    SessionOpen {
        session_id: Option<String>,
        #[serde(default)]
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        session_id: Option<String>,
        turn_id: Option<String>,
        #[serde(default)]
        items: Option<Vec<Value>>,
        prompt: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.cancel")]
    SessionCancel {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "session.authenticate")]
    SessionAuthenticate {
        session_id: Option<String>,
        #[serde(default)]
        method_id: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        #[serde(default)]
        config: Option<CrpSessionConfig>,
    },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct CrpSessionConfig {
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrpMcpServerConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    #[allow(dead_code)]
    pub tool_timeout_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpChannel {
    Control,
    Data,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum CrpEvent {
    #[serde(rename = "session.opened")]
    SessionOpened {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        models: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_model_id: Option<String>,
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
        #[serde(default)]
        catalog_source: Option<String>,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CrpModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrpToolStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrpEnvelope {
    pub channel: CrpChannel,
    #[serde(flatten)]
    pub event: CrpEvent,
}

#[derive(Debug, Serialize)]
struct CrpEventEnvelope<'a> {
    v: u32,
    seq: u64,
    channel: &'a CrpChannel,
    #[serde(flatten)]
    event: &'a CrpEvent,
}

pub struct CrpWriter<W> {
    writer: BufWriter<W>,
    seq: u64,
}

impl<W> CrpWriter<W>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    pub fn new(writer: W) -> Self {
        Self {
            writer: BufWriter::new(writer),
            seq: 0,
        }
    }

    pub async fn send(&mut self, envelope: &CrpEnvelope) -> Result<()> {
        self.seq = self.seq.saturating_add(1);
        let payload = CrpEventEnvelope {
            v: CRP_VERSION,
            seq: self.seq,
            channel: &envelope.channel,
            event: &envelope.event,
        };
        let line = serde_json::to_string(&payload)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}
