
pub(crate) fn normalize_session_update(
    msg: &serde_json::Value,
    state: &mut StreamState,
) -> Vec<NormalizedEvent> {
    let Some(params) = msg.get("params") else {
        return vec![];
    };
    let update = params.get("update").cloned().unwrap_or(json!({}));
    let context_window = extract_update_field(&update, "context_window");
    let usage = extract_update_field(&update, "usage");
    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    match kind {
        "agent_message_chunk" => {
            let mut out = Vec::new();
            if let Some(content) = update.get("content") {
                if let Some(text) = content_text(content) {
                    state.assistant_buf.push_str(&text);
                    let mut payload = Map::new();
                    payload.insert("content_fragment".to_string(), json!(text));
                    payload.insert("acp_update".to_string(), update.clone());
                    add_update_meta_fields(&mut payload, &context_window, &usage);
                    out.push(NormalizedEvent {
                        event_type: SessionEventType::AssistantChunk,
                        payload_json: Value::Object(payload),
                    });
                }
            }
            out
        }
        "agent_thought_chunk" => {
            if let Some(content) = update.get("content") {
                if let Some(text) = content_text(content) {
                    let mut payload = Map::new();
                    payload.insert("content_fragment".to_string(), json!(text));
                    payload.insert("acp_update".to_string(), update.clone());
                    add_update_meta_fields(&mut payload, &context_window, &usage);
                    return vec![NormalizedEvent {
                        event_type: SessionEventType::ThoughtChunk,
                        payload_json: Value::Object(payload),
                    }];
                }
            }
            vec![]
        }
        "agent_message" => {
            let mut out = Vec::new();
            if let Some(chunks) = update.get("content").and_then(|v| v.as_array()) {
                for block in chunks {
                    if let Some(text) = content_text(block) {
                        state.assistant_buf.push_str(&text);
                        let mut payload = Map::new();
                        payload.insert("content_fragment".to_string(), json!(text));
                        payload.insert("acp_update".to_string(), update.clone());
                        add_update_meta_fields(&mut payload, &context_window, &usage);
                        out.push(NormalizedEvent {
                            event_type: SessionEventType::AssistantChunk,
                            payload_json: Value::Object(payload),
                        });
                    }
                }
            }
            out
        }
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
                .or_else(|| update.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .or_else(|| {
                    update
                        .pointer("/rawInput/call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            update
                                .pointer("/raw_input/call_id")
                                .and_then(|v| v.as_str())
                        })
                        .map(|v| v.to_string())
                });
            let mut payload = Map::new();
            if let Some(tool_call_id) = tool_call_id {
                payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            }
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolCall,
                payload_json: Value::Object(payload),
            }]
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
                .or_else(|| update.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .or_else(|| {
                    update
                        .pointer("/rawInput/call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            update
                                .pointer("/raw_input/call_id")
                                .and_then(|v| v.as_str())
                        })
                        .map(|v| v.to_string())
                });
            let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = Map::new();
            if let Some(ref tool_call_id) = tool_call_id {
                payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            }
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            let mut out = vec![NormalizedEvent {
                event_type: SessionEventType::ToolCallUpdate,
                payload_json: Value::Object(payload),
            }];
            if matches!(status, "completed" | "failed") {
                let mut payload = Map::new();
                if let Some(ref tool_call_id) = tool_call_id {
                    payload.insert("tool_call_id".to_string(), json!(tool_call_id));
                }
                payload.insert("acp_update".to_string(), update.clone());
                add_update_meta_fields(&mut payload, &context_window, &usage);
                out.push(NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: Value::Object(payload),
                });
            }
            out
        }
        "plan" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Plan,
                payload_json: Value::Object(payload),
            }]
        }
        "available_commands_update" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: Value::Object(payload),
            }]
        }
        "error" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: Value::Object(payload),
            }]
        }
        _ => vec![],
    }
}

