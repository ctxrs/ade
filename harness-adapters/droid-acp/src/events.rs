use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DroidEvent {
    System {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        tools: Option<Vec<String>>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        subtype: Option<String>,
    },
    Message {
        role: String,
        text: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    ToolCall {
        id: String,
        #[serde(default, rename = "toolId")]
        tool_id: Option<String>,
        #[serde(default, rename = "toolName")]
        tool_name: Option<String>,
        #[serde(default)]
        parameters: Option<Value>,
        #[serde(default, rename = "messageId")]
        message_id: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    ToolResult {
        id: String,
        #[serde(default, rename = "toolId")]
        tool_id: Option<String>,
        #[serde(default, rename = "isError")]
        is_error: Option<bool>,
        #[serde(default)]
        value: Option<Value>,
        #[serde(default, rename = "messageId")]
        message_id: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    Completion {
        #[serde(default, rename = "finalText")]
        final_text: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    Error {
        #[serde(default)]
        message: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct DroidEventEnvelope {
    pub raw: Value,
    pub event: Option<DroidEvent>,
}

fn extract_event_value(value: &Value) -> Option<Value> {
    if value.get("type").is_some() {
        return Some(value.clone());
    }

    let params = value.get("params");
    if let Some(params) = params {
        if params.get("type").is_some() {
            return Some(params.clone());
        }
        if let Some(event) = params.get("event") {
            if event.get("type").is_some() {
                return Some(event.clone());
            }
        }
    }

    let result = value.get("result");
    if let Some(result) = result {
        if result.get("type").is_some() {
            return Some(result.clone());
        }
        if let Some(event) = result.get("event") {
            if event.get("type").is_some() {
                return Some(event.clone());
            }
        }
    }

    None
}

pub fn parse_event(raw: Value) -> Option<DroidEventEnvelope> {
    if let Some(error) = raw.get("error") {
        let message = error
            .get("message")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        return Some(DroidEventEnvelope {
            raw,
            event: Some(DroidEvent::Error { message }),
        });
    }

    let event_value = extract_event_value(&raw)?;
    let parsed = serde_json::from_value::<DroidEvent>(event_value).ok();

    Some(DroidEventEnvelope {
        raw,
        event: parsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_json_diff::assert_json_eq;
    use serde_json::json;

    #[test]
    fn parses_tool_call_event() {
        let raw = json!({
            "type": "tool_call",
            "id": "call-1",
            "toolId": "Execute",
            "toolName": "Execute",
            "parameters": {"command": "ls"}
        });

        let envelope = parse_event(raw.clone()).expect("envelope");
        let DroidEvent::ToolCall {
            id,
            tool_id,
            tool_name,
            parameters,
            ..
        } = envelope.event.expect("event")
        else {
            panic!("unexpected event");
        };

        assert_eq!(id, "call-1");
        assert_eq!(tool_id.as_deref(), Some("Execute"));
        assert_eq!(tool_name.as_deref(), Some("Execute"));
        assert_json_eq!(parameters.unwrap(), json!({"command": "ls"}));
    }

    #[test]
    fn parses_jsonrpc_wrapped_event() {
        let raw = json!({
            "jsonrpc": "2.0",
            "method": "event",
            "params": {
                "type": "message",
                "role": "assistant",
                "text": "hello"
            }
        });

        let envelope = parse_event(raw).expect("envelope");
        let DroidEvent::Message { role, text, .. } = envelope.event.expect("event") else {
            panic!("unexpected event");
        };

        assert_eq!(role, "assistant");
        assert_eq!(text, "hello");
    }
}
