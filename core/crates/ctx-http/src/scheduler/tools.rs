use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::fs;

use ctx_core::models::{SessionEventType, SessionTurnTool};

use crate::daemon::AppState;

mod preview;
use preview::*;

fn string_from_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn normalize_placeholder_tool_label(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '_' | '-'))
        .collect()
}

fn is_placeholder_tool_label(value: Option<&str>) -> bool {
    let normalized = normalize_placeholder_tool_label(value.unwrap_or_default());
    normalized.is_empty()
        || normalized == "unknown"
        || normalized == "tool"
        || normalized == "unknowntool"
}

#[derive(Clone, Copy)]
struct InferredToolIdentity {
    kind: &'static str,
    name: &'static str,
}

fn non_placeholder_tool_field(raw: Option<String>) -> Option<String> {
    raw.filter(|value| !is_placeholder_tool_label(Some(value.as_str())))
}

fn direct_input_preview(update: &Value) -> Option<Value> {
    update.get("input_preview").and_then(|value| {
        if value.is_null() {
            None
        } else {
            Some(value.clone())
        }
    })
}

fn inferred_tool_identity(
    tool_call_id: &str,
    input_preview: Option<&Value>,
    output_preview: Option<&str>,
) -> Option<InferredToolIdentity> {
    let input = input_preview.and_then(Value::as_object);
    if let Some(input) = input {
        if input.contains_key("command") || input.contains_key("parsed_cmd") {
            return Some(InferredToolIdentity {
                kind: "execute",
                name: "Bash",
            });
        }
        if input.contains_key("query")
            || input.contains_key("pattern")
            || input.contains_key("regex")
        {
            return Some(InferredToolIdentity {
                kind: "search",
                name: "Grep",
            });
        }
        if input.contains_key("file_path")
            || input.contains_key("filePath")
            || input.contains_key("filepath")
            || input.contains_key("filename")
            || input.contains_key("file")
            || input.contains_key("path")
        {
            let is_edit = input.contains_key("changes")
                || input.contains_key("edits")
                || input.contains_key("new_string")
                || input.contains_key("newText")
                || input.contains_key("replacement")
                || input.contains_key("diff_stats");
            return Some(if is_edit {
                InferredToolIdentity {
                    kind: "edit",
                    name: "Edit",
                }
            } else {
                InferredToolIdentity {
                    kind: "read",
                    name: "Read",
                }
            });
        }
    }

    let output = output_preview
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let read_like = output.lines().take(3).any(|line| {
        let trimmed = line.trim_start();
        let mut chars = trimmed.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_ascii_digit() && chars.as_str().starts_with('→')
    });
    if read_like {
        return Some(InferredToolIdentity {
            kind: "read",
            name: "Read",
        });
    }

    if tool_call_id.starts_with("toolu_") {
        return Some(InferredToolIdentity {
            kind: "execute",
            name: "Bash",
        });
    }

    None
}

fn tool_label_from_update(update: &Value) -> Option<String> {
    string_from_value(update.get("tool_label"))
        .or_else(|| string_from_value(update.get("toolLabel")))
        .or_else(|| string_from_value(update.pointer("/toolCall/tool_label")))
        .or_else(|| string_from_value(update.pointer("/toolCall/toolLabel")))
}

fn tool_name_from_update(update: &Value) -> Option<String> {
    string_from_value(update.get("tool_name"))
        .or_else(|| string_from_value(update.get("toolName")))
        .or_else(|| string_from_value(update.get("name")))
        .or_else(|| string_from_value(update.pointer("/toolCall/name")))
}

fn tool_kind_from_update(update: &Value) -> Option<String> {
    string_from_value(update.get("kind"))
        .or_else(|| string_from_value(update.pointer("/toolCall/kind")))
}

fn tool_title_from_update(update: &Value) -> Option<String> {
    string_from_value(update.get("title"))
        .or_else(|| string_from_value(update.get("tool_label")))
        .or_else(|| string_from_value(update.get("toolLabel")))
        .or_else(|| string_from_value(update.pointer("/toolCall/title")))
        .or_else(|| string_from_value(update.pointer("/toolCall/tool_label")))
        .or_else(|| string_from_value(update.pointer("/toolCall/toolLabel")))
        .or_else(|| tool_name_from_update(update))
}

