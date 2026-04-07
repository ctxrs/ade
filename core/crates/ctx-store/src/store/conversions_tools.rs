use super::*;

pub(super) fn summarize_session_turn_tool(tool: &SessionTurnTool) -> SessionTurnToolSummary {
    SessionTurnToolSummary {
        session_id: tool.session_id,
        tool_call_id: tool.tool_call_id.clone(),
        turn_id: tool.turn_id,
        tool_kind: tool.tool_kind.clone(),
        provider_tool_name: tool.provider_tool_name.clone(),
        title: tool.title.clone(),
        subtitle: tool.subtitle.clone(),
        status: tool.status.clone(),
        input_preview: tool_input_preview_from_value(tool.input_json.as_ref()),
        output_preview: tool.output_text.clone(),
        order_seq: tool.order_seq,
        first_event_seq: tool.first_event_seq,
        input_truncated: tool.input_truncated,
        input_original_bytes: tool.input_original_bytes,
        output_truncated: tool.output_truncated,
        output_original_bytes: tool.output_original_bytes,
        created_at: tool.created_at,
        updated_at: tool.updated_at,
    }
}

pub(super) fn compare_tool_summary_order(
    a: &SessionTurnToolSummary,
    b: &SessionTurnToolSummary,
) -> std::cmp::Ordering {
    a.order_seq
        .cmp(&b.order_seq)
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.tool_call_id.cmp(&b.tool_call_id))
}

pub(super) fn compare_tool_order(a: &SessionTurnTool, b: &SessionTurnTool) -> std::cmp::Ordering {
    a.order_seq
        .cmp(&b.order_seq)
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.tool_call_id.cmp(&b.tool_call_id))
}

fn read_event_order_seq(payload: &Value) -> Option<i64> {
    payload
        .get("order_seq")
        .and_then(Value::as_i64)
        .or_else(|| payload.get("orderSeq").and_then(Value::as_i64))
}

pub(super) const TOOL_PREVIEW_MAX_LINES: usize = 5;
pub(super) const TOOL_PREVIEW_MAX_LINE_CHARS: usize = 80;

pub(super) struct ToolTextPreview {
    preview: String,
    truncated: bool,
    original_bytes: usize,
}

pub(super) struct ToolJsonPreview {
    preview: Option<Value>,
    truncated: Option<bool>,
    original_bytes: Option<i64>,
}

