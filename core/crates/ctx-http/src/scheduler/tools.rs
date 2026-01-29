use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::fs;

use ctx_core::models::{SessionEventType, SessionTurnTool};

use crate::daemon::AppState;

#[derive(Default)]
struct DiffStats {
    added: usize,
    removed: usize,
    files: usize,
}

const MAX_PREVIEW_PATHS: usize = 5;
const TOOL_PREVIEW_MAX_LINES: usize = 5;
const TOOL_PREVIEW_MAX_LINE_CHARS: usize = 80;

struct ToolTextPreview {
    preview: String,
    truncated: bool,
    original_bytes: usize,
}

struct ToolJsonPreview {
    preview: Option<Value>,
    truncated: Option<bool>,
    original_bytes: Option<i64>,
}

fn push_preview_line(out: &mut Vec<String>, truncated: &mut bool, line: &str) {
    if line.chars().count() > TOOL_PREVIEW_MAX_LINE_CHARS {
        *truncated = true;
        out.push(line.chars().take(TOOL_PREVIEW_MAX_LINE_CHARS).collect());
    } else {
        out.push(line.to_string());
    }
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn diff_stats_from_patch(patch: &str) -> Option<DiffStats> {
    let mut stats = DiffStats::default();
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            stats.files += 1;
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        if line.starts_with('+') {
            stats.added += 1;
            continue;
        }
        if line.starts_with('-') {
            stats.removed += 1;
        }
    }
    if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
        stats.files = 1;
    }
    if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
        None
    } else {
        Some(stats)
    }
}

fn extract_old_new_text(obj: &serde_json::Map<String, Value>) -> (Option<&str>, Option<&str>) {
    let old_text = obj
        .get("oldText")
        .or_else(|| obj.get("old_text"))
        .or_else(|| obj.get("old"))
        .or_else(|| obj.get("before"))
        .and_then(|v| v.as_str());
    let new_text = obj
        .get("newText")
        .or_else(|| obj.get("new_text"))
        .or_else(|| obj.get("new"))
        .or_else(|| obj.get("text"))
        .or_else(|| obj.get("after"))
        .and_then(|v| v.as_str());
    (old_text, new_text)
}

fn diff_stats_from_edits(edits: &[Value]) -> Option<DiffStats> {
    let mut stats = DiffStats::default();
    let mut files = HashSet::new();
    for edit in edits {
        let Some(obj) = edit.as_object() else {
            continue;
        };
        for key in [
            "path",
            "file",
            "file_path",
            "filePath",
            "filepath",
            "target",
        ] {
            if let Some(Value::String(path)) = obj.get(key) {
                files.insert(path.clone());
            }
        }
        let (old_text, new_text) = extract_old_new_text(obj);
        stats.removed += count_lines(old_text.unwrap_or(""));
        stats.added += count_lines(new_text.unwrap_or(""));
    }
    stats.files = files.len();
    if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
        stats.files = 1;
    }
    if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
        None
    } else {
        Some(stats)
    }
}

fn diff_stats_from_changes(changes: &Value) -> Option<DiffStats> {
    match changes {
        Value::Array(edits) => diff_stats_from_edits(edits),
        Value::String(text) => diff_stats_from_patch(text),
        Value::Object(map) => {
            let mut stats = DiffStats::default();
            let mut files = HashSet::new();
            for (path, entry) in map {
                if !path.trim().is_empty() {
                    files.insert(path.clone());
                }
                if let Some(patch) = extract_patch_text(entry) {
                    if let Some(patch_stats) = diff_stats_from_patch(patch) {
                        stats.added += patch_stats.added;
                        stats.removed += patch_stats.removed;
                        stats.files = stats.files.max(patch_stats.files);
                    }
                    continue;
                }
                if let Some(text) = entry.as_str() {
                    if let Some(patch_stats) = diff_stats_from_patch(text) {
                        stats.added += patch_stats.added;
                        stats.removed += patch_stats.removed;
                        stats.files = stats.files.max(patch_stats.files);
                        continue;
                    }
                }
                if let Some(obj) = entry.as_object() {
                    let (old_text, new_text) = extract_old_new_text(obj);
                    stats.removed += count_lines(old_text.unwrap_or(""));
                    stats.added += count_lines(new_text.unwrap_or(""));
                }
            }
            stats.files = stats.files.max(files.len());
            if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
                stats.files = 1;
            }
            if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
                None
            } else {
                Some(stats)
            }
        }
        _ => None,
    }
}

