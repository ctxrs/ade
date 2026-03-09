use std::collections::HashMap;

use serde_json::{json, Value};

use ctx_core::models::SessionEventType;

use crate::events::NormalizedEvent;

use super::protocol::{CrpChannel, CrpEvent, CrpToolStatus, CrpTurnStatus};

pub(super) struct MappedCrpEvent {
    pub(super) events: Vec<NormalizedEvent>,
    pub(super) done: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CachedToolInput {
    pub(super) input: Option<Value>,
    pub(super) input_preview: Option<Value>,
}

pub(super) fn map_crp_event(
    event: CrpEvent,
    channel: CrpChannel,
    seq: u64,
    tool_output_cache: &mut HashMap<String, String>,
    tool_input_cache: &mut HashMap<String, CachedToolInput>,
) -> MappedCrpEvent {
    let crp_channel = match channel {
        CrpChannel::Data => Some("data"),
        CrpChannel::Control => None,
    };
    match event {
        CrpEvent::SessionOpened {
            session_id,
            provider_session_id,
            commands,
            slash_commands,
            models,
            current_model_id,
            agents,
            output_style,
            available_output_styles,
            skills,
            plugins,
            tools,
            permission_mode,
            mcp_servers,
            account,
            fast_mode_state,
        } => {
            let mut payload = serde_json::Map::new();
            payload.insert("session_id".to_string(), json!(session_id));
            if let Some(provider_session_id) = provider_session_id {
                payload.insert(
                    "provider_session_id".to_string(),
                    json!(provider_session_id),
                );
            }
            if let Some(commands) = commands {
                payload.insert("commands".to_string(), commands);
            }
            if let Some(slash_commands) = slash_commands {
                payload.insert("slash_commands".to_string(), json!(slash_commands));
            }
            if let Some(models) = models {
                payload.insert("models".to_string(), models);
            }
            if let Some(current_model_id) = current_model_id {
                payload.insert("current_model_id".to_string(), json!(current_model_id));
            }
            if let Some(agents) = agents {
                payload.insert("agents".to_string(), agents);
            }
            if let Some(output_style) = output_style {
                payload.insert("output_style".to_string(), json!(output_style));
            }
            if let Some(available_output_styles) = available_output_styles {
                payload.insert(
                    "available_output_styles".to_string(),
                    json!(available_output_styles),
                );
            }
            if let Some(skills) = skills {
                payload.insert("skills".to_string(), json!(skills));
            }
            if let Some(plugins) = plugins {
                payload.insert("plugins".to_string(), plugins);
            }
            if let Some(tools) = tools {
                payload.insert("tools".to_string(), json!(tools));
            }
            if let Some(permission_mode) = permission_mode {
                payload.insert("permission_mode".to_string(), json!(permission_mode));
            }
            if let Some(mcp_servers) = mcp_servers {
                payload.insert("mcp_servers".to_string(), mcp_servers);
            }
            if let Some(account) = account {
                payload.insert("account".to_string(), account);
            }
            if let Some(fast_mode_state) = fast_mode_state {
                payload.insert("fast_mode_state".to_string(), json!(fast_mode_state));
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::Init,
                    payload_json: Value::Object(payload),
                }],
                done: false,
            }
        }
        // The scheduler already emits the canonical turn lifecycle events. Treat harness-emitted
        // `turn.started` as internal signal only to avoid duplicating `turn_started` rows with a
        // mismatched payload shape.
        CrpEvent::TurnStarted { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::MessageDelta {
            delta, message_id, ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({
                    "content_fragment": delta,
                    "message_id": message_id,
                    "crp_seq": seq,
                    "crp_channel": crp_channel,
                }),
            }],
            done: false,
        },
        CrpEvent::MessageFinal {
            content,
            message_id,
            ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({
                    "full_content": content,
                    "message_id": message_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningSummary {
            text,
            item_id,
            summary_index,
            ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "reasoning_summary",
                    "summary_index": summary_index,
                    "text": text,
                    "item_id": item_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningTrace {
            chunk,
            encoding,
            summary_index,
            item_id,
            ..
        } => {
            let mut payload = json!({
                "content_fragment": chunk,
                "encoding": encoding,
                "summary_index": summary_index,
                "item_id": item_id,
                "crp_seq": seq,
            });
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ThoughtChunk,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ReasoningTraceFinal {
            content,
            encoding,
            summary_index,
            item_id,
            ..
        } => {
            let mut payload = json!({
                "content_fragment": "",
                "full_content": content,
                "is_final": true,
                "encoding": encoding,
                "summary_index": summary_index,
                "item_id": item_id,
                "crp_seq": seq,
            });
            // item_id enables deterministic thought chunk grouping/deduping in clients.
            if let Some(item_id) = item_id {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("item_id".to_string(), json!(item_id));
                }
            }
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ThoughtChunk,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolStarted {
            tool_call_id,
            tool_name,
            tool_label,
            input,
            input_preview,
            ..
        } => {
            if input.is_some() || input_preview.is_some() {
                tool_input_cache.insert(
                    tool_call_id.clone(),
                    CachedToolInput {
                        input: input.clone(),
                        input_preview: input_preview.clone(),
                    },
                );
            }
            let payload = build_tool_started_payload(
                tool_call_id,
                tool_name,
                tool_label,
                input,
                input_preview,
                seq,
            );
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolOutputDelta {
            tool_call_id,
            chunk,
            ..
        } => {
            let output_text = {
                let entry = tool_output_cache.entry(tool_call_id.clone()).or_default();
                entry.push_str(&chunk);
                entry.clone()
            };
            let mut payload = json!({
                "tool_call_id": tool_call_id,
                "outputText": output_text,
                "status": "running",
                "crp_seq": seq,
            });
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCallUpdate,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolCompleted {
            tool_call_id,
            tool_name,
            tool_label,
            status,
            output,
            error,
            input_preview,
            ..
        } => {
            let tool_call_id_for_cache = tool_call_id.clone();
            let cached = tool_input_cache.remove(&tool_call_id_for_cache);
            let (input, cached_preview) = cached
                .map(|c| (c.input, c.input_preview))
                .unwrap_or((None, None));
            let input_preview = input_preview.or(cached_preview);
            let payload = build_tool_completed_payload(
                tool_call_id,
                tool_name,
                tool_label,
                status,
                output,
                error,
                input,
                input_preview,
                seq,
            );
            tool_output_cache.remove(&tool_call_id_for_cache);
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::TurnCompleted { status, error, .. } => {
            let (event_type, payload) = match status {
                CrpTurnStatus::Success => (
                    SessionEventType::Done,
                    json!({"status": "completed", "crp_seq": seq}),
                ),
                CrpTurnStatus::Error => {
                    let message = error
                        .as_ref()
                        .map(|err| err.message.clone())
                        .unwrap_or_else(|| "crp_turn_error".to_string());
                    let kind = error.as_ref().and_then(|err| err.kind.clone());
                    let details = error.as_ref().and_then(|err| err.details.clone());
                    let mut payload = serde_json::Map::new();
                    payload.insert("message".to_string(), json!(message));
                    if let Some(kind) = kind {
                        payload.insert("kind".to_string(), json!(kind));
                    }
                    if let Some(details) = details {
                        payload.insert("details".to_string(), json!(details));
                    }
                    payload.insert("crp_seq".to_string(), json!(seq));
                    (SessionEventType::Error, Value::Object(payload))
                }
                CrpTurnStatus::Canceled | CrpTurnStatus::Interrupted => (
                    SessionEventType::TurnInterrupted,
                    json!({"reason": "crp_turn_interrupted", "crp_seq": seq}),
                ),
            };
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type,
                    payload_json: payload,
                }],
                done: true,
            }
        }
        CrpEvent::ModelsList { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::SessionNotice {
            code,
            severity,
            message,
            details,
            transient,
            ..
        } => {
            let mut payload = serde_json::Map::new();
            payload.insert("kind".to_string(), json!(code.clone()));
            payload.insert("code".to_string(), json!(code));
            if let Some(severity) = severity {
                payload.insert("severity".to_string(), json!(severity));
            }
            if let Some(message) = message {
                payload.insert("message".to_string(), json!(message));
            }
            if let Some(details) = details {
                if let Value::Object(map) = &details {
                    if let Some(auth_methods) =
                        map.get("auth_methods").or_else(|| map.get("authMethods"))
                    {
                        payload.insert("auth_methods".to_string(), auth_methods.clone());
                    }
                    if let Some(provider) = map.get("provider") {
                        payload.insert("provider".to_string(), provider.clone());
                    }
                }
                payload.insert("details".to_string(), details);
            }
            if let Some(transient) = transient {
                payload.insert("transient".to_string(), json!(transient));
            }
            payload.insert("crp_seq".to_string(), json!(seq));
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::Notice,
                    payload_json: Value::Object(payload),
                }],
                done: false,
            }
        }
        CrpEvent::SessionGap { reason, .. } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "session_gap",
                    "reason": reason,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
    }
}

fn build_tool_started_payload(
    tool_call_id: String,
    tool_name: String,
    tool_label: Option<String>,
    input: Option<Value>,
    input_preview: Option<Value>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    if let Some(label) = tool_label.as_ref() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    payload.insert("status".to_string(), json!("running"));
    let raw_input = input.clone().or_else(|| input_preview.clone());
    if let Some(input) = raw_input.clone() {
        payload.insert("rawInput".to_string(), input);
    }
    if let Some(input_preview) = input_preview.clone() {
        payload.insert("input_preview".to_string(), input_preview);
    }
    let mut tool_call_obj = serde_json::Map::new();
    tool_call_obj.insert("id".to_string(), json!(tool_call_id));
    tool_call_obj.insert("name".to_string(), json!(tool_name_for_call.clone()));
    tool_call_obj.insert("kind".to_string(), json!(tool_name_for_call));
    tool_call_obj.insert(
        "rawInput".to_string(),
        raw_input.clone().unwrap_or(Value::Null),
    );
    tool_call_obj.insert("status".to_string(), json!("running"));
    payload.insert("toolCall".to_string(), Value::Object(tool_call_obj));
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

#[allow(clippy::too_many_arguments)]
fn build_tool_completed_payload(
    tool_call_id: String,
    tool_name: String,
    tool_label: Option<String>,
    status: CrpToolStatus,
    output: Option<Value>,
    error: Option<String>,
    input: Option<Value>,
    input_preview: Option<Value>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    if let Some(label) = tool_label.as_ref() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    payload.insert(
        "status".to_string(),
        json!(match status {
            CrpToolStatus::Success => "completed",
            CrpToolStatus::Error => "failed",
        }),
    );
    let raw_input = input.clone().or_else(|| input_preview.clone());
    if let Some(input) = raw_input.clone() {
        payload.insert("rawInput".to_string(), input);
    }
    if let Some(input_preview) = input_preview.clone() {
        payload.insert("input_preview".to_string(), input_preview);
    }
    if let Some(output) = output.clone() {
        if let Some(text) = extract_output_text(&output) {
            payload.insert("output_text".to_string(), json!(text));
        }
        payload.insert("rawOutput".to_string(), output);
    }
    if let Some(err) = error {
        payload.insert("error".to_string(), json!(err));
    }
    if let Some(label) = tool_label.clone() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    let mut tool_call_obj = serde_json::Map::new();
    tool_call_obj.insert("id".to_string(), json!(tool_call_id));
    tool_call_obj.insert("name".to_string(), json!(tool_name_for_call.clone()));
    tool_call_obj.insert("kind".to_string(), json!(tool_name_for_call));
    tool_call_obj.insert(
        "rawInput".to_string(),
        raw_input.clone().unwrap_or(Value::Null),
    );
    tool_call_obj.insert(
        "rawOutput".to_string(),
        payload.get("rawOutput").cloned().unwrap_or(Value::Null),
    );
    tool_call_obj.insert(
        "status".to_string(),
        payload.get("status").cloned().unwrap_or(Value::Null),
    );
    payload.insert("toolCall".to_string(), Value::Object(tool_call_obj));
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

fn extract_output_text(output: &Value) -> Option<String> {
    if let Some(text) = output.get("formatted_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("aggregated_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("stdout").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("result").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    output.as_str().map(|text| text.to_string())
}

pub(super) fn event_turn_id(event: &CrpEvent) -> Option<&str> {
    match event {
        CrpEvent::SessionGap { turn_id, .. } => turn_id.as_deref(),
        CrpEvent::SessionNotice { turn_id, .. } => turn_id.as_deref(),
        CrpEvent::TurnStarted { turn_id, .. }
        | CrpEvent::MessageDelta { turn_id, .. }
        | CrpEvent::MessageFinal { turn_id, .. }
        | CrpEvent::ReasoningSummary { turn_id, .. }
        | CrpEvent::ReasoningTrace { turn_id, .. }
        | CrpEvent::ReasoningTraceFinal { turn_id, .. }
        | CrpEvent::ToolStarted { turn_id, .. }
        | CrpEvent::ToolOutputDelta { turn_id, .. }
        | CrpEvent::ToolCompleted { turn_id, .. }
        | CrpEvent::TurnCompleted { turn_id, .. } => Some(turn_id.as_str()),
        CrpEvent::SessionOpened { .. } | CrpEvent::ModelsList { .. } => None,
    }
}

pub(super) fn event_matches_session(event: &CrpEvent, session_id: &str) -> bool {
    match event {
        CrpEvent::SessionOpened { session_id: id, .. }
        | CrpEvent::TurnStarted { session_id: id, .. }
        | CrpEvent::MessageDelta { session_id: id, .. }
        | CrpEvent::MessageFinal { session_id: id, .. }
        | CrpEvent::ReasoningSummary { session_id: id, .. }
        | CrpEvent::ReasoningTrace { session_id: id, .. }
        | CrpEvent::ReasoningTraceFinal { session_id: id, .. }
        | CrpEvent::ToolStarted { session_id: id, .. }
        | CrpEvent::ToolOutputDelta { session_id: id, .. }
        | CrpEvent::ToolCompleted { session_id: id, .. }
        | CrpEvent::TurnCompleted { session_id: id, .. }
        | CrpEvent::SessionGap { session_id: id, .. }
        | CrpEvent::SessionNotice { session_id: id, .. } => id == session_id,
        CrpEvent::ModelsList { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_completed_retains_started_preview_when_completed_omits_it() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let started_preview = json!({"summary":"Read: foo.txt"});
        let started = map_crp_event(
            CrpEvent::ToolStarted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                input: None,
                input_preview: Some(started_preview.clone()),
            },
            CrpChannel::Control,
            1,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );
        assert_eq!(started.events.len(), 1);
        assert!(matches!(
            &started.events[0].event_type,
            SessionEventType::ToolCall
        ));

        let completed = map_crp_event(
            CrpEvent::ToolCompleted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                status: CrpToolStatus::Success,
                output: None,
                error: None,
                input_preview: None,
            },
            CrpChannel::Control,
            2,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );
        assert_eq!(completed.events.len(), 1);
        assert!(matches!(
            &completed.events[0].event_type,
            SessionEventType::ToolResult
        ));

        let payload = &completed.events[0].payload_json;
        assert_eq!(payload.get("input_preview"), Some(&started_preview));
        assert_eq!(payload.get("rawInput"), Some(&started_preview));
    }

    #[test]
    fn session_opened_preserves_claude_supported_command_metadata() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let mapped = map_crp_event(
            CrpEvent::SessionOpened {
                session_id: "session-1".to_string(),
                provider_session_id: Some("provider-session-1".to_string()),
                commands: Some(json!([
                    {
                        "name": "compact",
                        "description": "Summarize conversation to save context",
                        "argument_hint": "<focus>"
                    }
                ])),
                slash_commands: Some(vec!["compact".to_string(), "review".to_string()]),
                models: Some(json!([
                    {
                        "id": "sonnet",
                        "name": "Sonnet"
                    }
                ])),
                current_model_id: Some("sonnet".to_string()),
                agents: Some(json!([
                    {
                        "name": "Explore",
                        "description": "Research the repo"
                    }
                ])),
                output_style: Some("default".to_string()),
                available_output_styles: Some(vec!["default".to_string(), "brief".to_string()]),
                skills: Some(vec!["simplify".to_string()]),
                plugins: Some(json!([
                    {
                        "name": "plugin-a",
                        "path": "/tmp/plugin-a"
                    }
                ])),
                tools: Some(vec!["Read".to_string(), "Write".to_string()]),
                permission_mode: Some("default".to_string()),
                mcp_servers: Some(json!([{ "name": "github", "status": "connected" }])),
                account: Some(json!({ "email": "dev@example.com" })),
                fast_mode_state: Some("off".to_string()),
            },
            CrpChannel::Control,
            1,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );

        assert_eq!(mapped.events.len(), 1);
        assert!(matches!(
            mapped.events[0].event_type,
            SessionEventType::Init
        ));
        let payload = &mapped.events[0].payload_json;
        assert_eq!(payload.get("session_id"), Some(&json!("session-1")));
        assert_eq!(
            payload.get("provider_session_id"),
            Some(&json!("provider-session-1"))
        );
        assert_eq!(payload.pointer("/commands/0/name"), Some(&json!("compact")));
        assert_eq!(
            payload.pointer("/commands/0/description"),
            Some(&json!("Summarize conversation to save context"))
        );
        assert_eq!(
            payload.get("slash_commands"),
            Some(&json!(["compact", "review"]))
        );
        assert_eq!(payload.get("current_model_id"), Some(&json!("sonnet")));
        assert_eq!(payload.get("output_style"), Some(&json!("default")));
        assert_eq!(
            payload.get("available_output_styles"),
            Some(&json!(["default", "brief"]))
        );
        assert_eq!(payload.get("skills"), Some(&json!(["simplify"])));
        assert_eq!(payload.get("permission_mode"), Some(&json!("default")));
        assert_eq!(payload.get("fast_mode_state"), Some(&json!("off")));
    }

    #[cfg(feature = "fuzz_tests")]
    mod fuzz_tests {
        use super::*;
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        use serde_json::{json, Map, Value};
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use std::path::PathBuf;

        use crate::crp::policy::parse_crp_slash_command;
        use crate::crp::protocol::CrpEventEnvelope;

        const ITERATIONS: usize = 200;
        const MAX_DEPTH: u8 = 3;

        fn random_string(rng: &mut StdRng, max_len: usize) -> String {
            const ALPHABET: &[u8] =
                b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_- /.:";
            let len = rng.gen_range(1..=max_len.max(1));
            (0..len)
                .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                .collect()
        }

        fn random_value(rng: &mut StdRng, depth: u8) -> Value {
            if depth == 0 {
                return match rng.gen_range(0..5) {
                    0 => Value::String(random_string(rng, 16)),
                    1 => Value::Number((rng.gen_range(0..=9999_u64)).into()),
                    2 => Value::Bool(rng.gen_bool(0.5)),
                    3 => Value::Null,
                    _ => json!({"k": "v"}),
                };
            }

            match rng.gen_range(0..6) {
                0 => Value::String(random_string(rng, 24)),
                1 => Value::Array(
                    (0..rng.gen_range(0..=4))
                        .map(|_| random_value(rng, depth - 1))
                        .collect(),
                ),
                2 => {
                    let mut map = Map::new();
                    for _ in 0..rng.gen_range(0..=4) {
                        map.insert(random_string(rng, 12), random_value(rng, depth - 1));
                    }
                    Value::Object(map)
                }
                3 => Value::Bool(rng.gen_bool(0.5)),
                4 => Value::Number((rng.gen_range(0..=9999_u64)).into()),
                _ => Value::Null,
            }
        }

        fn random_event_type(rng: &mut StdRng) -> &'static str {
            const TYPES: &[&str] = &[
                "session.opened",
                "turn.started",
                "message.delta",
                "message.final",
                "reasoning.summary",
                "reasoning.trace",
                "reasoning.trace.final",
                "tool.started",
                "tool.output_delta",
                "tool.completed",
                "turn.completed",
                "session.notice",
                "session.gap",
            ];
            TYPES[rng.gen_range(0..TYPES.len())]
        }

        fn valid_message_delta(seq: u64) -> String {
            json!({
                "v": 1,
                "seq": seq,
                "channel": "control",
                "type": "message.delta",
                "session_id": "s",
                "turn_id": "t",
                "message_id": "m",
                "delta": "hello",
            })
            .to_string()
        }

        fn random_envelope_json(rng: &mut StdRng) -> String {
            if rng.gen_bool(0.15) {
                return random_value(rng, MAX_DEPTH).to_string();
            }

            let mut obj = Map::new();
            obj.insert(
                "seq".to_string(),
                Value::Number((rng.gen_range(0..=10_000_u64)).into()),
            );
            obj.insert(
                "channel".to_string(),
                Value::String(if rng.gen_bool(0.5) {
                    "control".to_string()
                } else {
                    "data".to_string()
                }),
            );
            obj.insert(
                "type".to_string(),
                Value::String(random_event_type(rng).to_string()),
            );
            obj.insert(
                "session_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert("turn_id".to_string(), Value::String(random_string(rng, 8)));
            obj.insert(
                "message_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert("delta".to_string(), Value::String(random_string(rng, 12)));
            obj.insert("content".to_string(), Value::String(random_string(rng, 16)));
            obj.insert(
                "summary_index".to_string(),
                Value::Number((rng.gen_range(0..=8_u64)).into()),
            );
            obj.insert("text".to_string(), Value::String(random_string(rng, 18)));
            obj.insert("chunk".to_string(), Value::String(random_string(rng, 18)));
            obj.insert(
                "tool_call_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert(
                "tool_name".to_string(),
                Value::String(random_string(rng, 10)),
            );
            obj.insert(
                "status".to_string(),
                Value::String(if rng.gen_bool(0.5) {
                    "success".to_string()
                } else {
                    "error".to_string()
                }),
            );
            obj.insert("reason".to_string(), Value::String(random_string(rng, 12)));
            if rng.gen_bool(0.5) {
                obj.insert("details".to_string(), random_value(rng, MAX_DEPTH - 1));
            }
            Value::Object(obj).to_string()
        }

        fn try_parse_and_map(line: &str) {
            let parsed = catch_unwind(AssertUnwindSafe(|| {
                serde_json::from_str::<CrpEventEnvelope>(line)
            }));
            assert!(parsed.is_ok(), "panic while parsing CRP envelope");
            if let Ok(env) = parsed.expect("parse panic already checked") {
                let mut tool_output_cache: HashMap<String, String> = HashMap::new();
                let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
                let mapped = catch_unwind(AssertUnwindSafe(|| {
                    map_crp_event(
                        env.event,
                        env.channel,
                        env.seq,
                        &mut tool_output_cache,
                        &mut tool_input_cache,
                    )
                }));
                assert!(mapped.is_ok(), "panic while mapping CRP event");
            }
        }

        #[test]
        fn fuzz_crp_envelope_parsing_and_mapping_do_not_panic() {
            let mut rng = StdRng::seed_from_u64(0xAC1F_2026);

            for idx in 0..ITERATIONS {
                let line = if idx % 10 == 0 {
                    valid_message_delta((idx as u64) + 1)
                } else {
                    random_envelope_json(&mut rng)
                };
                try_parse_and_map(&line);

                let random_slash = if rng.gen_bool(0.5) {
                    format!(
                        "/{} {}",
                        random_string(&mut rng, 10),
                        random_string(&mut rng, 16)
                    )
                } else {
                    random_string(&mut rng, 24)
                };
                let slash =
                    catch_unwind(AssertUnwindSafe(|| parse_crp_slash_command(&random_slash)));
                assert!(slash.is_ok(), "panic while parsing slash command");
            }
        }

        #[test]
        fn fuzz_acp_corpus_replay_does_not_panic() {
            let corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/acp");
            assert!(
                corpus_dir.is_dir(),
                "missing corpus dir: {}",
                corpus_dir.display()
            );

            let mut files = std::fs::read_dir(&corpus_dir)
                .expect("read_dir failed")
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.is_file())
                .collect::<Vec<_>>();
            files.sort();
            assert!(!files.is_empty(), "expected at least one corpus file");

            for file in files {
                let body = std::fs::read_to_string(&file).expect("read corpus file");
                for raw in body.lines() {
                    let line = raw.trim();
                    if line.is_empty() {
                        continue;
                    }
                    try_parse_and_map(line);
                }
            }
        }
    }
}