pub(super) fn tool_input_preview_from_value(input: Option<&Value>) -> Option<Value> {
    let input = input?;
    let obj = input.as_object()?;
    let mut out = serde_json::Map::new();
    for key in [
        "command",
        "query",
        "pattern",
        "text",
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "paths",
        "paths_total",
        "files",
        "file_paths",
        "filePaths",
        "target",
        "glob",
        "parsed_cmd",
        "cwd",
        "root",
        "url",
        "uri",
        "href",
        "method",
        "regex",
        "diff_stats",
        "description",
    ] {
        if let Some(value) = obj.get(key) {
            if value.is_string() || value.is_number() || value.is_array() || value.is_object() {
                out.insert(key.to_string(), value.clone());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        let value = Value::Object(out);
        let mut truncated = false;
        Some(truncate_preview_value(&value, &mut truncated))
    }
}

pub(super) fn truncate_preview_value(value: &Value, truncated: &mut bool) -> Value {
    match value {
        Value::String(value) => {
            let preview = build_text_preview(value);
            if preview.truncated {
                *truncated = true;
            }
            Value::String(preview.preview)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| truncate_preview_value(value, truncated))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                out.insert(key.clone(), truncate_preview_value(value, truncated));
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

pub(super) fn push_text_preview_line(out: &mut Vec<String>, truncated: &mut bool, line: &str) {
    if line.chars().count() > TOOL_PREVIEW_MAX_LINE_CHARS {
        *truncated = true;
        out.push(line.chars().take(TOOL_PREVIEW_MAX_LINE_CHARS).collect());
    } else {
        out.push(line.to_string());
    }
}

pub(super) fn build_text_preview(text: &str) -> ToolTextPreview {
    let original_bytes = text.len();
    let mut truncated = false;
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    let mut out = Vec::new();

    if total_lines <= TOOL_PREVIEW_MAX_LINES {
        for line in lines {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
    } else {
        truncated = true;
        let head_count = TOOL_PREVIEW_MAX_LINES / 2;
        let tail_count = TOOL_PREVIEW_MAX_LINES.saturating_sub(head_count + 1);
        for line in lines.iter().take(head_count) {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
        let omitted = total_lines.saturating_sub(head_count + tail_count);
        out.push(format!("... +{omitted} lines"));
        for line in lines.iter().skip(total_lines - tail_count) {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
    }

    ToolTextPreview {
        preview: out.join("\n"),
        truncated,
        original_bytes,
    }
}

pub(super) fn build_json_preview(input: Option<&Value>, preview: Option<Value>) -> ToolJsonPreview {
    let original_bytes = input
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| value.len() as i64);
    let mut preview_truncated = false;
    let preview = preview.map(|value| truncate_preview_value(&value, &mut preview_truncated));
    let preview_bytes = preview
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| value.len() as i64);
    let mut truncated = preview_truncated;
    if let (Some(original), Some(preview_bytes)) = (original_bytes, preview_bytes) {
        if original > preview_bytes {
            truncated = true;
        }
    } else if original_bytes.is_some() && preview.is_none() {
        truncated = true;
    }
    let truncated = if original_bytes.is_some() || preview.is_some() {
        Some(truncated)
    } else {
        None
    };
    ToolJsonPreview {
        preview,
        truncated,
        original_bytes,
    }
}

pub(super) fn build_output_preview(text: &str) -> ToolTextPreview {
    build_text_preview(text)
}

pub(super) fn normalize_tool_status(status: &str, event_type: SessionEventType) -> String {
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

pub(super) fn merge_tool_status(existing: Option<&str>, next: &str) -> String {
    match (existing, next) {
        (Some(current @ ("completed" | "failed")), "pending" | "in_progress") => {
            current.to_string()
        }
        _ => next.to_string(),
    }
}

pub(super) fn extract_tool_update(payload: &Value) -> &Value {
    payload
}

fn tool_kind_from_update(update: &Value) -> Option<String> {
    update
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| update.pointer("/toolCall/kind").and_then(Value::as_str))
        .map(str::to_owned)
}

fn string_from_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
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

fn preview_string(input: &Value, key: &str) -> Option<String> {
    input.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn preview_command(input: &Value) -> Option<String> {
    match input.get("command") {
        Some(Value::String(value)) => {
            Some(value.trim().to_owned()).filter(|value| !value.is_empty())
        }
        Some(Value::Array(parts)) => {
            let joined = parts
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            Some(joined).filter(|value| !value.is_empty())
        }
        _ => None,
    }
}

fn preview_path_summary(input: &Value) -> Option<String> {
    let direct = [
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "target",
    ]
    .into_iter()
    .find_map(|key| preview_string(input, key));
    if direct.is_some() {
        return direct;
    }
    for key in ["paths", "files", "file_paths", "filePaths"] {
        if let Some(Value::Array(values)) = input.get(key) {
            let paths = values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if let Some(first) = paths.first() {
                let more = paths.len().saturating_sub(1);
                return Some(if more > 0 {
                    format!("{first} +{more} more")
                } else {
                    (*first).to_owned()
                });
            }
        }
    }
    None
}

fn format_diff_stats(input: &Value) -> Option<String> {
    let stats = input.get("diff_stats")?.as_object()?;
    let added = stats.get("added").and_then(Value::as_i64).unwrap_or(0);
    let removed = stats.get("removed").and_then(Value::as_i64).unwrap_or(0);
    let files = stats.get("files").and_then(Value::as_i64).unwrap_or(0);
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("+{added}"));
    }
    if removed > 0 {
        parts.push(format!("-{removed}"));
    }
    if parts.is_empty() && files > 0 {
        parts.push(format!("{files} files"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("({})", parts.join(" ")))
    }
}

fn tool_subtitle_from_preview(
    tool_kind: Option<&str>,
    provider_tool_name: Option<&str>,
    input: Option<&Value>,
) -> Option<String> {
    let input = input?;
    input.as_object()?;
    if let Some(description) = preview_string(input, "description") {
        return Some(description);
    }
    let kind_hint = tool_kind
        .or(provider_tool_name)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let path = preview_path_summary(input);
    let query = preview_string(input, "query")
        .or_else(|| preview_string(input, "pattern"))
        .or_else(|| preview_string(input, "regex"))
        .or_else(|| preview_string(input, "text"));
    let command = preview_command(input);
    let glob = preview_string(input, "glob").or_else(|| preview_string(input, "pattern"));
    let url = preview_string(input, "url")
        .or_else(|| preview_string(input, "uri"))
        .or_else(|| preview_string(input, "href"));

    let combine_query_path = |q: String, path: Option<String>| match path {
        Some(path) => format!("{q} in {path}"),
        None => q,
    };
    let combine_path_stats = |path: Option<String>, stats: Option<String>| match (path, stats) {
        (Some(path), Some(stats)) => Some(format!("{path} {stats}")),
        (Some(path), None) => Some(path),
        (None, Some(stats)) => Some(stats),
        (None, None) => None,
    };

    match kind_hint.as_str() {
        "execute" | "exec" | "shell" | "bash" => command,
        "search" | "web_search" | "grep" => {
            query.map(|q| combine_query_path(q, path.clone())).or(path)
        }
        "glob" => glob.or(path),
        "list" | "list_files" => path,
        "read" | "read_file" => path,
        "edit" | "write" | "apply_patch" | "patch" => {
            combine_path_stats(path, format_diff_stats(input))
        }
        "fetch" | "http" | "curl" => {
            let method = preview_string(input, "method").unwrap_or_else(|| "GET".to_owned());
            url.map(|url| format!("{} {}", method.trim().to_uppercase(), url))
                .or(Some(method.trim().to_uppercase()))
        }
        _ => path.or(query).or(command).or(glob).or(url),
    }
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

fn tool_status_from_update(update: &Value) -> Option<&str> {
    update
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| update.pointer("/toolCall/status").and_then(Value::as_str))
}

pub(super) fn sanitize_tool_event_payload(
    event_type: &SessionEventType,
    raw_payload: &Value,
) -> Value {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload).unwrap_or_default();

    let tool_kind = tool_kind_from_update(update);
    let tool_label = tool_label_from_update(update);
    let tool_name = tool_name_from_update(update);
    let title = tool_title_from_update(update);
    let raw_status = tool_status_from_update(update);
    let status = if let Some(raw_status) = raw_status {
        normalize_tool_status(raw_status, event_type.clone())
    } else if matches!(event_type, SessionEventType::ToolResult) {
        "completed".to_string()
    } else {
        "pending".to_string()
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_preview = tool_input_preview_from_value(input);
    let input_preview = input_preview.or_else(|| update.get("input_preview").cloned());
    let input_meta = build_json_preview(input, input_preview);
    let subtitle = tool_subtitle_from_preview(
        tool_kind.as_deref(),
        tool_name.as_deref(),
        input_meta.preview.as_ref(),
    );

    let output_meta = extract_tool_output_text(update)
        .map(|output| build_output_preview(&output))
        .filter(|preview| !preview.preview.trim().is_empty());
    let output_artifact = update
        .get("output_artifact")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| {
            raw_payload
                .get("output_artifact")
                .filter(|value| value.is_object())
                .cloned()
        });

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
    let input_truncated = update
        .get("input_truncated")
        .and_then(|v| v.as_bool())
        .or(input_meta.truncated);
    let input_original_bytes = update
        .get("input_original_bytes")
        .and_then(|v| v.as_i64())
        .or(input_meta.original_bytes);
    if let Some(truncated) = input_truncated {
        obj.insert("input_truncated".to_string(), Value::Bool(truncated));
    }
    if let Some(bytes) = input_original_bytes {
        obj.insert(
            "input_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(bytes)),
        );
    }

    if let Some(output_meta) = output_meta {
        obj.insert(
            "output_preview".to_string(),
            Value::String(output_meta.preview),
        );
        let output_truncated = update
            .get("output_truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(output_meta.truncated);
        let output_original_bytes = update
            .get("output_original_bytes")
            .and_then(|v| v.as_i64())
            .unwrap_or(output_meta.original_bytes as i64);
        obj.insert(
            "output_truncated".to_string(),
            Value::Bool(output_truncated),
        );
        obj.insert(
            "output_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(output_original_bytes)),
        );
    }
    if let Some(artifact) = output_artifact {
        obj.insert("output_artifact".to_string(), artifact);
    }

    if let Some(value) = raw_payload
        .get("crp_seq")
        .or_else(|| raw_payload.get("crpSeq"))
        .or_else(|| update.get("crp_seq"))
        .or_else(|| update.get("crpSeq"))
    {
        obj.insert("crp_seq".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("order_seq")
        .or_else(|| raw_payload.get("orderSeq"))
        .or_else(|| update.get("order_seq"))
        .or_else(|| update.get("orderSeq"))
    {
        obj.insert("order_seq".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("crp_channel")
        .or_else(|| raw_payload.get("crpChannel"))
        .or_else(|| update.get("crp_channel"))
        .or_else(|| update.get("crpChannel"))
    {
        obj.insert("crp_channel".to_string(), value.clone());
    }

    Value::Object(obj)
}

pub(super) fn build_turn_tool_from_event(
    event: &SessionEvent,
    turn_id: TurnId,
) -> Option<SessionTurnTool> {
    if !matches!(
        event.event_type,
        SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
    ) {
        return None;
    }
    let tool_call_id = tool_call_id_from_payload(&event.payload_json)?;
    let update = extract_tool_update(&event.payload_json);

    let tool_kind = tool_kind_from_update(update);
    let provider_tool_name = tool_name_from_update(update);
    let title = tool_title_from_update(update);
    let raw_status = tool_status_from_update(update);
    let status = if let Some(raw_status) = raw_status {
        Some(normalize_tool_status(raw_status, event.event_type.clone()))
    } else if matches!(event.event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else if matches!(event.event_type, SessionEventType::ToolCall) {
        Some("pending".to_string())
    } else {
        None
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_preview = tool_input_preview_from_value(input);
    let input_preview = input_preview.or_else(|| update.get("input_preview").cloned());
    let input_meta = build_json_preview(input, input_preview);
    let subtitle = tool_subtitle_from_preview(
        tool_kind.as_deref(),
        provider_tool_name.as_deref(),
        input_meta.preview.as_ref(),
    );
    let input_truncated = update
        .get("input_truncated")
        .and_then(|v| v.as_bool())
        .or(input_meta.truncated);
    let input_original_bytes = update
        .get("input_original_bytes")
        .and_then(|v| v.as_i64())
        .or(input_meta.original_bytes);

    let output_meta = extract_tool_output_text(update)
        .map(|output| build_output_preview(&output))
        .filter(|preview| !preview.preview.trim().is_empty());
    let output_truncated = update
        .get("output_truncated")
        .and_then(|v| v.as_bool())
        .or(output_meta.as_ref().map(|preview| preview.truncated));
    let output_original_bytes = update
        .get("output_original_bytes")
        .and_then(|v| v.as_i64())
        .or(output_meta
            .as_ref()
            .map(|preview| preview.original_bytes as i64));

    Some(SessionTurnTool {
        session_id: event.session_id,
        tool_call_id,
        turn_id,
        tool_kind,
        provider_tool_name,
        title,
        subtitle,
        status,
        input_json: input_meta.preview,
        output_text: output_meta.as_ref().map(|preview| preview.preview.clone()),
        order_seq: read_event_order_seq(&event.payload_json)?,
        first_event_seq: Some(event.seq),
        input_truncated,
        input_original_bytes,
        output_truncated,
        output_original_bytes,
        created_at: event.created_at,
        updated_at: event.created_at,
    })
}

pub(super) fn tool_call_id_from_payload(payload: &Value) -> Option<String> {
    let direct = payload.get("tool_call_id").and_then(|v| v.as_str());
    if let Some(v) = direct {
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

pub(super) fn extract_tool_output_text(update: &Value) -> Option<String> {
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

pub(super) fn build_turn_tools_from_events(
    session_id: SessionId,
    turn_id: TurnId,
    events: &[SessionEvent],
) -> Vec<SessionTurnTool> {
    #[derive(Default)]
    struct ToolAgg {
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
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        first_event_seq: Option<i64>,
        initialized: bool,
    }

    let mut map: HashMap<String, ToolAgg> = HashMap::new();

    for ev in events {
        if !matches!(
            ev.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            continue;
        }
        let tool_call_id = match tool_call_id_from_payload(&ev.payload_json) {
            Some(v) => v,
            None => continue,
        };
        let update = extract_tool_update(&ev.payload_json);
        let entry = map.entry(tool_call_id.clone()).or_default();
        if !entry.initialized {
            entry.created_at = ev.created_at;
            entry.updated_at = ev.created_at;
            entry.first_event_seq = Some(ev.seq);
            entry.order_seq = read_event_order_seq(&ev.payload_json);
            entry.initialized = true;
        } else {
            entry.updated_at = ev.created_at;
            if entry
                .first_event_seq
                .is_none_or(|existing| ev.seq < existing)
            {
                entry.first_event_seq = Some(ev.seq);
                entry.created_at = ev.created_at;
            }
            if let Some(order_seq) = read_event_order_seq(&ev.payload_json) {
                entry.order_seq = Some(match entry.order_seq {
                    Some(existing) => existing.min(order_seq),
                    None => order_seq,
                });
            }
        }

        if let Some(v) = tool_kind_from_update(update) {
            entry.tool_kind = Some(v);
        }
        if let Some(v) = tool_name_from_update(update) {
            entry.provider_tool_name = Some(v);
        }
        if let Some(v) = tool_title_from_update(update) {
            entry.title = Some(v);
        }
        if let Some(raw) = tool_status_from_update(update) {
            let normalized = normalize_tool_status(raw, ev.event_type.clone());
            entry.status = Some(merge_tool_status(entry.status.as_deref(), &normalized));
        } else if matches!(ev.event_type, SessionEventType::ToolResult) {
            entry.status = Some(merge_tool_status(entry.status.as_deref(), "completed"));
        } else if matches!(ev.event_type, SessionEventType::ToolCall) && entry.status.is_none() {
            entry.status = Some("pending".to_string());
        }

        let input = update
            .pointer("/rawInput")
            .or_else(|| update.pointer("/toolCall/rawInput"))
            .or_else(|| update.pointer("/toolCall/input"))
            .or_else(|| update.pointer("/input"))
            .or_else(|| update.pointer("/args"));
        let input_preview = tool_input_preview_from_value(input);
        let input_preview = input_preview.or_else(|| update.get("input_preview").cloned());
        let input_meta = build_json_preview(input, input_preview);
        if let Some(value) = input_meta.preview {
            entry.input_json = Some(value);
        }
        if let Some(value) = tool_subtitle_from_preview(
            entry.tool_kind.as_deref(),
            entry.provider_tool_name.as_deref(),
            entry.input_json.as_ref(),
        ) {
            entry.subtitle = Some(value);
        }
        if let Some(value) = update
            .get("input_truncated")
            .and_then(|v| v.as_bool())
            .or(input_meta.truncated)
        {
            entry.input_truncated = Some(value);
        }
        if let Some(value) = update
            .get("input_original_bytes")
            .and_then(|v| v.as_i64())
            .or(input_meta.original_bytes)
        {
            entry.input_original_bytes = Some(value);
        }

        if let Some(output) = extract_tool_output_text(update) {
            let preview = build_output_preview(&output);
            entry.output_text = Some(preview.preview.clone());
            entry.output_truncated = Some(
                update
                    .get("output_truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(preview.truncated),
            );
            entry.output_original_bytes = Some(
                update
                    .get("output_original_bytes")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(preview.original_bytes as i64),
            );
        }
    }

    let mut out = Vec::with_capacity(map.len());
    for (tool_call_id, agg) in map {
        if !agg.initialized {
            continue;
        }
        out.push(SessionTurnTool {
            session_id,
            tool_call_id,
            turn_id,
            tool_kind: agg.tool_kind,
            provider_tool_name: agg.provider_tool_name,
            title: agg.title,
            subtitle: agg.subtitle,
            status: agg.status,
            input_json: agg.input_json,
            output_text: agg.output_text,
            order_seq: match agg.order_seq {
                Some(order_seq) => order_seq,
                None => continue,
            },
            first_event_seq: agg.first_event_seq,
            input_truncated: agg.input_truncated,
            input_original_bytes: agg.input_original_bytes,
            output_truncated: agg.output_truncated,
            output_original_bytes: agg.output_original_bytes,
            created_at: agg.created_at,
            updated_at: agg.updated_at,
        });
    }
    out.sort_by(compare_tool_order);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_turn_tools_from_events_keeps_latest_preview_instead_of_merging() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let created_at = Utc::now();
        let update_event = SessionEvent {
            seq: 1,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: Some(turn_id),
            event_type: SessionEventType::ToolCallUpdate,
            payload_json: serde_json::json!({
                "tool_call_id": "tool-1",
                "order_seq": 1,
                "status": "running",
                "output_preview": "line-1\nline-2\n... +4 lines\nline-7\nline-8",
                "output_truncated": true,
                "output_original_bytes": 64
            }),
            transient: true,
            created_at,
        };
        let result_event = SessionEvent {
            seq: 2,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: Some(turn_id),
            event_type: SessionEventType::ToolResult,
            payload_json: serde_json::json!({
                "tool_call_id": "tool-1",
                "order_seq": 1,
                "status": "completed",
                "output_preview": "line-1\nline-2\n... +6 lines\nline-9\nline-10",
                "output_truncated": true,
                "output_original_bytes": 80
            }),
            transient: false,
            created_at,
        };

        let tools =
            build_turn_tools_from_events(session_id, turn_id, &[update_event, result_event]);
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].output_text.as_deref(),
            Some("line-1\nline-2\n... +6 lines\nline-9\nline-10")
        );
        assert_eq!(tools[0].output_original_bytes, Some(80));
        assert_eq!(tools[0].status.as_deref(), Some("completed"));
    }
}
