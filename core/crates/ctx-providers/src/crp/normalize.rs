use std::collections::HashMap;

use serde_json::{json, Value};

use ctx_core::models::SessionEventType;

use crate::events::NormalizedEvent;

use super::normalize_tool_payload::{build_tool_completed_payload, build_tool_started_payload};
use super::protocol::{CrpChannel, CrpEvent, CrpTurnStatus, KnownCrpEvent};
use super::unknown_event::{
    bound_unknown_crp_payload, extract_unknown_crp_tool_name, extract_unknown_crp_tool_preview,
    summarize_unknown_crp_event,
};

pub(super) struct MappedCrpEvent {
    pub(super) events: Vec<NormalizedEvent>,
    pub(super) done: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CachedToolInput {
    pub(super) input: Option<Value>,
    pub(super) input_preview: Option<Value>,
}

fn is_auth_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_required"
            | "auth_error"
            | "auth_failed"
            | "auth_complete"
            | "auth_completed"
            | "auth_success"
            | "authenticated"
    )
}

fn safe_auth_notice_message(code: &str, message: Option<&str>) -> Option<String> {
    let generic = match code {
        "auth_required" => "Authentication required.",
        "auth_error" | "auth_failed" => "Authentication failed.",
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated" => {
            "Authentication complete."
        }
        _ => "Authentication update.",
    };

    let Some(message) = message.map(str::trim).filter(|value| !value.is_empty()) else {
        return Some(generic.to_string());
    };

    let lower = message.to_ascii_lowercase();
    if lower.contains("://")
        || lower.contains("www.")
        || lower.contains("token=")
        || lower.contains("code=")
        || lower.contains("bearer ")
    {
        return Some(generic.to_string());
    }

    Some(message.to_string())
}