/// Offline transcript replay helpers used by contract tests.
///
/// These helpers intentionally avoid spawning any real provider binaries; they replay captured (or
/// synthetic) ACP `session/update` notifications plus the final `session/prompt` response through
/// the same normalization logic used at runtime.
pub mod transcript {
    use std::path::Path;

    use anyhow::{Context, Result};
    use serde_json::{json, Value};

    use ctx_core::models::SessionEventType;

    use super::StreamState;
    use super::{
        is_auth_required_error, jsonrpc_id_u64, normalize_session_update, NormalizedEvent,
    };

    pub const TRANSCRIPT_VERSION: u32 = 1;

    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct AcpTranscript {
        pub version: u32,
        pub provider: String,
        pub case: String,
        #[serde(default)]
        pub description: Option<String>,
        pub events: Vec<AcpTranscriptEvent>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum AcpTranscriptEvent {
        SessionUpdate {
            msg: Value,
        },
        PromptResponse {
            msg: Value,
        },
        #[serde(rename = "note")]
        Note {
            text: String,
        },
    }

    pub fn load_transcript(path: impl AsRef<Path>) -> Result<AcpTranscript> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading transcript {}", path.display()))?;
        let transcript: AcpTranscript =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(transcript)
    }

    fn extract_session_id(msg: &Value) -> Option<String> {
        msg.get("params")
            .and_then(|p| p.get("sessionId").or_else(|| p.get("session_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn extract_stop_reason(prompt_response: &Value) -> String {
        prompt_response
            .get("result")
            .and_then(|v| v.get("stopReason"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    }

    pub fn replay_transcript(transcript: &AcpTranscript) -> Result<Vec<NormalizedEvent>> {
        anyhow::ensure!(
            transcript.version == TRANSCRIPT_VERSION,
            "unsupported transcript version {}; expected {}",
            transcript.version,
            TRANSCRIPT_VERSION
        );

        let mut state = StreamState::default();
        let mut out: Vec<NormalizedEvent> = Vec::new();
        let mut prompt_response: Option<Value> = None;
        let mut session_id: Option<String> = None;

        for ev in &transcript.events {
            match ev {
                AcpTranscriptEvent::SessionUpdate { msg } => {
                    if session_id.is_none() {
                        session_id = extract_session_id(msg);
                    }
                    out.extend(normalize_session_update(msg, &mut state));
                }
                AcpTranscriptEvent::PromptResponse { msg } => {
                    if prompt_response.is_some() {
                        anyhow::bail!("transcript contains more than one prompt_response");
                    }
                    prompt_response = Some(msg.clone());
                }
                AcpTranscriptEvent::Note { .. } => {}
            }
        }

        let prompt_response = prompt_response.context("transcript missing prompt_response")?;

        if let Some(err) = prompt_response.get("error") {
            if is_auth_required_error(err) {
                out.push(NormalizedEvent {
                    event_type: SessionEventType::AuthRequired,
                    payload_json: json!({
                        "kind": "auth_required",
                        "provider": transcript.provider,
                        "message": "Provider requires authentication to continue.",
                        "acp_error": err,
                    }),
                });
            }
            out.push(NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: json!({
                    "provider": transcript.provider,
                    "acp_error": err,
                }),
            });
        }

        if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
            state.saw_assistant_complete = true;
            out.push(NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({ "full_content": state.assistant_buf }),
            });
        }

        // Best-effort extraction: if the response has an `id`, preserve it for debugging.
        let response_id = prompt_response.get("id").and_then(jsonrpc_id_u64);
        out.push(NormalizedEvent {
            event_type: SessionEventType::Done,
            payload_json: json!({
                "provider": transcript.provider,
                "acp_session_id": session_id,
                "status": if prompt_response.get("error").is_some() { "error" } else { "success" },
                "stop_reason": extract_stop_reason(&prompt_response),
                "acp_response_id": response_id,
                "transcript_case": transcript.case,
            }),
        });

        Ok(out)
    }
}

fn jsonrpc_id_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}
