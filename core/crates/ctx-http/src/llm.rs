use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(base_url: String, api_key: String) -> Result<Self> {
        Self::new_with_timeout(base_url, api_key, Duration::from_secs(30))
    }

    pub fn new_with_timeout(base_url: String, api_key: String, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            base_url,
            api_key,
            client,
        })
    }

    pub async fn chat_completion(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let api_key = self.api_key.trim();
        if !api_key.is_empty() {
            let auth = format!("Bearer {api_key}");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth).context("invalid authorization header")?,
            );
        }

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/chat/completions");

        let resp = self
            .client
            .post(url)
            .headers(headers)
            .json(req)
            .send()
            .await
            .context("sending chat completion request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "chat completion failed: status={status} body={body}"
            ));
        }

        let payload = resp
            .json::<ChatCompletionResponse>()
            .await
            .context("decoding chat completion response")?;
        Ok(payload)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiResponsesClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiResponsesClient {
    pub fn new(base_url: String, api_key: String, timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            base_url,
            api_key,
            client,
        })
    }

    pub async fn create_response(&self, req: &ResponsesRequest) -> Result<serde_json::Value> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let api_key = self.api_key.trim();
        if !api_key.is_empty() {
            let auth = format!("Bearer {api_key}");
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth).context("invalid authorization header")?,
            );
        }

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/responses");

        let resp = self
            .client
            .post(url)
            .headers(headers)
            .json(req)
            .send()
            .await
            .context("sending responses request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "responses request failed: status={status} body={body}"
            ));
        }

        let payload = resp
            .json::<serde_json::Value>()
            .await
            .context("decoding responses response")?;
        Ok(payload)
    }
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonSchema { json_schema: JsonSchemaSpec },
}

#[derive(Debug, Serialize)]
pub struct JsonSchemaSpec {
    pub name: String,
    pub schema: serde_json::Value,
    pub strict: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionChoice {
    pub message: ChatCompletionMessage,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionMessage {
    pub content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct ResponsesReasoning {
    pub effort: String,
}

pub fn extract_responses_output_text(payload: &serde_json::Value) -> Option<String> {
    if let Some(text) = payload.get("output_text").and_then(|v| v.as_str()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let mut out = String::new();
    let items = payload.get("output").and_then(|v| v.as_array())?;

    for item in items {
        let Some(contents) = item.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for content in contents {
            for key in ["text", "value", "output_text"] {
                if let Some(text) = content.get(key).and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text.trim());
                        break;
                    }
                }
            }
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