fn safe_auth_methods_value(value: &Value) -> Option<Value> {
    let methods = value
        .as_array()?
        .iter()
        .filter_map(|method| {
            let object = method.as_object()?;
            let id = object
                .get("id")
                .or_else(|| object.get("methodId"))
                .or_else(|| object.get("method_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let name = object
                .get("name")
                .or_else(|| object.get("label"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(id);
            Some(json!({
                "id": id,
                "name": name,
            }))
        })
        .collect::<Vec<_>>();

    (!methods.is_empty()).then_some(Value::Array(methods))
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
        CrpEvent::Known(event) => match *event {
            KnownCrpEvent::SessionOpened {
                session_id,
                provider_session_id,
                supports_session_status,
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
                if let Some(supports_session_status) = supports_session_status {
                    payload.insert(
                        "supports_session_status".to_string(),
                        json!(supports_session_status),
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
                    payload.insert("account".to_string(), *account);
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
            KnownCrpEvent::TurnStarted { .. } => MappedCrpEvent {
                events: Vec::new(),
                done: false,
            },
            KnownCrpEvent::MessageDelta {
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
            KnownCrpEvent::MessageFinal {
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
            KnownCrpEvent::ReasoningSummary {
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
            KnownCrpEvent::ReasoningTrace {
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
            KnownCrpEvent::ReasoningTraceFinal {
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
            KnownCrpEvent::TurnContextWindowUpdated { context_window, .. } => MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ContextWindowUpdate,
                    payload_json: json!({
                        "context_window": context_window,
                        "crp_seq": seq,
                    }),
                }],
                done: false,
            },
            KnownCrpEvent::ToolStarted {
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
            KnownCrpEvent::ToolOutputDelta {
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
            KnownCrpEvent::ToolCompleted {
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
            KnownCrpEvent::TurnCompleted {
                status,
                context_window,
                error,
                ..
            } => {
                let (event_type, payload) = match status {
                    CrpTurnStatus::Success => {
                        let mut payload = serde_json::Map::new();
                        payload.insert("status".to_string(), json!("completed"));
                        payload.insert("crp_seq".to_string(), json!(seq));
                        if let Some(context_window) = context_window {
                            payload.insert("context_window".to_string(), context_window);
                        }
                        (SessionEventType::Done, Value::Object(payload))
                    }
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
            KnownCrpEvent::ModelsList { .. } => MappedCrpEvent {
                events: Vec::new(),
                done: false,
            },
            KnownCrpEvent::SessionNotice {
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
                if is_auth_notice_code(&code) {
                    if let Some(message) = safe_auth_notice_message(&code, message.as_deref()) {
                        payload.insert("message".to_string(), json!(message));
                    }
                    if let Some(Value::Object(map)) = details.as_ref() {
                        if let Some(auth_methods) = map
                            .get("auth_methods")
                            .or_else(|| map.get("authMethods"))
                            .and_then(safe_auth_methods_value)
                        {
                            payload.insert("auth_methods".to_string(), auth_methods);
                        }
                        if let Some(provider) = map
                            .get("provider")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            payload.insert("provider".to_string(), json!(provider));
                        }
                    }
                } else {
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
            KnownCrpEvent::SessionGap { reason, .. } => MappedCrpEvent {
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
        },
        CrpEvent::Unknown {
            event_type,
            parse_error,
            raw,
            ..
        } => {
            let summary = summarize_unknown_crp_event(&raw)
                .unwrap_or_else(|| format!("Unknown runtime event: {event_type}"));
            let tool_name = extract_unknown_crp_tool_name(&raw);
            let tool_preview = extract_unknown_crp_tool_preview(&raw);
            let mut payload = serde_json::Map::new();
            payload.insert("kind".to_string(), json!("crp_unknown_event"));
            payload.insert("original_type".to_string(), json!(event_type));
            payload.insert("message".to_string(), json!(summary));
            payload.insert("parse_error".to_string(), json!(parse_error));
            payload.insert("display_in_timeline".to_string(), json!(true));
            if let Some(tool_name) = tool_name {
                payload.insert("tool_name".to_string(), json!(tool_name));
            }
            if let Some(tool_preview) = tool_preview {
                payload.insert("tool_preview".to_string(), json!(tool_preview));
            }
            let (bounded_raw, raw_truncated) = bound_unknown_crp_payload(raw);
            payload.insert("raw".to_string(), bounded_raw);
            if raw_truncated {
                payload.insert("raw_truncated".to_string(), json!(true));
            }
            payload.insert("crp_seq".to_string(), json!(seq));
            if let Some(channel) = crp_channel {
                payload.insert("crp_channel".to_string(), json!(channel));
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::Notice,
                    payload_json: Value::Object(payload),
                }],
                done: false,
            }
        }
    }
}

pub(super) fn event_turn_id(event: &CrpEvent) -> Option<&str> {
    match event {
        CrpEvent::Known(event) => known_event_turn_id(event.as_ref()),
        CrpEvent::Unknown { turn_id, .. } => turn_id.as_deref(),
    }
}

pub(super) fn event_matches_session(event: &CrpEvent, session_id: &str) -> bool {
    match event {
        CrpEvent::Known(event) => known_event_session_id(event.as_ref()) == Some(session_id),
        CrpEvent::Unknown { session_id: id, .. } => id.as_deref() == Some(session_id),
    }
}

fn known_event_turn_id(event: &KnownCrpEvent) -> Option<&str> {
    match event {
        KnownCrpEvent::SessionGap { turn_id, .. }
        | KnownCrpEvent::SessionNotice { turn_id, .. } => turn_id.as_deref(),
        KnownCrpEvent::TurnStarted { turn_id, .. }
        | KnownCrpEvent::MessageDelta { turn_id, .. }
        | KnownCrpEvent::MessageFinal { turn_id, .. }
        | KnownCrpEvent::ReasoningSummary { turn_id, .. }
        | KnownCrpEvent::ReasoningTrace { turn_id, .. }
        | KnownCrpEvent::ReasoningTraceFinal { turn_id, .. }
        | KnownCrpEvent::TurnContextWindowUpdated { turn_id, .. }
        | KnownCrpEvent::ToolStarted { turn_id, .. }
        | KnownCrpEvent::ToolOutputDelta { turn_id, .. }
        | KnownCrpEvent::ToolCompleted { turn_id, .. }
        | KnownCrpEvent::TurnCompleted { turn_id, .. } => Some(turn_id.as_str()),
        KnownCrpEvent::SessionOpened { .. } | KnownCrpEvent::ModelsList { .. } => None,
    }
}

fn known_event_session_id(event: &KnownCrpEvent) -> Option<&str> {
    match event {
        KnownCrpEvent::SessionOpened { session_id, .. }
        | KnownCrpEvent::TurnStarted { session_id, .. }
        | KnownCrpEvent::MessageDelta { session_id, .. }
        | KnownCrpEvent::MessageFinal { session_id, .. }
        | KnownCrpEvent::ReasoningSummary { session_id, .. }
        | KnownCrpEvent::ReasoningTrace { session_id, .. }
        | KnownCrpEvent::ReasoningTraceFinal { session_id, .. }
        | KnownCrpEvent::TurnContextWindowUpdated { session_id, .. }
        | KnownCrpEvent::ToolStarted { session_id, .. }
        | KnownCrpEvent::ToolOutputDelta { session_id, .. }
        | KnownCrpEvent::ToolCompleted { session_id, .. }
        | KnownCrpEvent::TurnCompleted { session_id, .. }
        | KnownCrpEvent::SessionGap { session_id, .. }
        | KnownCrpEvent::SessionNotice { session_id, .. } => Some(session_id.as_str()),
        KnownCrpEvent::ModelsList { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crp::protocol::CrpToolStatus;

    fn known(event: KnownCrpEvent) -> CrpEvent {
        CrpEvent::Known(Box::new(event))
    }

    #[test]
    fn tool_completed_retains_started_preview_when_completed_omits_it() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let started_preview = json!({"summary":"Read: foo.txt"});
        let started = map_crp_event(
            known(KnownCrpEvent::ToolStarted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                input: None,
                input_preview: Some(started_preview.clone()),
            }),
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
            known(KnownCrpEvent::ToolCompleted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                status: CrpToolStatus::Success,
                output: None,
                error: None,
                input_preview: None,
            }),
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
            known(KnownCrpEvent::SessionOpened {
                session_id: "session-1".to_string(),
                provider_session_id: Some("provider-session-1".to_string()),
                supports_session_status: Some(true),
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
                account: Some(Box::new(json!({ "email": "dev@example.com" }))),
                fast_mode_state: Some("off".to_string()),
            }),
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
        assert_eq!(payload.get("supports_session_status"), Some(&json!(true)));
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

    #[test]
    fn auth_session_notice_persists_only_safe_subset() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let mapped = map_crp_event(
            known(KnownCrpEvent::SessionNotice {
                session_id: "session-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                code: "auth_required".to_string(),
                severity: Some("warning".to_string()),
                message: Some("Visit https://auth.example.test/start?token=secret".to_string()),
                details: Some(json!({
                    "provider": "amp",
                    "auth_methods": [
                        {
                            "id": "oauth",
                            "name": "Sign in",
                            "description": "Opens a browser",
                            "_meta": {
                                "authUrl": "https://auth.example.test/start?token=secret"
                            }
                        }
                    ],
                    "auth_url": "https://auth.example.test/start?token=secret",
                    "message": "raw provider payload"
                })),
                transient: Some(false),
            }),
            CrpChannel::Control,
            7,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );

        assert_eq!(mapped.events.len(), 1);
        assert!(matches!(
            mapped.events[0].event_type,
            SessionEventType::Notice
        ));
        let payload = mapped.events[0]
            .payload_json
            .as_object()
            .expect("notice payload");
        assert_eq!(payload.get("kind"), Some(&json!("auth_required")));
        assert_eq!(payload.get("code"), Some(&json!("auth_required")));
        assert_eq!(payload.get("severity"), Some(&json!("warning")));
        assert_eq!(
            payload.get("message"),
            Some(&json!("Authentication required."))
        );
        assert_eq!(payload.get("provider"), Some(&json!("amp")));
        assert_eq!(
            payload.get("auth_methods"),
            Some(&json!([{ "id": "oauth", "name": "Sign in" }]))
        );
        assert_eq!(payload.get("transient"), Some(&json!(false)));
        assert_eq!(payload.get("crp_seq"), Some(&json!(7)));
        assert!(!payload.contains_key("details"));
        let serialized = Value::Object(payload.clone()).to_string();
        assert!(!serialized.contains("https://auth.example.test/start"));
        assert!(!serialized.contains("token=secret"));
        assert!(!serialized.contains("raw provider payload"));
    }

    #[test]
    fn non_auth_session_notice_keeps_details() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let mapped = map_crp_event(
            known(KnownCrpEvent::SessionNotice {
                session_id: "session-1".to_string(),
                turn_id: None,
                code: "provider_guard_warning".to_string(),
                severity: Some("warning".to_string()),
                message: Some("Provider memory high".to_string()),
                details: Some(json!({
                    "provider": "amp",
                    "memory_mb": 1024
                })),
                transient: Some(false),
            }),
            CrpChannel::Control,
            8,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );

        let payload = mapped.events[0]
            .payload_json
            .as_object()
            .expect("notice payload");
        assert_eq!(payload.get("message"), Some(&json!("Provider memory high")));
        assert_eq!(
            payload.get("details"),
            Some(&json!({
                "provider": "amp",
                "memory_mb": 1024
            }))
        );
    }

    #[test]
    fn context_window_update_maps_to_partial_session_event() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let metrics = json!({
            "context_tokens_estimate": 4200,
            "context_window_tokens": 128000,
            "remaining_tokens_estimate": 123800,
            "remaining_fraction": 0.9671875,
        });
        let mapped = map_crp_event(
            known(KnownCrpEvent::TurnContextWindowUpdated {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                context_window: metrics.clone(),
            }),
            CrpChannel::Control,
            9,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );

        assert_eq!(mapped.events.len(), 1);
        assert!(!mapped.done);
        assert!(matches!(
            mapped.events[0].event_type,
            SessionEventType::ContextWindowUpdate
        ));
        assert_eq!(
            mapped.events[0].payload_json,
            json!({
                "context_window": metrics,
                "crp_seq": 9,
            })
        );
    }

    #[cfg(feature = "fuzz_tests")]
    mod fuzz_tests;
}
