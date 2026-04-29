use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenAiResponsesRequestV1Envelope {
    pub model: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesContentPart {
    InputText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResponsesMessageContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponsesMessage {
    pub role: ResponsesRole,
    pub content: ResponsesMessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResponsesInput {
    Text(String),
    Messages(Vec<ResponsesMessage>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidatedFunctionTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidatedOpenAiResponsesRequestV1 {
    pub model: String,
    pub input: ResponsesInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub max_output_tokens: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ValidatedFunctionTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl ValidatedOpenAiResponsesRequestV1 {
    pub fn estimated_input_bytes(&self) -> Result<u64, serde_json::Error> {
        let mut total = 0_u64;

        if let Some(instructions) = self.instructions.as_ref() {
            total = total.saturating_add(instructions.len() as u64);
        }
        total = total.saturating_add(match &self.input {
            ResponsesInput::Text(text) => text.len() as u64,
            ResponsesInput::Messages(messages) => messages
                .iter()
                .map(|message| match &message.content {
                    ResponsesMessageContent::Text(text) => text.len() as u64,
                    ResponsesMessageContent::Parts(parts) => parts
                        .iter()
                        .map(|part| match part {
                            ResponsesContentPart::InputText { text } => text.len() as u64,
                        })
                        .sum(),
                })
                .sum(),
        });

        for tool in &self.tools {
            total = total.saturating_add(tool.name.len() as u64);
            if let Some(description) = tool.description.as_ref() {
                total = total.saturating_add(description.len() as u64);
            }
            total = total.saturating_add(serde_json::to_vec(&tool.parameters)?.len() as u64);
        }

        Ok(total)
    }
}

#[derive(Debug, Error)]
pub enum OpenAiResponsesRequestValidationError {
    #[error("invalid top-level responses request shape: {0}")]
    InvalidTopLevelShape(String),
    #[error("max_output_tokens is required")]
    MissingMaxOutputTokens,
    #[error("max_output_tokens must be greater than zero")]
    InvalidMaxOutputTokens,
    #[error("background mode is not supported for relay v1")]
    BackgroundModeUnsupported,
    #[error("provider-side storage is not supported for relay v1")]
    ProviderStoreUnsupported,
    #[error("field {field} has invalid type, expected {expected}")]
    InvalidFieldType {
        field: String,
        expected: &'static str,
    },
    #[error("unsupported field {field} in {context}")]
    UnsupportedField { context: String, field: String },
    #[error("unsupported content type {content_type} in {field}")]
    UnsupportedContentType { field: String, content_type: String },
    #[error("unsupported tool type {tool_type}")]
    UnsupportedToolType { tool_type: String },
}

pub fn validate_openai_responses_request_v1(
    request: Value,
) -> Result<ValidatedOpenAiResponsesRequestV1, OpenAiResponsesRequestValidationError> {
    let raw: OpenAiResponsesRequestV1Envelope = serde_json::from_value(request).map_err(|err| {
        OpenAiResponsesRequestValidationError::InvalidTopLevelShape(err.to_string())
    })?;
    raw.validate()
}

impl OpenAiResponsesRequestV1Envelope {
    pub fn validate(
        self,
    ) -> Result<ValidatedOpenAiResponsesRequestV1, OpenAiResponsesRequestValidationError> {
        let max_output_tokens = self
            .max_output_tokens
            .ok_or(OpenAiResponsesRequestValidationError::MissingMaxOutputTokens)?;
        if max_output_tokens == 0 {
            return Err(OpenAiResponsesRequestValidationError::InvalidMaxOutputTokens);
        }
        if self.background.unwrap_or(false) {
            return Err(OpenAiResponsesRequestValidationError::BackgroundModeUnsupported);
        }
        if self.store.unwrap_or(false) {
            return Err(OpenAiResponsesRequestValidationError::ProviderStoreUnsupported);
        }

        Ok(ValidatedOpenAiResponsesRequestV1 {
            model: self.model,
            input: validate_input(self.input)?,
            instructions: self.instructions,
            max_output_tokens,
            tools: validate_tools(self.tools)?,
            stream: self.stream,
            temperature: self.temperature,
        })
    }
}

fn validate_input(input: Value) -> Result<ResponsesInput, OpenAiResponsesRequestValidationError> {
    match input {
        Value::String(text) => Ok(ResponsesInput::Text(text)),
        Value::Array(messages) => {
            let mut validated = Vec::with_capacity(messages.len());
            for (index, message) in messages.into_iter().enumerate() {
                validated.push(validate_message(message, format!("input[{index}]"))?);
            }
            Ok(ResponsesInput::Messages(validated))
        }
        _ => Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: "input".to_string(),
            expected: "string or message array",
        }),
    }
}

fn validate_message(
    value: Value,
    field: String,
) -> Result<ResponsesMessage, OpenAiResponsesRequestValidationError> {
    let object = value.as_object().ok_or_else(|| {
        OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: field.clone(),
            expected: "object",
        }
    })?;
    reject_unknown_keys(object, &["role", "content"], field.clone())?;

    let role_value = object.get("role").ok_or_else(|| {
        OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: format!("{field}.role"),
            expected: "role string",
        }
    })?;
    let role = parse_role(role_value, format!("{field}.role"))?;
    let content_value = object.get("content").ok_or_else(|| {
        OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: format!("{field}.content"),
            expected: "string or content array",
        }
    })?;

    Ok(ResponsesMessage {
        role,
        content: validate_message_content(content_value.clone(), format!("{field}.content"))?,
    })
}

fn validate_message_content(
    value: Value,
    field: String,
) -> Result<ResponsesMessageContent, OpenAiResponsesRequestValidationError> {
    match value {
        Value::String(text) => Ok(ResponsesMessageContent::Text(text)),
        Value::Array(parts) => {
            let mut validated = Vec::with_capacity(parts.len());
            for (index, part) in parts.into_iter().enumerate() {
                validated.push(validate_content_part(part, format!("{field}[{index}]"))?);
            }
            Ok(ResponsesMessageContent::Parts(validated))
        }
        _ => Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
            field,
            expected: "string or content array",
        }),
    }
}

