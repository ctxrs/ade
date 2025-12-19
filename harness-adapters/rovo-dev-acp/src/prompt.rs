use serde_json::Value;

pub fn format_prompt_blocks(blocks: &[Value]) -> String {
    let mut parts = Vec::with_capacity(blocks.len());
    for block in blocks {
        let part = match block.get("type").and_then(|v| v.as_str()) {
            Some("text") => block
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            Some("resource") => format_resource_block(block),
            Some("resource_link") => format_resource_link_block(block),
            Some("image") => "[Image content omitted]".to_string(),
            Some("audio") => "[Audio content omitted]".to_string(),
            other => format!("[Unsupported content type: {}]", other.unwrap_or("unknown")),
        };
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts.join("\n\n")
}

fn format_resource_block(block: &Value) -> String {
    let Some(resource) = block.get("resource") else {
        return "[Resource omitted]".to_string();
    };
    let uri = resource.get("uri").and_then(|v| v.as_str()).unwrap_or("unknown");
    if let Some(text) = resource.get("text").and_then(|v| v.as_str()) {
        return format!("[Resource: {}]\n{}\n[/Resource]", uri, text);
    }
    if let Some(blob) = resource.get("blob").and_then(|v| v.as_str()) {
        let mime = resource
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream");
        return format!(
            "[Resource: {}] ({} bytes, {})",
            uri,
            blob.len(),
            mime
        );
    }
    "[Resource omitted]".to_string()
}

fn format_resource_link_block(block: &Value) -> String {
    let uri = block.get("uri").and_then(|v| v.as_str()).unwrap_or("unknown");
    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("resource");
    let title = block.get("title").and_then(|v| v.as_str());
    let description = block.get("description").and_then(|v| v.as_str());
    let mut out = format!("[Resource link: {} ({})]", uri, name);
    if let Some(title) = title {
        out.push_str(&format!("\nTitle: {}", title));
    }
    if let Some(description) = description {
        out.push_str(&format!("\nDescription: {}", description));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn formats_text_blocks() {
        let blocks = vec![serde_json::json!({"type": "text", "text": "Hello"})];
        assert_eq!(format_prompt_blocks(&blocks), "Hello");
    }

    #[test]
    fn formats_resource_blocks() {
        let blocks = vec![serde_json::json!({
            "type": "resource",
            "resource": {
                "uri": "file:///tmp/example.txt",
                "text": "data"
            }
        })];
        assert_eq!(
            format_prompt_blocks(&blocks),
            "[Resource: file:///tmp/example.txt]\ndata\n[/Resource]"
        );
    }
}