fn diff_stats_from_content_blocks(blocks: &[&Value]) -> Option<DiffStats> {
    let mut stats = DiffStats::default();
    let mut files = HashSet::new();
    for block in blocks {
        let candidate = block.get("content").unwrap_or(block);
        if let Some(obj) = candidate.as_object() {
            for key in [
                "path",
                "file",
                "file_path",
                "filePath",
                "filepath",
                "target",
            ] {
                if let Some(Value::String(path)) = obj.get(key) {
                    files.insert(path.clone());
                }
            }
            if let Some(patch) = extract_patch_text(candidate) {
                if let Some(patch_stats) = diff_stats_from_patch(patch) {
                    stats.added += patch_stats.added;
                    stats.removed += patch_stats.removed;
                    stats.files = stats.files.max(patch_stats.files);
                    continue;
                }
            }
            let (old_text, new_text) = extract_old_new_text(obj);
            stats.removed += count_lines(old_text.unwrap_or(""));
            stats.added += count_lines(new_text.unwrap_or(""));
        } else if let Some(text) = candidate.as_str() {
            if let Some(patch_stats) = diff_stats_from_patch(text) {
                stats.added += patch_stats.added;
                stats.removed += patch_stats.removed;
                stats.files = stats.files.max(patch_stats.files);
            }
        }
    }
    stats.files = stats.files.max(files.len());
    if stats.files == 0 && (stats.added > 0 || stats.removed > 0) {
        stats.files = 1;
    }
    if stats.files == 0 && stats.added == 0 && stats.removed == 0 {
        None
    } else {
        Some(stats)
    }
}

fn extract_patch_text(input: &Value) -> Option<&str> {
    if let Some(text) = input.as_str() {
        return Some(text);
    }
    input
        .get("patch")
        .or_else(|| input.get("diff"))
        .or_else(|| input.get("patch_text"))
        .or_else(|| input.get("unified_diff"))
        .and_then(|v| v.as_str())
}

