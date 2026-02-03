use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Agent, CancelNotification, Client, ClientCapabilities, ClientSideConnection, InitializeRequest,
    NewSessionRequest, PermissionOptionKind, ProtocolVersion, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification,
};
use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::crp::{CrpChannel, CrpCommand, CrpEnvelope, CrpEvent, CrpTurnStatus, CrpWriter};
use crate::translate::Translator;

struct SessionState {
    translator: Translator,
    cwd: PathBuf,
    active_turn_id: Option<String>,
}

#[derive(Default)]
struct Sessions {
    by_acp: HashMap<String, SessionState>,
    by_crp: HashMap<String, String>,
}

struct BridgeClient {
    events_tx: mpsc::Sender<CrpEnvelope>,
    sessions: Arc<Mutex<Sessions>>,
}

#[async_trait::async_trait(?Send)]
impl Client for BridgeClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> agent_client_protocol::Result<RequestPermissionResponse> {
        let option_id = args
            .options
            .iter()
            .find(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                )
            })
            .or_else(|| args.options.first())
            .map(|option| option.option_id.clone());

        let outcome = match option_id {
            Some(option_id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
            None => RequestPermissionOutcome::Cancelled,
        };

        Ok(RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        let mut sessions = self.sessions.lock().await;
        let session_id = args.session_id.to_string();
        if let Some(state) = sessions.by_acp.get_mut(&session_id) {
            let events = state.translator.apply_update(args.update);
            for event in events {
                let _ = self.events_tx.send(event).await;
            }
        }
        Ok(())
    }
}

