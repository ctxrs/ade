use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use context_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier1CliFlavor {
    Codex,
    Claude,
    Gemini,
}

pub struct HeadlessCliAdapter {
    pub id: String,
    pub binary: String,
    flavor: Tier1CliFlavor,
    args_before_prompt: Vec<String>,
    args_after_prompt: Vec<String>,
}

impl HeadlessCliAdapter {
    pub fn codex() -> Self {
        Self {
            id: "codex".into(),
            binary: "codex".into(),
            flavor: Tier1CliFlavor::Codex,
            args_before_prompt: vec!["exec".into(), "--full-auto".into(), "--json".into()],
            args_after_prompt: vec![],
        }
    }

    pub fn claude() -> Self {
        Self {
            id: "claude".into(),
            binary: "claude".into(),
            flavor: Tier1CliFlavor::Claude,
            // Claude requires the prompt to be immediately after `-p`.
            args_before_prompt: vec!["-p".into()],
            // `--output-format stream-json` requires `--verbose` in print mode.
            args_after_prompt: vec![
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
                "--include-partial-messages".into(),
                "--permission-mode".into(),
                "acceptEdits".into(),
                "--allowedTools".into(),
                "Bash,Read".into(),
            ],
        }
    }

    pub fn gemini() -> Self {
        Self {
            id: "gemini".into(),
            binary: "gemini".into(),
            flavor: Tier1CliFlavor::Gemini,
            args_before_prompt: vec![
                "--output-format".into(),
                "stream-json".into(),
                "--approval-mode".into(),
                "yolo".into(),
                "--prompt".into(),
            ],
            args_after_prompt: vec![],
        }
    }
}

#[derive(Debug, Default)]
struct ParseState {
    assistant_buf: String,
    saw_assistant_complete: bool,
    saw_done: bool,
    saw_any_json: bool,
    stdout_non_json: Vec<String>,
    stderr_lines: Vec<String>,
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
        cmd.args(&self.args_before_prompt);
        cmd.arg(&input.content);
        cmd.args(&self.args_after_prompt);
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
        let stderr = child
            .stderr
            .take()
            .context("capturing provider stderr")?;
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        let sink = event_sink;
        let id = self.id.clone();
        let flavor = self.flavor;

