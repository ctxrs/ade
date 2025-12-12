use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use context_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

pub struct HeadlessCliAdapter {
    pub id: String,
    pub binary: String,
    pub args_prefix: Vec<String>,
}

impl HeadlessCliAdapter {
    pub fn codex() -> Self {
        Self {
            id: "codex".into(),
            binary: "codex".into(),
            args_prefix: vec!["exec".into(), "--json".into()],
        }
    }

    pub fn claude() -> Self {
        Self {
            id: "claude".into(),
            binary: "claude".into(),
            args_prefix: vec!["-p".into()],
        }
    }

    pub fn gemini() -> Self {
        Self {
            id: "gemini".into(),
            binary: "gemini".into(),
            args_prefix: vec!["-p".into()],
        }
    }
}

#[async_trait]
impl ProviderAdapter for HeadlessCliAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let detected_path = which::which(&self.binary).ok();
        let mut diagnostics = Vec::new();

        let output = Command::new(detected_path.as_deref().unwrap_or_else(|| self.binary.as_ref()))
            .arg("--version")
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                let mut version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if version.is_empty() {
                    version = String::from_utf8_lossy(&out.stderr).trim().to_string();
                }
                Ok(ProviderStatus {
                    provider_id: self.id.clone(),
                    installed: true,
                    detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
                    version: if version.is_empty() { None } else { Some(version) },
                    capabilities: Some(default_caps(&self.id)),
                    health: ProviderHealth::Ok,
                    diagnostics,
                    details: HashMap::new(),
                })
            }
            Ok(out) => {
                diagnostics.push(String::from_utf8_lossy(&out.stderr).trim().to_string());
                Ok(ProviderStatus {
                    provider_id: self.id.clone(),
                    installed: false,
                    detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
                    version: None,
                    capabilities: None,
                    health: ProviderHealth::Missing,
                    diagnostics,
                    details: HashMap::new(),
                })
            }
            Err(e) => {
                diagnostics.push(e.to_string());
                Ok(ProviderStatus {
                    provider_id: self.id.clone(),
                    installed: false,
                    detected_path: None,
                    version: None,
                    capabilities: None,
                    health: ProviderHealth::Error,
                    diagnostics,
                    details: HashMap::new(),
                })
            }
        }
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        let mut cmd = Command::new(&self.binary);
        cmd.current_dir(&workdir);
        cmd.args(&self.args_prefix);
        cmd.arg(&input.content);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().with_context(|| {
            format!("spawning provider binary {} in {}", self.binary, workdir.display())
        })?;

        let stdout = child
            .stdout
            .take()
            .context("capturing provider stdout")?;
        let mut reader = BufReader::new(stdout).lines();

        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        let sink = event_sink;
        let id = self.id.clone();

        let join = tokio::spawn(async move {
            let mut full = String::new();
            loop {
                tokio::select! {
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&l) {
                                    let events = normalize_json_line(val, &mut full);
                                    for ev in events {
                                        let _ = sink.send(ev).await;
                                    }
                                } else {
                                    full.push_str(&l);
                                    full.push('\n');
                                    let _ = sink.send(NormalizedEvent {
                                        event_type: SessionEventType::AssistantChunk,
                                        payload_json: json!({"content_fragment": l}),
                                    }).await;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                let _ = sink.send(NormalizedEvent {
                                    event_type: SessionEventType::Error,
                                    payload_json: json!({"provider": id, "message": e.to_string()}),
                                }).await;
                                break;
                            }
                        }
                    }
                    _ = &mut cancel_rx => {
                        let _ = child.kill().await;
                        let _ = sink.send(NormalizedEvent {
                            event_type: SessionEventType::InterruptRequested,
                            payload_json: json!({"provider": id}),
                        }).await;
                        return;
                    }
                }
            }

            let _ = child.wait().await;
            if !full.is_empty() {
                let _ = sink.send(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": full}),
                }).await;
            }
            let _ = sink.send(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: json!({}),
            }).await;
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

