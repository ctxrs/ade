use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::llm::{
    extract_responses_output_text, ChatCompletionRequest, ChatMessage, OpenAiClient,
    OpenAiResponsesClient, ResponsesReasoning, ResponsesRequest,
};
use crate::settings::OracleSettings;

#[derive(Debug, Clone, Deserialize)]
pub struct OracleRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OracleResponse {
    pub model: String,
    pub reasoning_effort: String,
    pub text: String,
}

pub async fn oracle_one_shot(cfg: &OracleSettings, req: OracleRequest) -> Result<OracleResponse> {
    if !cfg.enabled {
        return Err(anyhow!("oracle is disabled"));
    }
    if cfg.api_key.trim().is_empty() {
        return Err(anyhow!("oracle api key is not configured"));
    }
    if cfg.base_url.trim().is_empty() {
        return Err(anyhow!("oracle base_url is empty"));
    }

    let model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(|| cfg.model.clone());
    let reasoning_effort = req
        .reasoning_effort
        .clone()
        .or_else(|| cfg.reasoning_effort.clone())
        .unwrap_or_else(|| "high".to_string());
    let max_output_tokens = req.max_output_tokens.or(cfg.max_output_tokens);

    let timeout_ms = req.timeout_ms.or(cfg.timeout_ms).unwrap_or(10 * 60 * 1000);
    let timeout = Duration::from_millis(timeout_ms);

    let base_url = cfg.base_url.clone();
    let api_key = cfg.api_key.clone();

    // Prefer the Responses API to enable explicit reasoning controls.
    let responses_client = OpenAiResponsesClient::new(base_url.clone(), api_key.clone(), timeout)
        .context("creating responses client")?;
    let responses_req = ResponsesRequest {
        model: model.clone(),
        input: req.prompt.clone(),
        reasoning: Some(ResponsesReasoning {
            effort: reasoning_effort.clone(),
        }),
        max_output_tokens,
        temperature: None,
    };

    match responses_client.create_response(&responses_req).await {
        Ok(payload) => {
            let text = extract_responses_output_text(&payload)
                .or_else(|| {
                    payload
                        .get("output_text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| payload.to_string());
            Ok(OracleResponse {
                model,
                reasoning_effort,
                text,
            })
        }
        Err(err) => {
            // Fallback to chat completions for OpenAI-compatible endpoints that don't support
            // `/responses` yet.
            let fallback_client = OpenAiClient::new_with_timeout(base_url, api_key, timeout)
                .context("creating chat fallback client")?;
            let system = "You are Oracle GPT, a very high-reasoning model. You do not have access to any repository or tools. You must reason only from the text provided. Think through tradeoffs, failure modes, and alternative architectures. Provide an opinionated recommendation and a short validation plan.";
            let chat_req = ChatCompletionRequest {
                model: model.clone(),
                messages: vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: system.to_string(),
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: req.prompt,
                    },
                ],
                temperature: None,
                max_tokens: max_output_tokens,
                response_format: None,
            };
            let chat = fallback_client
                .chat_completion(&chat_req)
                .await
                .with_context(|| {
                    format!("oracle call failed (responses + chat fallback): {err}")
                })?;
            let text = chat
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "".to_string());
            if text.is_empty() {
                return Err(anyhow!("oracle returned empty response"));
            }
            Ok(OracleResponse {
                model,
                reasoning_effort,
                text,
            })
        }
    }
}