pub async fn run_bridge(config: Config) -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut command = Command::new(&config.command);
    command.args(&config.args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
    if let Some(cwd) = &config.cwd {
        command.current_dir(cwd);
    }
    if !config.env.is_empty() {
        command.envs(&config.env);
    }

    let mut child = command.spawn().context("spawn acp harness")?;
    let child_stdin = child.stdin.take().ok_or_else(|| anyhow!("acp stdin unavailable"))?;
    let child_stdout = child.stdout.take().ok_or_else(|| anyhow!("acp stdout unavailable"))?;

    let (events_tx, mut events_rx) = mpsc::channel::<CrpEnvelope>(256);
    let sessions = Arc::new(Mutex::new(Sessions::default()));

    let client = BridgeClient {
        events_tx: events_tx.clone(),
        sessions: Arc::clone(&sessions),
    };

    let local = LocalSet::new();
    local
        .run_until(async move {
            let (acp, io_task) = ClientSideConnection::new(
                client,
                child_stdin.compat_write(),
                child_stdout.compat(),
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            tokio::task::spawn_local(async move {
                if let Err(err) = io_task.await {
                    error!("acp io task failed: {err}");
                }
            });

            let init = InitializeRequest::new(ProtocolVersion::LATEST)
                .client_capabilities(ClientCapabilities::default());
            if let Err(err) = acp.initialize(init).await {
                return Err(anyhow!("acp initialize failed: {err}"));
            }

            let mut writer = CrpWriter::new(tokio::io::stdout());
            let writer_task = tokio::task::spawn_local(async move {
                while let Some(event) = events_rx.recv().await {
                    if let Err(err) = writer.send(&event).await {
                        error!("failed to write crp event: {err}");
                        break;
                    }
                }
            });

            let stdin = tokio::io::stdin();
            let mut lines = BufReader::new(stdin).lines();

            while let Some(line) = lines.next_line().await? {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let command: CrpCommand = match serde_json::from_str(trimmed) {
                    Ok(command) => command,
                    Err(err) => {
                        warn!("invalid crp command: {err}");
                        continue;
                    }
                };

                if let Err(err) = handle_command(command, &acp, &events_tx, &sessions, &config).await {
                    warn!("crp command error: {err}");
                }
            }

            writer_task.await.ok();
            Ok(())
        })
        .await
}

async fn handle_command(
    command: CrpCommand,
    acp: &ClientSideConnection,
    events_tx: &mpsc::Sender<CrpEnvelope>,
    sessions: &Arc<Mutex<Sessions>>,
    config: &Config,
) -> Result<()> {
    match command {
        CrpCommand::SessionOpen { session_id, config: crp_config } => {
            let crp_session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let cwd = crp_config
                .and_then(|cfg| cfg.cwd)
                .or_else(|| config.cwd.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

            let response = acp
                .new_session(NewSessionRequest::new(cwd.clone()))
                .await
                .context("acp new_session")?;

            let acp_session_id = response.session_id.to_string();

            let mut sessions_guard = sessions.lock().await;
            if sessions_guard.by_crp.contains_key(&crp_session_id) {
                warn!("session.open ignored: session already active");
                return Ok(());
            }

            let translator = Translator::new(crp_session_id.clone(), config.reasoning_mode);

            sessions_guard.by_crp.insert(crp_session_id.clone(), acp_session_id.clone());
            sessions_guard.by_acp.insert(
                acp_session_id.clone(),
                SessionState {
                    translator,
                    cwd,
                    active_turn_id: None,
                },
            );

            let opened = CrpEnvelope {
                channel: CrpChannel::Control,
                event: CrpEvent::SessionOpened {
                    session_id: crp_session_id,
                    provider_session_id: Some(acp_session_id),
                },
            };
            let _ = events_tx.send(opened).await;
        }
        CrpCommand::SessionPrompt {
            session_id,
            turn_id,
            items,
            prompt,
            cwd,
            ..
        } => {
            let crp_session_id = session_id.ok_or_else(|| anyhow!("session.prompt missing session_id"))?;
            let prompt_text = extract_prompt(prompt, items)
                .ok_or_else(|| anyhow!("session.prompt missing prompt"))?;

            let mut sessions_guard = sessions.lock().await;
            let acp_session_id = sessions_guard
                .by_crp
                .get(&crp_session_id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown session_id: {crp_session_id}"))?;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("unknown provider session: {acp_session_id}"))?;

            if let Some(cwd) = cwd {
                state.cwd = cwd;
            }

            let turn_id = turn_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let message_id = Uuid::new_v4().to_string();
            state.active_turn_id = Some(turn_id.clone());
            state.translator.start_turn(turn_id.clone(), message_id.clone());

            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::TurnStarted {
                        session_id: crp_session_id.clone(),
                        turn_id: turn_id.clone(),
                    },
                })
                .await;

            drop(sessions_guard);

            let prompt_blocks = vec![prompt_text.into()];
            let prompt_req = PromptRequest::new(acp_session_id.clone(), prompt_blocks);
            let response = acp.prompt(prompt_req).await.context("acp prompt")?;

            let mut sessions_guard = sessions.lock().await;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("missing session after prompt"))?;

            if let Some(final_event) = state.translator.finish_turn() {
                let _ = events_tx.send(final_event).await;
            }
            state.translator.clear_turn();
            state.active_turn_id = None;

            let status = if response.stop_reason == agent_client_protocol::StopReason::Cancelled {
                CrpTurnStatus::Canceled
            } else {
                CrpTurnStatus::Success
            };

            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::TurnCompleted {
                        session_id: crp_session_id,
                        turn_id,
                        status,
                        error: None,
                    },
                })
                .await;
        }
        CrpCommand::SessionCancel { session_id, .. } => {
            let crp_session_id = session_id.ok_or_else(|| anyhow!("session.cancel missing session_id"))?;
            let sessions_guard = sessions.lock().await;
            let acp_session_id = sessions_guard
                .by_crp
                .get(&crp_session_id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown session_id: {crp_session_id}"))?;
            drop(sessions_guard);

            let cancel = CancelNotification::new(acp_session_id);
            let _ = acp.cancel(cancel).await;
        }
        CrpCommand::ModelsList { .. } => {
            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::ModelsList {
                        models: vec![],
                        current_model_id: None,
                    },
                })
                .await;
        }
    }

    Ok(())
}

fn extract_prompt(prompt: Option<String>, items: Option<Vec<serde_json::Value>>) -> Option<String> {
    if let Some(prompt) = prompt {
        return Some(prompt);
    }

    let items = items?;
    let mut parts = Vec::new();
    for item in items {
        if let Some(text) = item.as_str() {
            parts.push(text.to_string());
            continue;
        }
        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
            parts.push(text.to_string());
            continue;
        }
        if let Some(text) = item.get("content").and_then(|v| v.as_str()) {
            parts.push(text.to_string());
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}
