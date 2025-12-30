use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::llm::{
    ChatCompletionRequest, ChatMessage, JsonSchemaSpec, OpenAiClient, ResponseFormat,
};
use crate::settings::TitleGenerationSettings;

pub const DEFAULT_SESSION_TITLE: &str = "New Task";
pub const TITLE_MAX_CHARS: usize = 60;

pub fn is_configured(cfg: &TitleGenerationSettings) -> bool {
    !cfg.base_url.trim().is_empty()
        && !cfg.api_key.trim().is_empty()
        && !cfg.model.trim().is_empty()
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(input: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    input.chars().take(max_len).collect()
}

fn strip_wrapping_quotes(input: &str) -> &str {
    input
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
}

fn strip_trailing_punct(input: &str) -> &str {
    const TRAILING_PUNCT: [char; 6] = ['.', ':', ';', '-', '\u{2013}', '\u{2014}'];
    input.trim_end_matches(|c: char| TRAILING_PUNCT.contains(&c))
}

pub fn normalize_title(raw: &str) -> String {
    let collapsed = collapse_whitespace(raw);
    let unquoted = strip_wrapping_quotes(&collapsed);
    let unpunct = strip_trailing_punct(unquoted).trim();
    truncate_chars(unpunct, TITLE_MAX_CHARS)
}

pub fn fallback_title_from_prompt(prompt: &str) -> String {
    let collapsed = collapse_whitespace(prompt);
    normalize_title(&collapsed)
}

fn structured_response_format() -> ResponseFormat {
    ResponseFormat::JsonSchema {
        json_schema: JsonSchemaSpec {
            name: "session_title".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
            strict: true,
        },
    }
}

fn build_prompt_messages(prompt: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You generate short, information-dense session titles. Requirements: usually <= 3 words, <= {} characters, no quotes, no trailing punctuation.",
                TITLE_MAX_CHARS
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: format!(
                "Create a title for this session based on the first user message:\n\n{}",
                prompt.trim()
            ),
        },
    ]
}

pub async fn generate_title(cfg: &TitleGenerationSettings, prompt: &str) -> Result<String> {
    let client = OpenAiClient::new(
        cfg.base_url.trim().to_string(),
        cfg.api_key.trim().to_string(),
    );
    let req = ChatCompletionRequest {
        model: cfg.model.trim().to_string(),
        messages: build_prompt_messages(prompt),
        temperature: Some(0.2),
        max_tokens: Some(32),
        response_format: cfg.use_json.then(structured_response_format),
    };

    let resp = client
        .chat_completion(&req)
        .await
        .context("chat completion request failed")?;

    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .trim();

    if content.is_empty() {
        return Err(anyhow!("empty completion response"));
    }

    let raw_title = if cfg.use_json {
        let value: serde_json::Value =
            serde_json::from_str(content).context("decoding json response")?;
        value
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing title field in json response"))?
            .to_string()
    } else {
        content.to_string()
    };

    let normalized = normalize_title(&raw_title);
    if normalized.is_empty() {
        return Err(anyhow!("normalized title is empty"));
    }
    Ok(normalized)
}
