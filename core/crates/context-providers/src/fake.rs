use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use context_core::models::SessionEventType;

use crate::adapters::{ProviderAdapter, ProviderStatus, RunHandle, TurnInput};
use crate::events::NormalizedEvent;

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
            let tool_call_id = Uuid::new_v4().to_string();
            let sink = event_sink;

            let send = |event_type, payload| async {
                let _ = sink
                    .send(NormalizedEvent {
                        event_type,
                        payload_json: payload,
                    })
                    .await;
            };

            tokio::select! {
                _ = async {
                    send(SessionEventType::AssistantChunk, json!({"content": format!("echo: {}", input.content)})).await;
                    sleep(Duration::from_millis(10)).await;
                    send(SessionEventType::ToolCall, json!({"tool_call_id": tool_call_id, "name": "fake_tool", "args": {}})).await;
                    sleep(Duration::from_millis(10)).await;
                    send(SessionEventType::ToolResult, json!({"tool_call_id": tool_call_id, "result": "ok"})).await;
                    sleep(Duration::from_millis(10)).await;
                    send(SessionEventType::AssistantComplete, json!({"content": "done"})).await;
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