fn default_caps(id: &str) -> ProviderCapabilities {
    match id {
        "codex" => ProviderCapabilities {
            stream_events: true,
            stream_format: "jsonl".into(),
            has_turn_boundaries: true,
            has_tool_call_ids: true,
            has_file_change_events: true,
            has_command_events: true,
            supports_resume: true,
            supports_stable_session_id: true,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: false,
            supports_acp: false,
            supports_interactive_tui: true,
            supports_private_state_dir: false,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: vec![],
        },
        "claude" => ProviderCapabilities {
            stream_events: true,
            stream_format: "stream-json".into(),
            has_turn_boundaries: false,
            has_tool_call_ids: true,
            has_file_change_events: false,
            has_command_events: false,
            supports_resume: true,
            supports_stable_session_id: true,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: false,
            supports_acp: false,
            supports_interactive_tui: true,
            supports_private_state_dir: true,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: vec!["Uses CLAUDE_CONFIG_DIR for isolation".into()],
        },
        "gemini" => ProviderCapabilities {
            stream_events: true,
            stream_format: "stream-json".into(),
            has_turn_boundaries: false,
            has_tool_call_ids: true,
            has_file_change_events: false,
            has_command_events: true,
            supports_resume: false,
            supports_stable_session_id: false,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: false,
            supports_acp: false,
            supports_interactive_tui: true,
            supports_private_state_dir: false,
            supports_sandbox_flags: true,
            supports_approval_flags: true,
            notes: vec![],
        },
        _ => ProviderCapabilities {
            stream_events: false,
            stream_format: "none".into(),
            has_turn_boundaries: false,
            has_tool_call_ids: false,
            has_file_change_events: false,
            has_command_events: false,
            supports_resume: false,
            supports_stable_session_id: false,
            supports_fork_or_rewind: false,
            supports_headless: false,
            supports_server_mode: false,
            supports_acp: false,
            supports_interactive_tui: false,
            supports_private_state_dir: false,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: vec![],
        },
    }
}

fn normalize_json_line(val: serde_json::Value, full: &mut String) -> Vec<NormalizedEvent> {
    let type_str = val
        .get("type")
        .or_else(|| val.get("event_type"))
        .and_then(|v| v.as_str());

    match type_str {
        Some(t) if matches!(t, "assistant_chunk" | "assistant_delta" | "assistant_message_delta") => {
            let frag = val
                .get("content_fragment")
                .or_else(|| val.get("delta"))
                .or_else(|| val.get("content"))
                .or_else(|| val.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            full.push_str(frag);
            vec![NormalizedEvent {
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({"content_fragment": frag}),
            }]
        }
        Some(t) if matches!(t, "assistant_complete" | "assistant_message" | "assistant") => {
            let content = val
                .get("full_content")
                .or_else(|| val.get("content"))
                .or_else(|| val.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !content.is_empty() {
                full.clear();
                full.push_str(&content);
            }
            vec![NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({"full_content": content}),
            }]
        }
        Some(t) if matches!(t, "tool_call" | "tool_use") => {
            let mut payload = val;
            if payload.get("tool_call_id").is_none() {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("tool_call_id".into(), json!(Uuid::new_v4().to_string()));
                }
            }
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolCall,
                payload_json: payload,
            }]
        }
        Some(t) if matches!(t, "tool_result" | "tool_response") => {
            let mut payload = val;
            if payload.get("tool_call_id").is_none() {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("tool_call_id".into(), json!(Uuid::new_v4().to_string()));
                }
            }
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolResult,
                payload_json: payload,
            }]
        }
        Some(t) if matches!(t, "done" | "turn_complete" | "complete") => vec![NormalizedEvent {
            event_type: SessionEventType::Done,
            payload_json: val,
        }],
        Some(t) if t == "error" => vec![NormalizedEvent {
            event_type: SessionEventType::Error,
            payload_json: val,
        }],
        _ => {
            if let Some(content) = val
                .get("content")
                .or_else(|| val.get("text"))
                .and_then(|v| v.as_str())
            {
                full.push_str(content);
                vec![NormalizedEvent {
                    event_type: SessionEventType::AssistantChunk,
                    payload_json: json!({"content_fragment": content}),
                }]
            } else {
                vec![NormalizedEvent {
                    event_type: SessionEventType::AssistantChunk,
                    payload_json: json!({"content_fragment": val.to_string()}),
                }]
            }
        }
    }
}
