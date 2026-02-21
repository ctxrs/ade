use std::collections::HashMap;

use agent_client_protocol::{
    ContentBlock, EmbeddedResourceResource, SessionUpdate, ToolCall, ToolCallStatus,
    ToolCallUpdate, ToolKind,
};
use serde_json::Value;

use crate::crp::{CrpChannel, CrpEnvelope, CrpEvent, CrpToolStatus};

#[derive(Clone, Copy, Debug)]
pub enum ReasoningMode {
    RawThoughts,
    Omit,
    SummaryIfAvailable,
}

#[derive(Debug, Clone)]
struct ToolCache {
    tool_name: String,
    tool_label: Option<String>,
    input: Option<Value>,
    input_preview: Option<Value>,
}

#[derive(Debug)]
pub struct Translator {
    session_id: String,
    reasoning_mode: ReasoningMode,
    turn_id: Option<String>,
    message_id: Option<String>,
    message_buffer: String,
    tool_cache: HashMap<String, ToolCache>,
}

impl Translator {
    pub fn new(session_id: impl Into<String>, reasoning_mode: ReasoningMode) -> Self {
        Self {
            session_id: session_id.into(),
            reasoning_mode,
            turn_id: None,
            message_id: None,
            message_buffer: String::new(),
            tool_cache: HashMap::new(),
        }
    }

    pub fn start_turn(&mut self, turn_id: impl Into<String>, message_id: impl Into<String>) {
        self.turn_id = Some(turn_id.into());
        self.message_id = Some(message_id.into());
        self.message_buffer.clear();
    }

    pub fn finish_turn(&mut self) -> Option<CrpEnvelope> {
        let (turn_id, message_id) = match (self.turn_id.as_ref(), self.message_id.as_ref()) {
            (Some(turn_id), Some(message_id)) => (turn_id.clone(), message_id.clone()),
            _ => return None,
        };

        if self.message_buffer.is_empty() {
            return None;
        }

        let event = CrpEvent::MessageFinal {
            session_id: self.session_id.clone(),
            turn_id,
            message_id,
            content: self.message_buffer.clone(),
        };

        Some(CrpEnvelope {
            channel: CrpChannel::Control,
            event,
        })
    }

    pub fn clear_turn(&mut self) {
        self.turn_id = None;
        self.message_id = None;
        self.message_buffer.clear();
    }

    pub fn has_buffered_message(&self) -> bool {
        !self.message_buffer.is_empty()
    }