pub(super) fn sanitize_tool_event_payload(
    event_type: &SessionEventType,
    raw_payload: &Value,
    output_spool_path: Option<&str>,
) -> Value {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload).unwrap_or_default();

    let raw_tool_kind = tool_kind_from_update(update);
    let raw_provider_tool_name = tool_name_from_update(update);
    let tool_label = tool_label_from_update(update);
    let raw_title = tool_title_from_update(update);

    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw) = raw_status {
        normalize_tool_status(raw, event_type.clone())
    } else if matches!(event_type, SessionEventType::ToolResult) {
        "completed".to_string()
    } else {
        "pending".to_string()
    };

    let input = extract_tool_input(update);
    let input_preview = direct_input_preview(update).or_else(|| {
        tool_input_preview(
            input,
            update,
            raw_tool_kind.as_deref(),
            raw_title.as_deref(),
        )
    });
    let input_meta = build_json_preview(input, input_preview);

    let patch_preview = if is_edit_tool(raw_tool_kind.as_deref(), raw_title.as_deref()) {
        extract_patch_text_owned(input, update).map(|t| build_output_preview(&t))
    } else {
        None
    };

    let output_preview = extract_tool_output_text(update)
        .map(|t| build_output_preview(&t))
        .or(patch_preview)
        .filter(|preview| !preview.preview.trim().is_empty());

    let inferred = inferred_tool_identity(
        &tool_call_id,
        input_meta.preview.as_ref(),
        output_preview
            .as_ref()
            .map(|preview| preview.preview.as_str()),
    );
    let tool_kind = non_placeholder_tool_field(raw_tool_kind)
        .or_else(|| inferred.map(|identity| identity.kind.to_string()));
    let provider_tool_name = non_placeholder_tool_field(raw_provider_tool_name)
        .or_else(|| inferred.map(|identity| identity.name.to_string()));
    let tool_name = provider_tool_name.clone();
    let title = non_placeholder_tool_field(raw_title)
        .or_else(|| tool_label.clone())
        .or_else(|| inferred.map(|identity| identity.name.to_string()));
    let subtitle = tool_subtitle_from_preview(
        tool_kind.as_deref(),
        provider_tool_name.as_deref(),
        input_meta.preview.as_ref(),
    );

    let mut obj = serde_json::Map::new();
    if !tool_call_id.trim().is_empty() {
        obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
    }
    if let Some(v) = tool_kind {
        obj.insert("kind".to_string(), Value::String(v));
    }
    if let Some(v) = tool_label {
        obj.insert("tool_label".to_string(), Value::String(v));
    }
    if let Some(v) = tool_name {
        obj.insert("tool_name".to_string(), Value::String(v));
    }
    if let Some(v) = title {
        obj.insert("title".to_string(), Value::String(v));
    }
    if let Some(v) = subtitle {
        obj.insert("subtitle".to_string(), Value::String(v));
    }
    obj.insert("status".to_string(), Value::String(status));
    if let Some(v) = input_meta.preview {
        obj.insert("input_preview".to_string(), v);
    }
    if let Some(truncated) = input_meta.truncated {
        obj.insert("input_truncated".to_string(), Value::Bool(truncated));
    }
    if let Some(bytes) = input_meta.original_bytes {
        obj.insert(
            "input_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(bytes)),
        );
    }
    if let Some(preview) = output_preview {
        obj.insert("output_preview".to_string(), Value::String(preview.preview));
        obj.insert(
            "output_truncated".to_string(),
            Value::Bool(preview.truncated),
        );
        obj.insert(
            "output_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(preview.original_bytes as i64)),
        );
    }
    if let Some(value) = raw_payload.get("crp_seq").or_else(|| update.get("crp_seq")) {
        obj.insert("crp_seq".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("crp_channel")
        .or_else(|| update.get("crp_channel"))
    {
        obj.insert("crp_channel".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("order_seq")
        .or_else(|| raw_payload.get("orderSeq"))
        .or_else(|| update.get("order_seq"))
        .or_else(|| update.get("orderSeq"))
    {
        obj.insert("order_seq".to_string(), value.clone());
    }
    if let Some(path) = output_spool_path {
        obj.insert(
            "output_spool_path".to_string(),
            Value::String(path.to_string()),
        );
    }
    Value::Object(obj)
}

#[derive(Debug, Clone)]
pub(super) struct ToolOpsMeta {
    pub(super) tool_call_id: Option<String>,
    pub(super) tool_kind: Option<String>,
    pub(super) title: Option<String>,
    pub(super) status: Option<String>,
    pub(super) input_preview: Option<Value>,
    pub(super) cwd: Option<String>,
}

pub(super) fn build_tool_ops_meta(
    event_type: &SessionEventType,
    raw_payload: &Value,
) -> ToolOpsMeta {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload);
    let tool_kind = tool_kind_from_update(update);
    let title = tool_title_from_update(update);
    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw) = raw_status {
        Some(normalize_tool_status(raw, event_type.clone()))
    } else if matches!(event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else {
        Some("pending".to_string())
    };
    let input = extract_tool_input(update);
    let input_preview = direct_input_preview(update)
        .or_else(|| tool_input_preview(input, update, tool_kind.as_deref(), title.as_deref()));
    let cwd = input_preview
        .as_ref()
        .and_then(|preview| preview.get("cwd"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    ToolOpsMeta {
        tool_call_id,
        tool_kind,
        title,
        status,
        input_preview,
        cwd,
    }
}

pub(super) fn cwd_outside_worktree(
    cwd: &str,
    workdir_root: &Path,
    workdir_canonical: Option<&PathBuf>,
) -> bool {
    if cwd.trim().is_empty() {
        return false;
    }
    let cwd_path = Path::new(cwd);
    if cwd_path.is_relative() {
        return false;
    }
    if cwd_path.starts_with(workdir_root) {
        return false;
    }
    if let Some(root) = workdir_canonical {
        if cwd_path.starts_with(root) {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug)]
pub(super) struct TurnToolUpdate {
    pub(super) tool_call_id: String,
    order_seq: Option<i64>,
    tool_kind: Option<String>,
    provider_tool_name: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    status: Option<String>,
    input_json: Option<Value>,
    output_text: Option<String>,
    input_truncated: Option<bool>,
    input_original_bytes: Option<i64>,
    output_truncated: Option<bool>,
    output_original_bytes: Option<i64>,
}

pub(super) fn build_turn_tool_update_from_payload(
    event_type: &SessionEventType,
    payload_json: &Value,
) -> Option<TurnToolUpdate> {
    let update = extract_tool_update(payload_json);
    let tool_call_id = tool_call_id_from_payload(payload_json)?;
    let order_seq = payload_json
        .get("order_seq")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            payload_json
                .get("orderSeq")
                .and_then(|value| value.as_i64())
        });
    let raw_tool_kind = tool_kind_from_update(update);
    let raw_provider_tool_name = tool_name_from_update(update);
    let raw_title = tool_title_from_update(update);
    let raw_status = update
        .get("status")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/status").and_then(|v| v.as_str()));
    let status = if let Some(raw) = raw_status {
        Some(normalize_tool_status(raw, event_type.clone()))
    } else if matches!(event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else if matches!(event_type, SessionEventType::ToolCall) {
        Some("pending".to_string())
    } else {
        None
    };

    let input = extract_tool_input(update);
    let input_json = direct_input_preview(update).or_else(|| {
        tool_input_preview(
            input,
            update,
            raw_tool_kind.as_deref(),
            raw_title.as_deref(),
        )
    });
    let input_meta = build_json_preview(input, input_json);
    let output_preview = extract_tool_output_text(update)
        .map(|t| build_output_preview(&t))
        .filter(|preview| !preview.preview.trim().is_empty());
    let inferred = inferred_tool_identity(
        &tool_call_id,
        input_meta.preview.as_ref(),
        output_preview
            .as_ref()
            .map(|preview| preview.preview.as_str()),
    );
    let tool_kind = non_placeholder_tool_field(raw_tool_kind)
        .or_else(|| inferred.map(|identity| identity.kind.to_string()));
    let provider_tool_name = non_placeholder_tool_field(raw_provider_tool_name)
        .or_else(|| inferred.map(|identity| identity.name.to_string()));
    let title = non_placeholder_tool_field(raw_title)
        .or_else(|| inferred.map(|identity| identity.name.to_string()));
    let subtitle = tool_subtitle_from_preview(
        tool_kind.as_deref(),
        provider_tool_name.as_deref(),
        input_meta.preview.as_ref(),
    );

    let patch_preview = if is_edit_tool(tool_kind.as_deref(), title.as_deref()) {
        extract_patch_text_owned(input, update).map(|t| build_output_preview(&t))
    } else {
        None
    };

    let output_preview = extract_tool_output_text(update)
        .map(|t| build_output_preview(&t))
        .or(patch_preview)
        .filter(|preview| !preview.preview.trim().is_empty());

    Some(TurnToolUpdate {
        tool_call_id,
        order_seq,
        tool_kind,
        provider_tool_name,
        title,
        subtitle,
        status,
        input_json: input_meta.preview,
        output_text: output_preview
            .as_ref()
            .map(|preview| preview.preview.clone()),
        input_truncated: input_meta.truncated,
        input_original_bytes: input_meta.original_bytes,
        output_truncated: output_preview.as_ref().map(|preview| preview.truncated),
        output_original_bytes: output_preview
            .as_ref()
            .map(|preview| preview.original_bytes as i64),
    })
}

pub(super) fn merge_tool_update(
    prev: Option<&SessionTurnTool>,
    update: TurnToolUpdate,
    session_id: ctx_core::ids::SessionId,
    turn_id: ctx_core::ids::TurnId,
    event_seq: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<SessionTurnTool> {
    let created_at = prev.map(|t| t.created_at).unwrap_or(now);
    let order_seq = match (prev.map(|t| t.order_seq), update.order_seq) {
        (Some(prev_seq), Some(event_seq)) => prev_seq.min(event_seq),
        (Some(prev_seq), None) => prev_seq,
        (None, Some(event_seq)) => event_seq,
        (None, None) => {
            tracing::error!(
                tool_call_id = %update.tool_call_id,
                session_id = %session_id.0,
                turn_id = %turn_id.0,
                "tool update missing canonical order_seq"
            );
            return None;
        }
    };
    let first_event_seq = match prev.and_then(|t| t.first_event_seq) {
        Some(prev_seq) => Some(prev_seq.min(event_seq)),
        None => Some(event_seq),
    };
    let tool_kind = update
        .tool_kind
        .or_else(|| prev.and_then(|t| t.tool_kind.clone()));
    let provider_tool_name = update
        .provider_tool_name
        .or_else(|| prev.and_then(|t| t.provider_tool_name.clone()));
    let title = update.title.or_else(|| prev.and_then(|t| t.title.clone()));
    let subtitle = update
        .subtitle
        .or_else(|| prev.and_then(|t| t.subtitle.clone()));
    let status = update
        .status
        .or_else(|| prev.and_then(|t| t.status.clone()));
    let input_json = update
        .input_json
        .or_else(|| prev.and_then(|t| t.input_json.clone()));
    let input_truncated = update
        .input_truncated
        .or_else(|| prev.and_then(|t| t.input_truncated));
    let input_original_bytes = update
        .input_original_bytes
        .or_else(|| prev.and_then(|t| t.input_original_bytes));
    let output_text = match update.output_text {
        Some(next) => Some(merge_streaming_text(
            prev.and_then(|t| t.output_text.as_deref()),
            &next,
        )),
        None => prev.and_then(|t| t.output_text.clone()),
    };
    let output_truncated = match (
        update.output_truncated,
        prev.and_then(|t| t.output_truncated),
    ) {
        (Some(next), Some(prev)) => Some(prev || next),
        (Some(next), None) => Some(next),
        (None, Some(prev)) => Some(prev),
        (None, None) => None,
    };
    let output_original_bytes = match (
        update.output_original_bytes,
        prev.and_then(|t| t.output_original_bytes),
    ) {
        (Some(next), Some(prev)) => Some(prev.max(next)),
        (Some(next), None) => Some(next),
        (None, Some(prev)) => Some(prev),
        (None, None) => None,
    };
    Some(SessionTurnTool {
        session_id,
        tool_call_id: update.tool_call_id,
        turn_id,
        tool_kind,
        provider_tool_name,
        title,
        subtitle,
        status,
        input_json,
        output_text,
        order_seq,
        first_event_seq,
        input_truncated,
        input_original_bytes,
        output_truncated,
        output_original_bytes,
        created_at,
        updated_at: now,
    })
}

pub(super) fn tool_count_deltas(
    prev: Option<&SessionTurnTool>,
    next: &SessionTurnTool,
) -> (i64, i64, i64, i64, i64) {
    let prev_bucket = tool_status_bucket(prev.and_then(|t| t.status.as_deref()));
    let next_bucket = tool_status_bucket(next.status.as_deref());

    let mut delta_total = 0;
    let mut delta_pending = 0;
    let mut delta_running = 0;
    let mut delta_completed = 0;
    let mut delta_failed = 0;

    if prev.is_none() {
        delta_total += 1;
    }
    if prev_bucket != next_bucket {
        if let Some(bucket) = prev_bucket {
            match bucket {
                "pending" => delta_pending -= 1,
                "in_progress" => delta_running -= 1,
                "completed" => delta_completed -= 1,
                "failed" => delta_failed -= 1,
                _ => {}
            }
        }
        if let Some(bucket) = next_bucket {
            match bucket {
                "pending" => delta_pending += 1,
                "in_progress" => delta_running += 1,
                "completed" => delta_completed += 1,
                "failed" => delta_failed += 1,
                _ => {}
            }
        }
    }

    (
        delta_total,
        delta_pending,
        delta_running,
        delta_completed,
        delta_failed,
    )
}

fn tool_status_bucket(status: Option<&str>) -> Option<&'static str> {
    let s = status.unwrap_or("").to_lowercase();
    match s.as_str() {
        "pending" | "queued" => Some("pending"),
        "in_progress" | "inprogress" | "running" => Some("in_progress"),
        "completed" | "complete" | "ok" | "succeeded" => Some("completed"),
        "failed" | "error" => Some("failed"),
        "" => Some("pending"),
        _ => Some("pending"),
    }
}

fn normalize_tool_status(status: &str, event_type: SessionEventType) -> String {
    let s = status.trim().to_lowercase();
    if s == "inprogress" || s == "in_progress" || s == "running" {
        return "in_progress".to_string();
    }
    if s == "pending" || s == "queued" {
        return "pending".to_string();
    }
    if s == "completed" || s == "complete" || s == "ok" || s == "succeeded" {
        return "completed".to_string();
    }
    if s == "failed" || s == "error" {
        return "failed".to_string();
    }
    if matches!(event_type, SessionEventType::ToolResult) {
        return "completed".to_string();
    }
    if s.is_empty() {
        return "pending".to_string();
    }
    s
}

fn extract_tool_update(payload: &Value) -> &Value {
    payload
}

fn tool_call_id_from_payload(payload: &Value) -> Option<String> {
    if let Some(v) = payload.get("tool_call_id").and_then(|v| v.as_str()) {
        return Some(v.to_string());
    }
    let update = extract_tool_update(payload);
    let direct = update
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("tool_call_id").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let from_raw = update
        .pointer("/rawInput/call_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            update
                .pointer("/raw_input/call_id")
                .and_then(|v| v.as_str())
        });
    from_raw.map(|v| v.to_string())
}

fn sanitize_spool_segment(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push_str("tool_output");
    }
    if out.len() > 80 {
        out.truncate(80);
    }
    out
}

fn extract_tool_output_raw_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/toolCall/outputText")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            update
                .pointer("/toolCall/output_text")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/rawOutput/aggregated_output")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.pointer("/rawOutput/output").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        if !v.trim().is_empty() {
            return Some(v.to_string());
        }
    }

    let blocks = update.get("content").and_then(|v| v.as_array())?;
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
        {
            out.push_str(t);
        } else if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| update.get("output_preview").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/toolCall/outputText")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            update
                .pointer("/toolCall/output_text")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/rawOutput/aggregated_output")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.pointer("/rawOutput/output").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let blocks = update.get("content").and_then(|v| v.as_array())?;
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
        {
            out.push_str(t);
        } else if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