fn extract_patch_text_from_changes(changes: &Value) -> Option<String> {
    match changes {
        Value::String(text) => Some(text.to_string()),
        Value::Array(items) => {
            for item in items {
                if let Some(patch) = extract_patch_text(item) {
                    return Some(patch.to_string());
                }
                if let Some(text) = item.as_str() {
                    return Some(text.to_string());
                }
            }
            None
        }
        Value::Object(map) => {
            for (_path, entry) in map {
                if let Some(patch) = extract_patch_text(entry) {
                    return Some(patch.to_string());
                }
                if let Some(text) = entry.as_str() {
                    return Some(text.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_content_blocks(update: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    for candidate in [update.get("content"), update.pointer("/toolCall/content")] {
        if let Some(items) = candidate.and_then(|v| v.as_array()) {
            for item in items {
                out.push(item);
            }
        }
    }
    out
}

fn collect_paths_from_value(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::String(path) => {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                paths.push(trimmed.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_value(item, paths);
            }
        }
        Value::Object(obj) => {
            for key in [
                "path",
                "file",
                "filename",
                "file_path",
                "filePath",
                "filepath",
                "target",
                "uri",
            ] {
                if let Some(Value::String(path)) = obj.get(key) {
                    let trimmed = path.trim();
                    if !trimmed.is_empty() {
                        paths.push(trimmed.to_string());
                    }
                }
            }
            for key in ["paths", "files", "file_paths", "filePaths"] {
                if let Some(value) = obj.get(key) {
                    collect_paths_from_value(value, paths);
                }
            }
            if let Some(Value::Array(cmds)) = obj.get("parsed_cmd") {
                for cmd in cmds {
                    if let Some(path) = cmd.get("path") {
                        collect_paths_from_value(path, paths);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_paths_from_changes(value: &Value, paths: &mut Vec<String>) {
    let Some(changes) = value.as_object() else {
        return;
    };
    for (path, _) in changes {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            paths.push(trimmed.to_string());
        }
    }
}

fn dedupe_paths(paths: &mut Vec<String>) {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in paths.drain(..) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    *paths = out;
}

fn extract_paths_from_update(update: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    for value in [
        update.get("locations"),
        update.pointer("/toolCall/locations"),
    ]
    .into_iter()
    .flatten()
    {
        collect_paths_from_value(value, &mut paths);
    }
    for block in extract_content_blocks(update) {
        let candidate = block.get("content").unwrap_or(block);
        collect_paths_from_value(candidate, &mut paths);
    }
    dedupe_paths(&mut paths);
    paths
}

fn extract_tool_input(update: &Value) -> Option<&Value> {
    update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/raw_input"))
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/raw_input"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"))
}

fn is_edit_tool(tool_kind: Option<&str>, title: Option<&str>) -> bool {
    let kind = tool_kind.unwrap_or("").trim().to_lowercase();
    if matches!(kind.as_str(), "edit" | "write" | "apply_patch" | "patch") {
        return true;
    }
    let title = title.unwrap_or("").trim().to_lowercase();
    title.contains("edit") || title.contains("patch") || title.contains("apply")
}

fn tool_input_preview(
    input: Option<&Value>,
    update: &Value,
    tool_kind: Option<&str>,
    title: Option<&str>,
) -> Option<Value> {
    let mut out = serde_json::Map::new();
    let obj = input.and_then(|value| value.as_object());
    if let Some(obj) = obj {
        for key in [
            "command",
            "query",
            "pattern",
            "regex",
            "text",
            "path",
            "file",
            "filename",
            "file_path",
            "filePath",
            "filepath",
            "target",
            "paths",
            "files",
            "file_paths",
            "filePaths",
            "glob",
            "parsed_cmd",
            "url",
            "uri",
            "href",
            "method",
            "cwd",
            "root",
        ] {
            if let Some(value) = obj.get(key) {
                if matches!(key, "paths" | "files" | "file_paths" | "filePaths") {
                    let mut paths = Vec::new();
                    collect_paths_from_value(value, &mut paths);
                    dedupe_paths(&mut paths);
                    if !paths.is_empty() {
                        out.insert(
                            key.to_string(),
                            Value::Array(paths.into_iter().map(Value::String).collect()),
                        );
                    }
                    continue;
                }
                if value.is_string() || value.is_array() || value.is_object() {
                    out.insert(key.to_string(), value.clone());
                }
            }
        }
    }

    let mut paths = Vec::new();
    if let Some(input) = input {
        collect_paths_from_value(input, &mut paths);
        if let Some(changes) = input.get("changes") {
            collect_paths_from_changes(changes, &mut paths);
        }
    }
    paths.extend(extract_paths_from_update(update));
    if let Some(changes) = update.get("changes") {
        collect_paths_from_changes(changes, &mut paths);
    }
    dedupe_paths(&mut paths);
    if !paths.is_empty() {
        let total_paths = paths.len();
        if total_paths > MAX_PREVIEW_PATHS {
            paths.truncate(MAX_PREVIEW_PATHS);
            out.insert(
                "paths_total".to_string(),
                Value::Number(serde_json::Number::from(total_paths as u64)),
            );
        }
        if !out.contains_key("path")
            && !out.contains_key("file")
            && !out.contains_key("filename")
            && !out.contains_key("file_path")
            && !out.contains_key("filePath")
            && !out.contains_key("filepath")
            && !out.contains_key("target")
            && paths.len() == 1
        {
            out.insert("path".to_string(), Value::String(paths[0].clone()));
        }
        if out.contains_key("paths") || paths.len() > 1 {
            out.insert(
                "paths".to_string(),
                Value::Array(paths.iter().cloned().map(Value::String).collect()),
            );
        }
    }

    if is_edit_tool(tool_kind, title) {
        let content_blocks = extract_content_blocks(update);
        let stats = input
            .and_then(extract_patch_text)
            .and_then(diff_stats_from_patch)
            .or_else(|| {
                input
                    .and_then(|value| value.get("edits"))
                    .and_then(|v| v.as_array())
                    .and_then(|edits| diff_stats_from_edits(edits))
            })
            .or_else(|| {
                input
                    .and_then(|value| value.get("changes"))
                    .and_then(diff_stats_from_changes)
            })
            .or_else(|| diff_stats_from_content_blocks(&content_blocks));
        if let Some(mut stats) = stats {
            if stats.files == 0 && !paths.is_empty() {
                stats.files = paths.len();
            }
            out.insert(
                "diff_stats".to_string(),
                json!({
                    "added": stats.added,
                    "removed": stats.removed,
                    "files": stats.files,
                }),
            );
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

fn tool_input_ops_preview(input: Option<&Value>) -> Option<Value> {
    let mut out = serde_json::Map::new();
    let obj = input.and_then(|value| value.as_object());
    if let Some(obj) = obj {
        for key in [
            "cwd",
            "root",
            "path",
            "file",
            "filename",
            "file_path",
            "filePath",
            "filepath",
            "target",
            "glob",
            "paths",
            "files",
            "file_paths",
            "filePaths",
        ] {
            if let Some(value) = obj.get(key) {
                if value.is_string() || value.is_array() || value.is_object() {
                    out.insert(key.to_string(), value.clone());
                }
            }
        }
    }

    let mut paths = Vec::new();
    if !out.is_empty() {
        collect_paths_from_value(&Value::Object(out.clone()), &mut paths);
    }
    if let Some(input) = input {
        if let Some(changes) = input.get("changes") {
            collect_paths_from_changes(changes, &mut paths);
        }
    }
    dedupe_paths(&mut paths);
    if !paths.is_empty() {
        out.insert(
            "paths".to_string(),
            Value::Array(paths.into_iter().map(Value::String).collect()),
        );
    }

    if out.is_empty() {
        None
    } else {
        let value = Value::Object(out);
        let mut truncated = false;
        Some(truncate_preview_value(&value, &mut truncated))
    }
}

fn truncate_preview_value(value: &Value, truncated: &mut bool) -> Value {
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

fn build_text_preview(text: &str) -> ToolTextPreview {
    let original_bytes = text.len();
    let mut truncated = false;
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    let mut out = Vec::new();

    if total_lines <= TOOL_PREVIEW_MAX_LINES {
        for line in lines {
            push_preview_line(&mut out, &mut truncated, line);
        }
    } else {
        truncated = true;
        let head_count = TOOL_PREVIEW_MAX_LINES / 2;
        let tail_count = TOOL_PREVIEW_MAX_LINES.saturating_sub(head_count + 1);
        for line in lines.iter().take(head_count) {
            push_preview_line(&mut out, &mut truncated, line);
        }
        let omitted = total_lines.saturating_sub(head_count + tail_count);
        out.push(format!("... +{omitted} lines"));
        for line in lines.iter().skip(total_lines - tail_count) {
            push_preview_line(&mut out, &mut truncated, line);
        }
    }

    ToolTextPreview {
        preview: out.join("\n"),
        truncated,
        original_bytes,
    }
}

fn build_json_preview(input: Option<&Value>, preview: Option<Value>) -> ToolJsonPreview {
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

fn build_output_preview(text: &str) -> ToolTextPreview {
    build_text_preview(text)
}

fn extract_patch_text_owned(input: Option<&Value>, update: &Value) -> Option<String> {
    if let Some(input) = input {
        if let Some(patch) = extract_patch_text(input) {
            return Some(patch.to_string());
        }
        if let Some(changes) = input.get("changes") {
            if let Some(patch) = extract_patch_text_from_changes(changes) {
                return Some(patch);
            }
        }
    }
    for block in extract_content_blocks(update) {
        let candidate = block.get("content").unwrap_or(block);
        if let Some(patch) = extract_patch_text(candidate) {
            return Some(patch.to_string());
        }
    }
    None
}

pub(super) fn sanitize_tool_event_payload(
    event_type: &SessionEventType,
    raw_payload: &Value,
    output_spool_path: Option<&str>,
) -> Value {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload).unwrap_or_default();

    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

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
    let input_preview = tool_input_ops_preview(input);
    let input_meta = build_json_preview(input, input_preview);

    let patch_preview = if is_edit_tool(tool_kind.as_deref(), title.as_deref()) {
        extract_patch_text_owned(input, update).map(|t| build_output_preview(&t))
    } else {
        None
    };

    let output_preview = extract_tool_output_text(update)
        .map(|t| build_output_preview(&t))
        .or(patch_preview)
        .filter(|preview| !preview.preview.trim().is_empty());

    let mut obj = serde_json::Map::new();
    if !tool_call_id.trim().is_empty() {
        obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
    }
    if let Some(v) = tool_kind {
        obj.insert("kind".to_string(), Value::String(v));
    }
    if let Some(v) = title {
        obj.insert("title".to_string(), Value::String(v));
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
    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
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
    let input_preview = tool_input_preview(input, update, tool_kind.as_deref(), title.as_deref());
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
    tool_kind: Option<String>,
    title: Option<String>,
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
    let tool_kind = update
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/kind").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let title = update
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| update.pointer("/toolCall/title").and_then(|v| v.as_str()))
        .or_else(|| update.pointer("/toolCall/name").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
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
    let input_json = tool_input_preview(input, update, tool_kind.as_deref(), title.as_deref());
    let input_meta = build_json_preview(input, input_json);

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
        tool_kind,
        title,
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
) -> SessionTurnTool {
    let created_at = prev.map(|t| t.created_at).unwrap_or(now);
    let first_event_seq = match prev.and_then(|t| t.first_event_seq) {
        Some(prev_seq) => Some(prev_seq.min(event_seq)),
        None => Some(event_seq),
    };
    let tool_kind = update
        .tool_kind
        .or_else(|| prev.and_then(|t| t.tool_kind.clone()));
    let title = update.title.or_else(|| prev.and_then(|t| t.title.clone()));
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
    SessionTurnTool {
        session_id,
        tool_call_id: update.tool_call_id,
        turn_id,
        tool_kind,
        title,
        status,
        input_json,
        output_text,
        first_event_seq,
        input_truncated,
        input_original_bytes,
        output_truncated,
        output_original_bytes,
        created_at,
        updated_at: now,
    }
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
    payload.get("acp_update").unwrap_or(payload)
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
        build_text_preview, sanitize_tool_event_payload, TOOL_PREVIEW_MAX_LINES,
        TOOL_PREVIEW_MAX_LINE_CHARS,
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
}