    pub fn apply_update(&mut self, update: SessionUpdate) -> Vec<CrpEnvelope> {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => self.handle_message_chunk(chunk.content),
            SessionUpdate::AgentThoughtChunk(chunk) => self.handle_thought_chunk(chunk.content),
            SessionUpdate::ToolCall(tool_call) => self.handle_tool_call(tool_call),
            SessionUpdate::ToolCallUpdate(update) => self.handle_tool_update(update),
            _ => Vec::new(),
        }
    }

    fn handle_message_chunk(&mut self, content: ContentBlock) -> Vec<CrpEnvelope> {
        let text = match content_block_to_text(&content) {
            Some(text) if !text.is_empty() => text,
            _ => return Vec::new(),
        };
        let (turn_id, message_id) = match self.active_ids() {
            Some(ids) => ids,
            None => return Vec::new(),
        };

        self.message_buffer.push_str(&text);

        vec![CrpEnvelope {
            channel: CrpChannel::Data,
            event: CrpEvent::MessageDelta {
                session_id: self.session_id.clone(),
                turn_id,
                message_id,
                delta: text,
            },
        }]
    }

    fn handle_thought_chunk(&mut self, content: ContentBlock) -> Vec<CrpEnvelope> {
        if matches!(self.reasoning_mode, ReasoningMode::Omit) {
            return Vec::new();
        }
        let text = match content_block_to_text(&content) {
            Some(text) if !text.is_empty() => text,
            _ => return Vec::new(),
        };
        let (turn_id, _message_id) = match self.active_ids() {
            Some(ids) => ids,
            None => return Vec::new(),
        };

        vec![CrpEnvelope {
            channel: CrpChannel::Data,
            event: CrpEvent::ReasoningTrace {
                session_id: self.session_id.clone(),
                turn_id,
                chunk: text,
                encoding: None,
            },
        }]
    }

    fn handle_tool_call(&mut self, tool_call: ToolCall) -> Vec<CrpEnvelope> {
        let (turn_id, _message_id) = match self.active_ids() {
            Some(ids) => ids,
            None => return Vec::new(),
        };
        let tool_id = tool_call.tool_call_id.to_string();
        let tool_name = tool_kind_name(tool_call.kind).to_string();
        let tool_label = Some(tool_call.title.clone());

        let input = tool_call.raw_input.clone();
        let input_preview = tool_call.raw_input.clone();

        self.tool_cache.insert(
            tool_id.clone(),
            ToolCache {
                tool_name: tool_name.clone(),
                tool_label: tool_label.clone(),
                input,
                input_preview,
            },
        );

        vec![CrpEnvelope {
            channel: CrpChannel::Control,
            event: CrpEvent::ToolStarted {
                session_id: self.session_id.clone(),
                turn_id,
                tool_call_id: tool_id,
                tool_name,
                tool_label,
                input: tool_call.raw_input.clone(),
                input_preview: tool_call.raw_input,
            },
        }]
    }

    fn handle_tool_update(&mut self, update: ToolCallUpdate) -> Vec<CrpEnvelope> {
        let (turn_id, _message_id) = match self.active_ids() {
            Some(ids) => ids,
            None => return Vec::new(),
        };
        let tool_id = update.tool_call_id.to_string();

        let entry = self
            .tool_cache
            .entry(tool_id.clone())
            .or_insert_with(|| ToolCache {
                tool_name: "other".to_string(),
                tool_label: None,
                input: None,
                input_preview: None,
            });

        if let Some(kind) = update.fields.kind {
            entry.tool_name = tool_kind_name(kind).to_string();
        }
        if let Some(title) = update.fields.title {
            entry.tool_label = Some(title);
        }
        if let Some(raw_input) = update.fields.raw_input.clone() {
            entry.input = Some(raw_input.clone());
            entry.input_preview = Some(raw_input);
        }

        let mut events = Vec::new();
        let status = update.fields.status;

        if let Some(raw_output) = update.fields.raw_output.clone() {
            let should_emit_delta = !matches!(
                status,
                Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
            );
            if should_emit_delta {
                let chunk = render_value(&raw_output);
                if !chunk.is_empty() {
                    events.push(CrpEnvelope {
                        channel: CrpChannel::Data,
                        event: CrpEvent::ToolOutputDelta {
                            session_id: self.session_id.clone(),
                            turn_id: turn_id.clone(),
                            tool_call_id: tool_id.clone(),
                            chunk,
                        },
                    });
                }
            }
        }

        if let Some(status) = status {
            match status {
                ToolCallStatus::Completed | ToolCallStatus::Failed => {
                    let crp_status = if matches!(status, ToolCallStatus::Failed) {
                        CrpToolStatus::Error
                    } else {
                        CrpToolStatus::Success
                    };

                    let output = update.fields.raw_output.clone();
                    let error = if matches!(status, ToolCallStatus::Failed) {
                        update
                            .fields
                            .raw_output
                            .as_ref()
                            .map(render_value)
                            .filter(|value| !value.is_empty())
                    } else {
                        None
                    };

                    events.push(CrpEnvelope {
                        channel: CrpChannel::Control,
                        event: CrpEvent::ToolCompleted {
                            session_id: self.session_id.clone(),
                            turn_id,
                            tool_call_id: tool_id,
                            tool_name: entry.tool_name.clone(),
                            tool_label: entry.tool_label.clone(),
                            status: crp_status,
                            output,
                            error,
                            input_preview: entry.input_preview.clone(),
                        },
                    });
                }
                _ => {}
            }
        }

        events
    }

    fn active_ids(&self) -> Option<(String, String)> {
        Some((self.turn_id.clone()?, self.message_id.clone()?))
    }
}

fn tool_kind_name(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        ToolKind::Other => "other",
        _ => "other",
    }
}

fn content_block_to_text(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text.clone()),
        ContentBlock::ResourceLink(link) => Some(link.uri.clone()),
        ContentBlock::Resource(resource) => match &resource.resource {
            EmbeddedResourceResource::TextResourceContents(text) => Some(text.text.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        session_id: String,
        turn_id: String,
        message_id: String,
        updates: Vec<Value>,
        expected: Vec<Value>,
        final_event: Value,
    }

    #[test]
    fn translate_basic_fixture() {
        let raw = std::fs::read_to_string("fixtures/basic.json").expect("fixture read");
        let fixture: Fixture = serde_json::from_str(&raw).expect("fixture parse");

        let mut translator =
            Translator::new(fixture.session_id.clone(), ReasoningMode::RawThoughts);
        translator.start_turn(fixture.turn_id.clone(), fixture.message_id.clone());

        let mut out = Vec::new();
        for update in fixture.updates {
            let update: SessionUpdate = serde_json::from_value(update).expect("update parse");
            out.extend(translator.apply_update(update));
        }

        if let Some(final_event) = translator.finish_turn() {
            out.push(final_event);
        }

        let actual: Vec<Value> = out
            .into_iter()
            .map(|env| serde_json::to_value(env).expect("serialize"))
            .collect();

        let mut expected = fixture.expected;
        expected.push(fixture.final_event);

        assert_eq!(actual, expected);
    }
}
