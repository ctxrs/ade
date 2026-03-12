use super::*;

#[derive(Default)]
pub(super) struct DiffStats {
    added: usize,
    removed: usize,
    files: usize,
}

pub(super) const MAX_PREVIEW_PATHS: usize = 5;
pub(super) const TOOL_PREVIEW_MAX_LINES: usize = 5;
pub(super) const TOOL_PREVIEW_MAX_LINE_CHARS: usize = 80;

pub(super) struct ToolTextPreview {
    pub(super) preview: String,
    pub(super) truncated: bool,
    pub(super) original_bytes: usize,
}

pub(super) struct ToolJsonPreview {
    pub(super) preview: Option<Value>,
    pub(super) truncated: Option<bool>,
    pub(super) original_bytes: Option<i64>,
}

pub(super) fn push_preview_line(out: &mut Vec<String>, truncated: &mut bool, line: &str) {
    if line.chars().count() > TOOL_PREVIEW_MAX_LINE_CHARS {
        *truncated = true;
        out.push(line.chars().take(TOOL_PREVIEW_MAX_LINE_CHARS).collect());
    } else {
        out.push(line.to_string());
    }
}

pub(super) fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

pub(super) fn diff_stats_from_patch(patch: &str) -> Option<DiffStats> {
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

pub(super) fn extract_old_new_text(
    obj: &serde_json::Map<String, Value>,
) -> (Option<&str>, Option<&str>) {
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

pub(super) fn diff_stats_from_edits(edits: &[Value]) -> Option<DiffStats> {
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

pub(super) fn diff_stats_from_changes(changes: &Value) -> Option<DiffStats> {
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

pub(super) fn diff_stats_from_content_blocks(blocks: &[&Value]) -> Option<DiffStats> {
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

pub(super) fn extract_patch_text(input: &Value) -> Option<&str> {
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

pub(super) fn extract_patch_text_from_changes(changes: &Value) -> Option<String> {
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

pub(super) fn extract_content_blocks(update: &Value) -> Vec<&Value> {
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

pub(super) fn collect_paths_from_value(value: &Value, paths: &mut Vec<String>) {
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

pub(super) fn collect_paths_from_changes(value: &Value, paths: &mut Vec<String>) {
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

pub(super) fn dedupe_paths(paths: &mut Vec<String>) {
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

pub(super) fn extract_paths_from_update(update: &Value) -> Vec<String> {
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

pub(super) fn extract_tool_input(update: &Value) -> Option<&Value> {
    update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/raw_input"))
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/raw_input"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"))
}

pub(super) fn is_edit_tool(tool_kind: Option<&str>, title: Option<&str>) -> bool {
    let kind = tool_kind.unwrap_or("").trim().to_lowercase();
    if matches!(kind.as_str(), "edit" | "write" | "apply_patch" | "patch") {
        return true;
    }
    let title = title.unwrap_or("").trim().to_lowercase();
    title.contains("edit") || title.contains("patch") || title.contains("apply")
}

pub(super) fn tool_input_preview(
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

pub(super) fn build_text_preview(text: &str) -> ToolTextPreview {
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

pub(super) fn extract_patch_text_owned(input: Option<&Value>, update: &Value) -> Option<String> {
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