pub(super) async fn maybe_spool_tool_output(
    state: &AppState,
    raw_payload: &Value,
    session_id: ctx_core::ids::SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> Option<String> {
    if !state.core.tool_output_spool_enabled {
        return None;
    }
    let tool_call_id = tool_call_id_from_payload(raw_payload)?;
    let update = extract_tool_update(raw_payload);
    let output = extract_tool_output_raw_text(update)?;
    if output.trim().is_empty() {
        return None;
    }
    let mut dir = state
        .core
        .tool_output_spool_dir
        .join(session_id.0.to_string());
    dir = dir.join(turn_id.0.to_string());
    if let Err(err) = fs::create_dir_all(&dir).await {
        tracing::warn!(
            "failed to create tool output spool dir {}: {err}",
            dir.to_string_lossy()
        );
        return None;
    }
    let file_name = format!("{}.txt", sanitize_spool_segment(&tool_call_id));
    let path = dir.join(file_name);
    if let Err(err) = fs::write(&path, output.as_bytes()).await {
        tracing::warn!(
            "failed to write tool output spool {}: {err}",
            path.to_string_lossy()
        );
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

fn merge_streaming_text(prev: Option<&str>, next: &str) -> String {
    let prev = prev.unwrap_or("");
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.starts_with(next) {
        return prev.to_string();
    }
    if next.len() >= prev.len() {
        next.to_string()
    } else {
        prev.to_string()
    }
}

#[cfg(test)]
mod tool_preview_tests {
    use super::{
        build_text_preview, build_tool_ops_meta, build_turn_tool_update_from_payload,
        sanitize_tool_event_payload, TOOL_PREVIEW_MAX_LINES, TOOL_PREVIEW_MAX_LINE_CHARS,
    };
    use ctx_core::models::SessionEventType;
    use serde_json::json;

    #[test]
    fn preview_truncates_lines_and_counts() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line-{i}")).collect();
        let text = lines.join("\n");
        let preview = build_text_preview(&text);
        assert!(preview.truncated);
        assert_eq!(preview.preview.lines().count(), TOOL_PREVIEW_MAX_LINES);
        assert!(preview.preview.contains("... +6 lines"));
    }

    #[test]
    fn preview_truncates_long_lines() {
        let text = "x".repeat(TOOL_PREVIEW_MAX_LINE_CHARS + 20);
        let preview = build_text_preview(&text);
        assert!(preview.truncated);
        assert_eq!(preview.preview.chars().count(), TOOL_PREVIEW_MAX_LINE_CHARS);
    }

    #[test]
    fn sanitize_tool_payload_keeps_spool_path() {
        let raw = json!({
            "tool_call_id": "call-1",
            "output_text": "line1\nline2\nline3\nline4\nline5\nline6"
        });
        let sanitized = sanitize_tool_event_payload(
            &SessionEventType::ToolResult,
            &raw,
            Some("tool-output-spool/call-1.txt"),
        );
        assert!(sanitized.get("output_text").is_none());
        assert_eq!(
            sanitized.get("output_spool_path").and_then(|v| v.as_str()),
            Some("tool-output-spool/call-1.txt")
        );
        assert!(sanitized.get("output_preview").is_some());
        assert_eq!(
            sanitized.get("output_truncated").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn tool_title_falls_back_to_provider_tool_name_consistently() {
        let raw = json!({
            "tool_call_id": "call-2",
            "kind": "execute",
            "toolCall": {
                "name": "Bash",
                "kind": "execute",
            },
            "status": "running",
            "rawInput": { "command": "pwd" }
        });

        let sanitized = sanitize_tool_event_payload(&SessionEventType::ToolCall, &raw, None);
        assert_eq!(
            sanitized.get("title").and_then(|v| v.as_str()),
            Some("Bash")
        );
        assert_eq!(
            sanitized.get("subtitle").and_then(|v| v.as_str()),
            Some("pwd")
        );
        assert_eq!(
            sanitized.get("tool_name").and_then(|v| v.as_str()),
            Some("Bash")
        );

        let meta = build_tool_ops_meta(&SessionEventType::ToolCall, &raw);
        assert_eq!(meta.title.as_deref(), Some("Bash"));

        let update = build_turn_tool_update_from_payload(&SessionEventType::ToolCall, &raw)
            .expect("tool update");
        assert_eq!(update.title.as_deref(), Some("Bash"));
        assert_eq!(update.subtitle.as_deref(), Some("pwd"));
    }

    #[test]
    fn tool_title_prefers_explicit_labels_over_raw_name() {
        let raw = json!({
            "tool_call_id": "call-3",
            "kind": "execute",
            "tool_label": "Run shell",
            "toolCall": {
                "name": "Bash",
                "title": "Nested title",
            },
            "rawInput": {
                "description": "Run shell command"
            }
        });

        let sanitized = sanitize_tool_event_payload(&SessionEventType::ToolCall, &raw, None);
        assert_eq!(
            sanitized.get("title").and_then(|v| v.as_str()),
            Some("Run shell")
        );

        let meta = build_tool_ops_meta(&SessionEventType::ToolCall, &raw);
        assert_eq!(meta.title.as_deref(), Some("Run shell"));

        let update = build_turn_tool_update_from_payload(&SessionEventType::ToolCall, &raw)
            .expect("tool update");
        assert_eq!(update.title.as_deref(), Some("Run shell"));
        assert_eq!(update.subtitle.as_deref(), Some("Run shell command"));
    }

    #[test]
    fn sanitize_tool_payload_infers_claude_read_from_numbered_output_preview() {
        let raw = json!({
            "tool_call_id": "toolu_read_1",
            "kind": "unknown",
            "tool_name": "unknown",
            "title": "unknown",
            "status": "completed",
            "output_preview": "1→# agent instructions\n2→follow the rules"
        });

        let sanitized = sanitize_tool_event_payload(&SessionEventType::ToolResult, &raw, None);
        assert_eq!(sanitized.get("kind").and_then(|v| v.as_str()), Some("read"));
        assert_eq!(
            sanitized.get("tool_name").and_then(|v| v.as_str()),
            Some("Read")
        );
        assert_eq!(
            sanitized.get("title").and_then(|v| v.as_str()),
            Some("Read")
        );

        let update = build_turn_tool_update_from_payload(&SessionEventType::ToolResult, &sanitized)
            .expect("tool update");
        assert_eq!(update.tool_kind.as_deref(), Some("read"));
        assert_eq!(update.provider_tool_name.as_deref(), Some("Read"));
        assert_eq!(update.title.as_deref(), Some("Read"));
        assert_eq!(
            update.output_text.as_deref(),
            Some("1→# agent instructions\n2→follow the rules")
        );
    }

    #[test]
    fn sanitize_tool_payload_infers_claude_bash_from_scalar_output_preview() {
        let raw = json!({
            "tool_call_id": "toolu_exec_1",
            "kind": "unknown",
            "tool_name": "unknown",
            "title": "unknown",
            "status": "completed",
            "output_preview": "1"
        });

        let sanitized = sanitize_tool_event_payload(&SessionEventType::ToolResult, &raw, None);
        assert_eq!(
            sanitized.get("kind").and_then(|v| v.as_str()),
            Some("execute")
        );
        assert_eq!(
            sanitized.get("tool_name").and_then(|v| v.as_str()),
            Some("Bash")
        );
        assert_eq!(
            sanitized.get("title").and_then(|v| v.as_str()),
            Some("Bash")
        );
    }
}
