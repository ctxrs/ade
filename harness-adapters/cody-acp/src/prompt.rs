use agent_client_protocol::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, ResourceLink, TextContent,
    TextResourceContents,
};
use serde_json::{json, Value};

pub struct PromptInput {
    pub text: String,
    pub context_items: Vec<Value>,
}

pub fn build_prompt(blocks: &[ContentBlock]) -> PromptInput {
    let mut parts = Vec::new();
    let mut context_items = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text(TextContent { text, .. }) => {
                if !text.is_empty() {
                    parts.push(text.clone());
                }
            }
            ContentBlock::ResourceLink(ResourceLink { uri, name, .. }) => {
                parts.push(format_resource_link(uri, name));
                if let Some(item) = context_item_from_uri(uri) {
                    context_items.push(item);
                }
            }
            ContentBlock::Resource(EmbeddedResource {
                resource:
                    EmbeddedResourceResource::TextResourceContents(TextResourceContents {
                        text,
                        uri,
                        ..
                    }),
                ..
            }) => {
                parts.push(format!("[Resource: {}]\n{}\n[/Resource]", uri, text));
            }
            ContentBlock::Image(_) => {
                parts.push("[Image content omitted]".to_string());
            }
            ContentBlock::Audio(_) => {
                parts.push("[Audio content omitted]".to_string());
            }
            ContentBlock::Resource(_) => {
                parts.push("[Resource omitted]".to_string());
            }
            _ => {
                parts.push("[Unsupported content omitted]".to_string());
            }
        }
    }

    PromptInput {
        text: parts.join("\n\n"),
        context_items,
    }
}

fn format_resource_link(uri: &str, name: &str) -> String {
    if name.is_empty() {
        format!("[Resource link: {}]", uri)
    } else {
        format!("[Resource link: {} ({})]", uri, name)
    }
}

fn context_item_from_uri(uri: &str) -> Option<Value> {
    if uri.starts_with("file://") {
        Some(json!({
            "type": "file",
            "uri": uri,
        }))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{
        ContentBlock, EmbeddedResource, EmbeddedResourceResource, ResourceLink, TextContent,
        TextResourceContents,
    };
    use pretty_assertions::assert_eq;

    #[test]
    fn builds_text_prompt() {
        let blocks = vec![ContentBlock::Text(TextContent::new("Hello"))];
        let prompt = build_prompt(&blocks);
        assert_eq!(prompt.text, "Hello");
        assert!(prompt.context_items.is_empty());
    }

    #[test]
    fn builds_resource_link_prompt() {
        let blocks = vec![ContentBlock::ResourceLink(ResourceLink::new(
            "example.txt".to_string(),
            "file:///tmp/example.txt".to_string(),
        ))];
        let prompt = build_prompt(&blocks);
        assert_eq!(
            prompt.text,
            "[Resource link: file:///tmp/example.txt (example.txt)]"
        );
        assert_eq!(prompt.context_items.len(), 1);
    }

    #[test]
    fn builds_embedded_resource_prompt() {
        let resource = EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(
                "contents".to_string(),
                "file:///tmp/resource.txt".to_string(),
            )),
        );
        let blocks = vec![ContentBlock::Resource(resource)];
        let prompt = build_prompt(&blocks);
        assert_eq!(
            prompt.text,
            "[Resource: file:///tmp/resource.txt]\ncontents\n[/Resource]"
        );
    }
}