        let join = tokio::spawn(async move {
            let mut state = ParseState::default();
            let mut stdout_done = false;
            let mut stderr_done = false;
            loop {
                tokio::select! {
                    line = stdout_reader.next_line(), if !stdout_done => {
                        match line {
                            Ok(Some(l)) => {
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&l) {
                                    state.saw_any_json = true;
                                    let events = normalize_json_line(flavor, val, &mut state);
                                    for ev in events {
                                        let _ = sink.send(ev).await;
                                    }
                                } else {
                                    // Some providers emit non-JSON preamble lines even in JSONL modes.
                                    // Record but do not treat as assistant content.
                                    state.stdout_non_json.push(l);
                                }
                            }
                            Ok(None) => stdout_done = true,
                            Err(e) => {
                                let _ = sink.send(NormalizedEvent {
                                    event_type: SessionEventType::Error,
                                    payload_json: json!({"provider": id, "stream": "stdout", "message": e.to_string()}),
                                }).await;
                                stdout_done = true;
                            }
                        }
                    }
                    line = stderr_reader.next_line(), if !stderr_done => {
                        match line {
                            Ok(Some(l)) => {
                                state.stderr_lines.push(l);
                            }
                            Ok(None) => stderr_done = true,
                            Err(e) => {
                                state.stderr_lines.push(e.to_string());
                                stderr_done = true;
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

                if stdout_done && stderr_done {
                    break;
                }
            }

            let status = match child.wait().await {
                Ok(s) => s,
                Err(e) => {
                    let _ = sink.send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({"provider": id, "message": e.to_string()}),
                    }).await;
                    let _ = sink.send(NormalizedEvent {
                        event_type: SessionEventType::Done,
                        payload_json: json!({"status":"error","provider_exit_code":null,"provider":id}),
                    }).await;
                    return;
                }
            };

            let exit_code = status.code();
            if !status.success() {
                let _ = sink.send(NormalizedEvent {
                    event_type: SessionEventType::Error,
                    payload_json: json!({
                        "provider": id,
                        "message": "provider process exited non-zero",
                        "provider_exit_code": exit_code,
                        "stderr_tail": tail_lines(&state.stderr_lines, 50),
                        "stdout_non_json_tail": tail_lines(&state.stdout_non_json, 50),
                    }),
                }).await;
            }

            if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
                state.saw_assistant_complete = true;
                let _ = sink.send(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": state.assistant_buf}),
                }).await;
            }

            if !state.saw_done {
                state.saw_done = true;
                let _ = sink.send(NormalizedEvent {
                    event_type: SessionEventType::Done,
                    payload_json: json!({
                        "status": if status.success() { "success" } else { "error" },
                        "provider_exit_code": exit_code,
                        "stdout_non_json_tail": tail_lines(&state.stdout_non_json, 50),
                        "stderr_tail": tail_lines(&state.stderr_lines, 50),
                    }),
                }).await;
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

fn default_caps(id: &str) -> ProviderCapabilities {
    match id {
        "codex" => ProviderCapabilities {
            stream_events: true,
            stream_format: "codex-jsonl".into(),
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
            stream_format: "claude-stream-json".into(),
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
            notes: vec!["Requires --verbose for stream-json in print mode".into()],
        },
        "gemini" => ProviderCapabilities {
            stream_events: true,
            stream_format: "gemini-stream-json".into(),
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
            notes: vec!["May emit non-JSON preamble lines on stdout".into()],
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

fn tail_lines(lines: &[String], max: usize) -> Vec<String> {
    let start = lines.len().saturating_sub(max);
    lines[start..].to_vec()
}

fn normalize_json_line(
    flavor: Tier1CliFlavor,
    val: serde_json::Value,
    state: &mut ParseState,
) -> Vec<NormalizedEvent> {
    match flavor {
        Tier1CliFlavor::Gemini => normalize_gemini(val, state),
        Tier1CliFlavor::Codex => normalize_codex(val, state),
        Tier1CliFlavor::Claude => normalize_claude(val, state),
    }
}

fn normalize_gemini(val: serde_json::Value, state: &mut ParseState) -> Vec<NormalizedEvent> {
    let Some(t) = val.get("type").and_then(|v| v.as_str()) else {
        return vec![];
    };

    match t {
        "init" => vec![NormalizedEvent {
            event_type: SessionEventType::Init,
            payload_json: val,
        }],
        "message" => {
            let role = val.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if role == "assistant" && !content.is_empty() {
                state.assistant_buf.push_str(content);
                vec![NormalizedEvent {
                    event_type: SessionEventType::AssistantChunk,
                    payload_json: json!({
                        "content_fragment": content,
                        "provider_event": val,
                    }),
                }]
            } else {
                vec![]
            }
        }
        "tool_use" => {
            let tool_id = val
                .get("tool_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut payload = val;
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("tool_call_id".into(), json!(tool_id));
            }
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolCall,
                payload_json: payload,
            }]
        }
        "tool_result" => {
            let tool_id = val
                .get("tool_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut payload = val;
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("tool_call_id".into(), json!(tool_id));
            }
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolResult,
                payload_json: payload,
            }]
        }
        "error" => vec![NormalizedEvent {
            event_type: SessionEventType::Error,
            payload_json: val,
        }],
        "result" => {
            state.saw_done = true;
            let mut out = Vec::new();
            if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
                state.saw_assistant_complete = true;
                out.push(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": state.assistant_buf}),
                });
            }
            out.push(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: val,
            });
            out
        }
        _ => vec![],
    }
}

