use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use context_core::models::SessionEventType;

use crate::acp::{AcpAgentConfig, AcpClientConfig, AcpMcpServer, AcpSessionPool};
use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

pub struct Tier1AcpAdapter {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pool: Arc<AcpSessionPool>,
}

impl Tier1AcpAdapter {
    fn new(id: &str, command: &str, args: Vec<String>) -> Self {
        let agent = AcpAgentConfig {
            provider_id: id.to_string(),
            command: command.to_string(),
            args: args.clone(),
        };
        let pool = Arc::new(AcpSessionPool::new(agent));
        pool.spawn_reaper();
        Self {
            id: id.to_string(),
            command: command.to_string(),
            args,
            pool,
        }
    }

    pub fn codex() -> Self {
        Self::new("codex", "codex-acp", vec![])
    }

    pub fn claude() -> Self {
        Self::new("claude", "claude-code-acp", vec![])
    }

    pub fn gemini() -> Self {
        Self::new("gemini", "gemini", vec!["--experimental-acp".into()])
    }
}

#[async_trait]
impl ProviderAdapter for Tier1AcpAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let detected_path = which::which(&self.command).ok();
        let installed = detected_path.is_some();

        let mut diagnostics = Vec::new();
        if !installed {
            diagnostics.push(format!(
                "ACP agent executable not found on PATH: {}",
                self.command
            ));
        }

        Ok(ProviderStatus {
            provider_id: self.id.clone(),
            installed,
            detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
            version: None,
            capabilities: if installed {
                Some(default_caps(&self.id))
            } else {
                None
            },
            health: if installed {
                ProviderHealth::Ok
            } else {
                ProviderHealth::Missing
            },
            diagnostics,
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let mut mcp_env = HashMap::new();
        if let Some(url) = env.get("CONTEXT_DAEMON_URL") {
            mcp_env.insert("CONTEXT_DAEMON_URL".to_string(), url.clone());
        }
        if let Some(token) = env.get("CONTEXT_MCP_TOKEN") {
            mcp_env.insert("CONTEXT_MCP_TOKEN".to_string(), token.clone());
        }
        if let Some(session_id) = env.get("CONTEXT_SESSION_ID") {
            mcp_env.insert("CONTEXT_SESSION_ID".to_string(), session_id.clone());
        }

        let mcp_command = env
            .get("CONTEXT_MCP_COMMAND")
            .cloned()
            .unwrap_or_else(|| "context-mcp".to_string());

        let mcp_enabled = env
            .get("CONTEXT_MCP_DISABLED")
            .map(|v| v != "1" && v.to_lowercase() != "true")
            .unwrap_or(true);

        let mcp_servers = if mcp_enabled {
            vec![AcpMcpServer {
                name: "context".to_string(),
                command: mcp_command,
                args: vec!["--stdio".to_string()],
                env: mcp_env,
            }]
        } else {
            vec![]
        };

        let client = AcpClientConfig {
            client_name: "context".to_string(),
            client_title: "Context".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            client_capabilities: json!({
                "fs": {"readTextFile": false, "writeTextFile": false},
                "terminal": false
            }),
            mcp_servers,
        };

        let session_key = env
            .get("CONTEXT_SESSION_ID")
            .cloned()
            .unwrap_or_else(|| "unknown-session".to_string());

        let provider_id = self.id.clone();
        let pool = Arc::clone(&self.pool);
        let join = tokio::spawn(async move {
            let mut prompt: Vec<serde_json::Value> = Vec::new();

            for att in input.attachments.iter() {
                match att {
                    context_core::models::MessageAttachment::Image {
                        mime_type,
                        data_base64,
                        ..
                    } => {
                        prompt.push(json!({"type":"image","data": data_base64, "mimeType": mime_type}));
                    }
                }
            }

            if let Ok(mut blocks) = embed_at_file_refs(&workdir, &input.content).await {
                prompt.append(&mut blocks);
            }

            prompt.push(json!({"type":"text","text": input.content}));
            if let Err(e) = pool
                .prompt(
                    session_key,
                    client,
                    prompt,
                    workdir,
                    env,
                    event_sink.clone(),
                    cancel_rx,
                )
            .await
            {
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({"provider": provider_id, "message": e.to_string()}),
                    })
                    .await;
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Done,
                        payload_json: json!({"provider": provider_id, "status": "error"}),
                    })
                    .await;
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
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle.join).await;
        handle.join.abort(); // best-effort cleanup
        Ok(())
    }

    async fn set_session_model(&self, session_key: String, model_id: String) -> Result<()> {
        self.pool.set_model(session_key, model_id).await
    }

    async fn set_session_mode(&self, session_key: String, mode_id: String) -> Result<()> {
        self.pool.set_mode(session_key, mode_id).await
    }
}

async fn embed_at_file_refs(workdir: &PathBuf, input: &str) -> Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    let root = workdir.canonicalize().unwrap_or_else(|_| workdir.clone());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for tok in input.split_whitespace() {
        let Some(rest) = tok.strip_prefix('@') else { continue };
        let path = rest
            .trim_matches(|c: char| c == '"' || c == '\'' || c == ')' || c == '(' || c == ',' || c == '.' || c == ';' || c == ':' )
            .to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }

        let candidate = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            root.join(&path)
        };
        let Ok(canon) = candidate.canonicalize() else { continue };
        if !canon.starts_with(&root) {
            continue;
        }

        let bytes = match tokio::fs::read(&canon).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        const MAX: usize = 200 * 1024;
        let bytes = if bytes.len() > MAX { &bytes[..MAX] } else { &bytes[..] };
        let text = String::from_utf8_lossy(bytes).to_string();
        let uri = format!("file://{}", canon.to_string_lossy());
        out.push(json!({
            "type": "resource",
            "resource": { "uri": uri, "text": text, "mimeType": "text/plain" }
        }));
    }

    Ok(out)
}

fn default_caps(id: &str) -> ProviderCapabilities {
    match id {
        "codex" | "claude" | "gemini" => ProviderCapabilities {
            stream_events: true,
            stream_format: "acp-jsonrpc".into(),
            has_turn_boundaries: true,
            has_tool_call_ids: true,
            has_file_change_events: false,
            has_command_events: false,
            supports_resume: false,
            supports_stable_session_id: false,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: false,
            supports_acp: true,
            supports_interactive_tui: false,
            supports_private_state_dir: false,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: vec![],
        },
        _ => ProviderCapabilities {
            stream_events: true,
            stream_format: "acp-jsonrpc".into(),
            has_turn_boundaries: true,
            has_tool_call_ids: true,
            has_file_change_events: false,
            has_command_events: false,
            supports_resume: false,
            supports_stable_session_id: false,
            supports_fork_or_rewind: false,
            supports_headless: true,
            supports_server_mode: false,
            supports_acp: true,
            supports_interactive_tui: false,
            supports_private_state_dir: false,
            supports_sandbox_flags: false,
            supports_approval_flags: false,
            notes: vec![],
        },
    }
}
