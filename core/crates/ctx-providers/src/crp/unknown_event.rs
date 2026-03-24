use serde_json::{json, Value};

const UNKNOWN_EVENT_MAX_DEPTH: usize = 5;
const UNKNOWN_EVENT_MAX_KEYS: usize = 24;
const UNKNOWN_EVENT_MAX_ITEMS: usize = 24;
const UNKNOWN_EVENT_MAX_STRING_CHARS: usize = 400;
const UNKNOWN_EVENT_SUMMARY_MAX_CHARS: usize = 160;

fn normalize_unknown_tool_label(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '_' | '-'))
        .collect()
}

fn non_placeholder_unknown_tool_label(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        let normalized = normalize_unknown_tool_label(trimmed);
        if normalized.is_empty()
            || normalized == "unknown"
            || normalized == "tool"
            || normalized == "unknowntool"
        {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn read_trimmed_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_joined_strings(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(Value::Array(items)) => {
            let parts = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .take(8)
                .collect::<Vec<_>>();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

fn truncate_summary_fragment(value: &str) -> String {
    let mut chars = value.chars();
    let truncated: String = chars
        .by_ref()
        .take(UNKNOWN_EVENT_SUMMARY_MAX_CHARS)
        .collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub(super) fn extract_unknown_crp_tool_name(raw: &Value) -> Option<String> {
    non_placeholder_unknown_tool_label(
        [
            read_trimmed_string(raw.get("tool_name")),
            read_trimmed_string(raw.get("toolName")),
            read_trimmed_string(raw.get("tool_label")),
            read_trimmed_string(raw.get("toolLabel")),
            read_trimmed_string(raw.pointer("/tool/name")),
            read_trimmed_string(raw.pointer("/tool/tool_name")),
            read_trimmed_string(raw.pointer("/toolCall/name")),
            read_trimmed_string(raw.pointer("/toolCall/tool_name")),
            read_trimmed_string(raw.pointer("/details/tool_name")),
            read_trimmed_string(raw.pointer("/details/toolName")),
            read_trimmed_string(raw.pointer("/details/tool_label")),
            read_trimmed_string(raw.pointer("/details/toolLabel")),
            read_trimmed_string(raw.pointer("/payload/tool_name")),
            read_trimmed_string(raw.pointer("/payload/toolName")),
            read_trimmed_string(raw.pointer("/payload/tool_label")),
            read_trimmed_string(raw.pointer("/payload/toolLabel")),
            read_trimmed_string(raw.pointer("/data/tool_name")),
            read_trimmed_string(raw.pointer("/data/toolName")),
            read_trimmed_string(raw.get("name")),
        ]
        .into_iter()
        .flatten()
        .next(),
    )
}

pub(super) fn extract_unknown_crp_tool_preview(raw: &Value) -> Option<String> {
    [
        read_joined_strings(raw.get("command")),
        read_joined_strings(raw.pointer("/details/command")),
        read_joined_strings(raw.pointer("/payload/command")),
        read_joined_strings(raw.pointer("/tool/command")),
        read_joined_strings(raw.pointer("/toolCall/command")),
        read_trimmed_string(raw.get("description")),
        read_trimmed_string(raw.pointer("/details/description")),
        read_trimmed_string(raw.pointer("/payload/description")),
        read_trimmed_string(raw.get("file_path")),
        read_trimmed_string(raw.get("filePath")),
        read_trimmed_string(raw.get("path")),
        read_trimmed_string(raw.pointer("/details/file_path")),
        read_trimmed_string(raw.pointer("/details/path")),
        read_trimmed_string(raw.pointer("/payload/file_path")),
        read_trimmed_string(raw.pointer("/payload/path")),
        read_trimmed_string(raw.get("query")),
        read_trimmed_string(raw.get("pattern")),
        read_trimmed_string(raw.get("regex")),
        read_trimmed_string(raw.pointer("/details/query")),
        read_trimmed_string(raw.pointer("/details/pattern")),
        read_trimmed_string(raw.pointer("/payload/query")),
        read_trimmed_string(raw.pointer("/payload/pattern")),
        read_trimmed_string(raw.get("message")),
        read_trimmed_string(raw.get("text")),
        read_trimmed_string(raw.get("title")),
        read_trimmed_string(raw.get("summary")),
    ]
    .into_iter()
    .flatten()
    .map(|value| truncate_summary_fragment(&value))
    .next()
}

pub(super) fn summarize_unknown_crp_event(raw: &Value) -> Option<String> {
    if let Some(tool_name) = extract_unknown_crp_tool_name(raw) {
        let preview = extract_unknown_crp_tool_preview(raw)
            .filter(|value| !value.eq_ignore_ascii_case(tool_name.trim()));
        return Some(match preview {
            Some(preview) => format!("Unknown tool event: {tool_name} · {preview}"),
            None => format!("Unknown tool event: {tool_name}"),
        });
    }
    let summary = raw
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| raw.get("text").and_then(Value::as_str))
        .or_else(|| raw.get("title").and_then(Value::as_str))
        .or_else(|| raw.get("description").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(format!("Unknown runtime event: {summary}"))
}

fn truncate_unknown_string(value: &str) -> (String, bool) {
    let mut chars = value.chars();
    let truncated: String = chars
        .by_ref()
        .take(UNKNOWN_EVENT_MAX_STRING_CHARS)
        .collect();
    if chars.next().is_some() {
        (format!("{truncated}..."), true)
    } else {
        (truncated, false)
    }
}

pub(super) fn bound_unknown_crp_payload(value: Value) -> (Value, bool) {
    bound_unknown_crp_payload_inner(value, 0)
}

fn bound_unknown_crp_payload_inner(value: Value, depth: usize) -> (Value, bool) {
    if depth >= UNKNOWN_EVENT_MAX_DEPTH {
        return (json!("[truncated]"), true);
    }
    match value {
        Value::String(text) => {
            let (text, truncated) = truncate_unknown_string(&text);
            (Value::String(text), truncated)
        }
        Value::Array(items) => {
            let mut truncated = false;
            let total = items.len();
            let mut out = Vec::with_capacity(total.min(UNKNOWN_EVENT_MAX_ITEMS) + 1);
            for item in items.into_iter().take(UNKNOWN_EVENT_MAX_ITEMS) {
                let (bounded, item_truncated) = bound_unknown_crp_payload_inner(item, depth + 1);
                truncated |= item_truncated;
                out.push(bounded);
            }
            if total > UNKNOWN_EVENT_MAX_ITEMS {
                truncated = true;
                out.push(json!({
                    "_truncated_items": total - UNKNOWN_EVENT_MAX_ITEMS,
                }));
            }
            (Value::Array(out), truncated)
        }
        Value::Object(map) => {
            let mut truncated = false;
            let total = map.len();
            let mut out = serde_json::Map::with_capacity(total.min(UNKNOWN_EVENT_MAX_KEYS) + 1);
            for (idx, (key, value)) in map.into_iter().enumerate() {
                if idx >= UNKNOWN_EVENT_MAX_KEYS {
                    truncated = true;
                    break;
                }
                let (bounded, value_truncated) = bound_unknown_crp_payload_inner(value, depth + 1);
                truncated |= value_truncated;
                out.insert(key, bounded);
            }
            if total > UNKNOWN_EVENT_MAX_KEYS {
                out.insert(
                    "_truncated_keys".to_string(),
                    json!(total - UNKNOWN_EVENT_MAX_KEYS),
                );
            }
            (Value::Object(out), truncated)
        }
        other => (other, false),
    }
}