fn normalize_codex(val: serde_json::Value, state: &mut ParseState) -> Vec<NormalizedEvent> {
    let Some(t) = val.get("type").and_then(|v| v.as_str()) else {
        return vec![];
    };

    match t {
        "thread.started" => vec![NormalizedEvent {
            event_type: SessionEventType::Init,
            payload_json: val,
        }],
        "turn.started" => vec![NormalizedEvent {
            event_type: SessionEventType::Init,
            payload_json: val,
        }],
        "turn.completed" => {
            state.saw_done = true;
            vec![NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: val,
            }]
        }
        "turn.failed" | "error" => {
            state.saw_done = true;
            vec![
                NormalizedEvent {
                    event_type: SessionEventType::Error,
                    payload_json: val.clone(),
                },
                NormalizedEvent {
                    event_type: SessionEventType::Done,
                    payload_json: json!({"status":"error","provider_event": val}),
                },
            ]
        }
        "item.started" | "item.updated" | "item.completed" => {
            let item = val.get("item").cloned().unwrap_or_else(|| json!({}));
            let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match (t, item_type) {
                ("item.completed", "agent_message") => {
                    let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    state.saw_assistant_complete = true;
                    state.assistant_buf.clear();
                    state.assistant_buf.push_str(text);
                    vec![NormalizedEvent {
                        event_type: SessionEventType::AssistantComplete,
                        payload_json: json!({"full_content": text, "provider_item": item}),
                    }]
                }
                ("item.completed", "reasoning") => {
                    let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    vec![NormalizedEvent {
                        event_type: SessionEventType::AssistantChunk,
                        payload_json: json!({"content_fragment": text, "kind":"reasoning", "provider_item": item}),
                    }]
                }
                ("item.started", "command_execution") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "command_execution",
                        "provider_item": item,
                    }),
                }],
                ("item.completed", "command_execution") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "command_execution",
                        "provider_item": item,
                    }),
                }],
                ("item.completed", "file_change") => vec![
                    NormalizedEvent {
                        event_type: SessionEventType::ToolCall,
                        payload_json: json!({
                            "tool_call_id": item_id,
                            "tool_name": "file_change",
                            "provider_item": item,
                        }),
                    },
                    NormalizedEvent {
                        event_type: SessionEventType::ToolResult,
                        payload_json: json!({
                            "tool_call_id": item_id,
                            "tool_name": "file_change",
                            "provider_item": item,
                        }),
                    },
                ],
                ("item.started", "mcp_tool_call") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "mcp_tool_call",
                        "provider_item": item,
                    }),
                }],
                ("item.completed", "mcp_tool_call") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "mcp_tool_call",
                        "provider_item": item,
                    }),
                }],
                ("item.started", "web_search") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "web_search",
                        "provider_item": item,
                    }),
                }],
                ("item.completed", "web_search") => vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": item_id,
                        "tool_name": "web_search",
                        "provider_item": item,
                    }),
                }],
                ("item.completed", "error") => vec![NormalizedEvent {
                    event_type: SessionEventType::Error,
                    payload_json: json!({"provider_item": item}),
                }],
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

fn normalize_claude(val: serde_json::Value, state: &mut ParseState) -> Vec<NormalizedEvent> {
    let Some(t) = val.get("type").and_then(|v| v.as_str()) else {
        return vec![];
    };

    match t {
        "system" => {
            if val.get("subtype").and_then(|v| v.as_str()) == Some("init") {
                vec![NormalizedEvent {
                    event_type: SessionEventType::Init,
                    payload_json: val,
                }]
            } else {
                vec![]
            }
        }
        // The verbose stream includes raw streaming events; keep parsing based on the
        // higher-level assistant/user messages which contain the fully-formed tool calls/results.
        "stream_event" => vec![],
        "assistant" => normalize_claude_message_blocks(val, state, "assistant"),
        "user" => normalize_claude_message_blocks(val, state, "user"),
        "result" => {
            state.saw_done = true;
            let mut out = Vec::new();
            if !state.saw_assistant_complete {
                if let Some(result) = val.get("result").and_then(|v| v.as_str()) {
                    if !result.trim().is_empty() {
                        state.saw_assistant_complete = true;
                        out.push(NormalizedEvent {
                            event_type: SessionEventType::AssistantComplete,
                            payload_json: json!({"full_content": result}),
                        });
                    }
                } else if !state.assistant_buf.trim().is_empty() {
                    state.saw_assistant_complete = true;
                    out.push(NormalizedEvent {
                        event_type: SessionEventType::AssistantComplete,
                        payload_json: json!({"full_content": state.assistant_buf}),
                    });
                }
            }
            out.push(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: val,
            });
            out
        }
        _ => vec![],
    }
}

fn normalize_claude_message_blocks(
    val: serde_json::Value,
    state: &mut ParseState,
    kind: &str,
) -> Vec<NormalizedEvent> {
    let Some(message) = val.get("message") else {
        return vec![];
    };
    let Some(content) = message.get("content").and_then(|v| v.as_array()) else {
        return vec![];
    };

    let mut out = Vec::new();
    for block in content {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match (kind, block_type) {
            ("assistant", "tool_use") => {
                let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                out.push(NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: json!({
                        "tool_call_id": tool_id,
                        "tool_name": tool_name,
                        "parameters": input,
                        "provider_block": block,
                    }),
                });
            }
            ("user", "tool_result") => {
                let tool_id = block
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push(NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": tool_id,
                        "provider_block": block,
                        "provider_message": message,
                        "provider_event": val,
                    }),
                });
            }
            ("assistant", "text") => {
                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if !text.is_empty() {
                    state.assistant_buf.push_str(text);
                    out.push(NormalizedEvent {
                        event_type: SessionEventType::AssistantChunk,
                        payload_json: json!({
                            "content_fragment": text,
                            "provider_block": block,
                        }),
                    });
                }
            }
            _ => {}
        }
    }

    out
}
