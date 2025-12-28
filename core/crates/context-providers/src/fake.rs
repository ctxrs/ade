use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use context_core::models::SessionEventType;

use crate::adapters::{ProviderAdapter, ProviderStatus, RunHandle, TurnInput};
use crate::events::NormalizedEvent;

#[derive(Debug, Clone)]
struct FixtureToolCall {
    kind: String,
    title: Option<String>,
    input: Option<Value>,
    output_text: Option<String>,
}

fn parse_fixture_tools(content: &str) -> Option<Vec<FixtureToolCall>> {
    let start = content.find("[[tool_calls]]")?;
    let end = content.find("[[/tool_calls]]")?;
    if end <= start {
        return None;
    }
    let raw = &content[start + "[[tool_calls]]".len()..end];
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let tools_value = if parsed.is_array() {
        parsed
    } else {
        parsed.get("tool_calls")?.clone()
    };
    let list = tools_value.as_array()?.to_vec();
    let mut out = Vec::new();
    for tool in list {
        let kind = tool
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("execute");
        let title = tool
            .get("title")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let input = tool.get("input").cloned();
        let output_text = tool
            .get("output_text")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        out.push(FixtureToolCall {
            kind: kind.to_string(),
            title,
            input,
            output_text,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Default)]
pub struct FakeProviderAdapter;

impl FakeProviderAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProviderAdapter for FakeProviderAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "fake".into(),
            installed: true,
            detected_path: None,
            version: Some("0.1.0".into()),
            capabilities: Some(crate::adapters::ProviderCapabilities {
                stream_events: true,
                stream_format: "jsonl".into(),
                has_turn_boundaries: true,
                has_tool_call_ids: true,
                has_file_change_events: false,
                has_command_events: false,
                supports_resume: false,
                supports_stable_session_id: false,
                supports_fork_or_rewind: false,
                supports_headless: true,
                supports_server_mode: false,
                supports_acp: false,
                supports_interactive_tui: false,
                supports_private_state_dir: false,
                supports_sandbox_flags: false,
                supports_approval_flags: false,
                notes: vec!["Deterministic fake provider for CI".into()],
            }),
            health: crate::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        let join = tokio::spawn(async move {
            let sink = event_sink;
            let fixture_tools = parse_fixture_tools(&input.content);

            let send = |event_type, payload| async {
                let _ = sink
                    .send(NormalizedEvent {
                        event_type,
                        payload_json: payload,
                    })
                    .await;
            };

            let slow = input.content.contains("slow-diff-test");
            let delay = if slow {
                Duration::from_millis(1200)
            } else {
                Duration::from_millis(10)
            };

            tokio::select! {
                _ = async {
                    send(SessionEventType::AssistantChunk, json!({"content": format!("echo: {}", input.content)})).await;
                    sleep(delay).await;
                    if let Some(tools) = fixture_tools {
                        for tool in tools {
                            let tool_call_id = Uuid::new_v4().to_string();
                            send(SessionEventType::ToolCall, json!({
                                "tool_call_id": tool_call_id,
                                "kind": tool.kind,
                                "title": tool.title,
                                "rawInput": tool.input,
                            })).await;
                            sleep(delay).await;
                            send(SessionEventType::ToolResult, json!({
                                "tool_call_id": tool_call_id,
                                "kind": tool.kind,
                                "title": tool.title,
                                "outputText": tool.output_text.unwrap_or_else(|| "ok".to_string()),
                            })).await;
                            sleep(delay).await;
                        }
                    } else {
                        let tool_call_id = Uuid::new_v4().to_string();
                        send(SessionEventType::ToolCall, json!({"tool_call_id": tool_call_id, "name": "fake_tool", "args": {}})).await;
                        sleep(delay).await;
                        send(SessionEventType::ToolResult, json!({"tool_call_id": tool_call_id, "result": "ok"})).await;
                        sleep(delay).await;
                    }
                    send(SessionEventType::AssistantComplete, json!({"content": format!("done: {}", input.content)})).await;
                    sleep(delay).await;
                    send(SessionEventType::Done, json!({})).await;
                } => {}
                _ = &mut cancel_rx => {
                    send(SessionEventType::Error, json!({"message":"cancelled"})).await;
                }
            }
        });

        Ok(RunHandle {
            join,
            cancel: Some(cancel_tx),
        })
    }

    async fn cancel(&self, mut handle: RunHandle) -> Result<()> {
        if let Some(cancel) = handle.cancel.take() {
            let _ = cancel.send(());
        }
        handle.join.abort();
        Ok(())
    }
}
