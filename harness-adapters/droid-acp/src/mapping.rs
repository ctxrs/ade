use agent_client_protocol as acp;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn tool_kind_from_name(name: &str) -> acp::ToolKind {
    let lower = name.to_lowercase();
    if lower.contains("read") || lower.contains("ls") || lower.contains("list") {
        return acp::ToolKind::Read;
    }
    if lower.contains("write") || lower.contains("edit") || lower.contains("apply") {
        return acp::ToolKind::Edit;
    }
    if lower.contains("delete") || lower.contains("remove") {
        return acp::ToolKind::Delete;
    }
    if lower.contains("move") || lower.contains("rename") {
        return acp::ToolKind::Move;
    }
    if lower.contains("search") || lower.contains("grep") || lower.contains("find") {
        return acp::ToolKind::Search;
    }
    if lower.contains("execute") || lower.contains("run") || lower.contains("command") {
        return acp::ToolKind::Execute;
    }
    if lower.contains("fetch") || lower.contains("http") || lower.contains("download") {
        return acp::ToolKind::Fetch;
    }

    acp::ToolKind::Other
}

pub fn tool_title(tool_name: Option<&str>, tool_id: Option<&str>) -> String {
    tool_name
        .or(tool_id)
        .unwrap_or("Tool")
        .to_string()
}

pub fn raw_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

pub fn text_content(text: String) -> acp::ToolCallContent {
    acp::ToolCallContent::Content {
        content: acp::ContentBlock::Text(acp::TextContent {
            text,
            annotations: None,
            meta: None,
        }),
    }
}

pub fn resolve_path(path: &str, cwd: Option<&Path>) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else if let Some(root) = cwd {
        root.join(candidate)
    } else {
        candidate
    }
}

pub fn extract_location(parameters: &Value, cwd: Option<&Path>) -> Option<acp::ToolCallLocation> {
    let object = parameters.as_object()?;
    let path_value = object
        .get("path")
        .or_else(|| object.get("file"))
        .or_else(|| object.get("file_path"))
        .and_then(|value| value.as_str())?;

    let line = object
        .get("line")
        .or_else(|| object.get("line_number"))
        .and_then(|value| value.as_u64())
        .map(|value| value as u32);

    Some(acp::ToolCallLocation {
        path: resolve_path(path_value, cwd),
        line,
        meta: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tool_kinds() {
        assert_eq!(tool_kind_from_name("Read"), acp::ToolKind::Read);
        assert_eq!(tool_kind_from_name("ApplyPatch"), acp::ToolKind::Edit);
        assert_eq!(tool_kind_from_name("Execute"), acp::ToolKind::Execute);
        assert_eq!(tool_kind_from_name("Search"), acp::ToolKind::Search);
    }

    #[test]
    fn resolves_relative_paths() {
        let cwd = Path::new("/tmp/project");
        let resolved = resolve_path("src/main.rs", Some(cwd));
        assert_eq!(resolved, PathBuf::from("/tmp/project/src/main.rs"));
    }
}