fn validate_content_part(
    value: Value,
    field: String,
) -> Result<ResponsesContentPart, OpenAiResponsesRequestValidationError> {
    let object = value.as_object().ok_or_else(|| {
        OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: field.clone(),
            expected: "object",
        }
    })?;
    let content_type = required_string(object, "type", field.clone())?;

    match content_type.as_str() {
        "input_text" => {
            reject_unknown_keys(object, &["type", "text"], field.clone())?;
            Ok(ResponsesContentPart::InputText {
                text: required_string(object, "text", field)?,
            })
        }
        "input_file" => Err(
            OpenAiResponsesRequestValidationError::UnsupportedContentType {
                field,
                content_type,
            },
        ),
        "input_image" => Err(
            OpenAiResponsesRequestValidationError::UnsupportedContentType {
                field,
                content_type,
            },
        ),
        "input_audio" => Err(
            OpenAiResponsesRequestValidationError::UnsupportedContentType {
                field,
                content_type,
            },
        ),
        "input_video" => Err(
            OpenAiResponsesRequestValidationError::UnsupportedContentType {
                field,
                content_type,
            },
        ),
        _ => Err(
            OpenAiResponsesRequestValidationError::UnsupportedContentType {
                field,
                content_type,
            },
        ),
    }
}

fn validate_tools(
    tools: Vec<Value>,
) -> Result<Vec<ValidatedFunctionTool>, OpenAiResponsesRequestValidationError> {
    let mut validated = Vec::with_capacity(tools.len());

    for (index, tool) in tools.into_iter().enumerate() {
        let field = format!("tools[{index}]");
        let object = tool.as_object().ok_or_else(|| {
            OpenAiResponsesRequestValidationError::InvalidFieldType {
                field: field.clone(),
                expected: "object",
            }
        })?;
        let tool_type = required_string(object, "type", field.clone())?;

        match tool_type.as_str() {
            "function" => {
                reject_unknown_keys(
                    object,
                    &["type", "name", "description", "parameters", "strict"],
                    field.clone(),
                )?;
                let parameters = object.get("parameters").ok_or_else(|| {
                    OpenAiResponsesRequestValidationError::InvalidFieldType {
                        field: format!("{field}.parameters"),
                        expected: "object",
                    }
                })?;
                if !parameters.is_object() {
                    return Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
                        field: format!("{field}.parameters"),
                        expected: "object",
                    });
                }
                let description = optional_string(object, "description", field.clone())?;
                let strict = optional_bool(object, "strict", field.clone())?;
                validated.push(ValidatedFunctionTool {
                    name: required_string(object, "name", field.clone())?,
                    description,
                    parameters: parameters.clone(),
                    strict,
                });
            }
            "web_search"
            | "web_search_preview"
            | "code_interpreter"
            | "computer_use"
            | "computer_use_preview"
            | "file_search"
            | "image_generation" => {
                return Err(OpenAiResponsesRequestValidationError::UnsupportedToolType {
                    tool_type,
                });
            }
            _ => {
                return Err(OpenAiResponsesRequestValidationError::UnsupportedToolType {
                    tool_type,
                });
            }
        }
    }

    Ok(validated)
}

fn parse_role(
    value: &Value,
    field: String,
) -> Result<ResponsesRole, OpenAiResponsesRequestValidationError> {
    let value =
        value
            .as_str()
            .ok_or_else(|| OpenAiResponsesRequestValidationError::InvalidFieldType {
                field: field.clone(),
                expected: "string",
            })?;

    match value {
        "system" => Ok(ResponsesRole::System),
        "developer" => Ok(ResponsesRole::Developer),
        "user" => Ok(ResponsesRole::User),
        "assistant" => Ok(ResponsesRole::Assistant),
        _ => Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
            field,
            expected: "system, developer, user, or assistant",
        }),
    }
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
    field: String,
) -> Result<String, OpenAiResponsesRequestValidationError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: format!("{field}.{key}"),
            expected: "string",
        })
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    field: String,
) -> Result<Option<String>, OpenAiResponsesRequestValidationError> {
    match object.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: format!("{field}.{key}"),
            expected: "string",
        }),
        None => Ok(None),
    }
}

fn optional_bool(
    object: &Map<String, Value>,
    key: &str,
    field: String,
) -> Result<Option<bool>, OpenAiResponsesRequestValidationError> {
    match object.get(key) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(OpenAiResponsesRequestValidationError::InvalidFieldType {
            field: format!("{field}.{key}"),
            expected: "boolean",
        }),
        None => Ok(None),
    }
}

fn reject_unknown_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    context: String,
) -> Result<(), OpenAiResponsesRequestValidationError> {
    for key in object.keys() {
        if !allowed.iter().any(|allowed_key| allowed_key == key) {
            return Err(OpenAiResponsesRequestValidationError::UnsupportedField {
                context,
                field: key.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        validate_openai_responses_request_v1, OpenAiResponsesRequestValidationError, ResponsesInput,
    };

    #[test]
    fn request_shape_validation_accepts_text_messages_and_function_tools() {
        let request = json!({
            "model": "gpt-5",
            "input": [
                {
                    "role": "developer",
                    "content": "Keep the answer concise."
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "Summarize this crate."
                        }
                    ]
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "emit_json",
                    "description": "Return structured output.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "summary": { "type": "string" }
                        },
                        "required": ["summary"]
                    },
                    "strict": true
                }
            ],
            "max_output_tokens": 512,
            "stream": true
        });

        let validated = validate_openai_responses_request_v1(request).unwrap();
        assert!(matches!(validated.input, ResponsesInput::Messages(_)));
        assert_eq!(validated.tools.len(), 1);
        assert_eq!(validated.max_output_tokens, 512);
        assert!(validated.estimated_input_bytes().unwrap() > 0);
    }

    #[test]
    fn request_shape_validation_rejects_provider_hosted_tools() {
        let request = json!({
            "model": "gpt-5",
            "input": "hello",
            "tools": [
                {
                    "type": "web_search_preview"
                }
            ],
            "max_output_tokens": 128
        });

        assert!(matches!(
            validate_openai_responses_request_v1(request),
            Err(OpenAiResponsesRequestValidationError::UnsupportedToolType { .. })
        ));
    }

    #[test]
    fn request_shape_validation_rejects_non_text_media_inputs() {
        let request = json!({
            "model": "gpt-5",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "image_url": "https://example.com/image.png"
                        }
                    ]
                }
            ],
            "max_output_tokens": 128
        });

        assert!(matches!(
            validate_openai_responses_request_v1(request),
            Err(OpenAiResponsesRequestValidationError::UnsupportedContentType { .. })
        ));
    }

    #[test]
    fn request_shape_validation_requires_max_output_tokens() {
        let request = json!({
            "model": "gpt-5",
            "input": "hello"
        });

        assert!(matches!(
            validate_openai_responses_request_v1(request),
            Err(OpenAiResponsesRequestValidationError::MissingMaxOutputTokens)
        ));
    }
}
