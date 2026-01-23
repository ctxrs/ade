
#[derive(Debug, Clone)]
pub struct AcpProviderOptionsProbe {
    pub supports_load: bool,
    pub supports_resume: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub modes: Option<serde_json::Value>,
    pub models: Option<serde_json::Value>,
    pub auth_required: bool,
    pub acp_error: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct AcpProviderVerifyProbe {
    /// One of: `ok`, `auth_required`, `network_error`, `error`.
    pub status: String,
    pub auth_required: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub acp_error: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct AcpProviderAuthenticateProbe {
    /// One of: `ok`, `auth_required`, `error`.
    pub status: String,
    pub auth_required: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub acp_error: Option<serde_json::Value>,
}

fn default_auth_method_id(methods: &serde_json::Value) -> Option<String> {
    let list = methods.as_array()?;
    for m in list {
        if let Some(id) = m
            .get("methodId")
            .or_else(|| m.get("method_id"))
            .or_else(|| m.get("id"))
            .and_then(|v| v.as_str())
        {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Best-effort probe for ACP providers that only expose model/mode lists on `session/new`.
///
/// This intentionally does **not** prompt; it runs `initialize` + `session/new`, extracts
/// `modes/models` from the response, then terminates the child process.
pub async fn probe_provider_options(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<AcpProviderOptionsProbe> {
    let probe_timeout = if agent.provider_id == "gemini" {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(20)
    };

    timeout(probe_timeout, async move {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning ACP agent {} ({})", agent.provider_id, agent.command))?;

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": client.client_capabilities,
                "clientInfo": {
                    "name": client.client_name,
                    "title": client.client_title,
                    "version": client.client_version,
                }
            }),
        )
        .await?;
        if let Some(err) = init_resp.get("error") {
            anyhow::bail!("ACP initialize error: {err}");
        }

        let capabilities = init_resp
            .get("result")
            .and_then(|v| v.get("capabilities").or_else(|| v.get("agentCapabilities")));
        let supports_load = capabilities
            .and_then(|v| v.get("loadSession"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supports_resume = capabilities
            .and_then(|v| {
                v.get("sessionCapabilities")
                    .or_else(|| v.get("session_capabilities"))
            })
            .and_then(|v| v.get("resume"))
            .map(|v| match v {
                serde_json::Value::Bool(value) => *value,
                serde_json::Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

        let cwd = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.clone())
            .to_string_lossy()
            .to_string();
        let mcp_servers = client
            .mcp_servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;
        if let Some(err) = new_resp.get("error") {
            let auth_required = is_auth_required_error(err);
            let _ = child.kill().await;
            return Ok(AcpProviderOptionsProbe {
                supports_load,
                supports_resume,
                auth_methods,
                modes: None,
                models: None,
                auth_required,
                acp_error: Some(err.clone()),
            });
        }

        let modes = new_resp.get("result").and_then(|v| v.get("modes")).cloned();
        let models = new_resp.get("result").and_then(|v| v.get("models")).cloned();

        let _ = child.kill().await;

        Ok(AcpProviderOptionsProbe {
            supports_load,
            supports_resume,
            auth_methods,
            modes,
            models,
            auth_required: false,
            acp_error: None,
        })
    })
    .await
    .context("ACP probe timed out")?
}

fn is_network_error(err: &serde_json::Value) -> bool {
    let Some(obj) = err.as_object() else {
        return false;
    };

    let msg = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let data_str = obj
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    let hay = format!("{msg}\n{data_str}");
    [
        "econnrefused",
        "enotfound",
        "ehostunreach",
        "enetunreach",
        "etimedout",
        "timeout",
        "timed out",
        "tls",
        "ssl",
        "certificate",
        "handshake",
        "connection reset",
        "socket hang up",
        "network is unreachable",
        "no route to host",
        "failed to connect",
        "could not resolve",
        "name or service not known",
        "temporary failure in name resolution",
    ]
    .iter()
    .any(|needle| hay.contains(needle))
}

/// Best-effort connectivity check for ACP providers.
///
/// This intentionally **does** prompt with a tiny request so we can detect failures that only
/// surface on `session/prompt` (for example missing BYO API keys/endpoints).
pub async fn verify_provider_connection(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<AcpProviderVerifyProbe> {
    let probe_timeout = if agent.provider_id == "gemini" {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(30)
    };

    timeout(probe_timeout, async move {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning ACP agent {} ({})", agent.provider_id, agent.command))?;

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": client.client_capabilities,
                "clientInfo": {
                    "name": client.client_name,
                    "title": client.client_title,
                    "version": client.client_version,
                }
            }),
        )
        .await?;

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

        if let Some(err) = init_resp.get("error") {
            let _ = child.kill().await;
            return Ok(AcpProviderVerifyProbe {
                status: "error".to_string(),
                auth_required: is_auth_required_error(err),
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let cwd = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.clone())
            .to_string_lossy()
            .to_string();
        let mcp_servers = client
            .mcp_servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;
        if let Some(err) = new_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            let status = if auth_required {
                "auth_required"
            } else if is_network_error(err) {
                "network_error"
            } else {
                "error"
            };
            return Ok(AcpProviderVerifyProbe {
                status: status.to_string(),
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let acp_session_id = new_resp
            .get("result")
            .and_then(|v| v.get("sessionId"))
            .and_then(|v| v.as_str())
            .context("missing sessionId in session/new response")?
            .to_string();

        let prompt_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/prompt",
            json!({
                "sessionId": acp_session_id,
                "prompt": [
                    {"type":"text","text":"Respond with exactly: OK"}
                ]
            }),
        )
        .await?;

        if let Some(err) = prompt_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            let status = if auth_required {
                "auth_required"
            } else if is_network_error(err) {
                "network_error"
            } else {
                "error"
            };
            return Ok(AcpProviderVerifyProbe {
                status: status.to_string(),
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let _ = child.kill().await;
        Ok(AcpProviderVerifyProbe {
            status: "ok".to_string(),
            auth_required: false,
            auth_methods,
            acp_error: None,
        })
    })
    .await
    .context("ACP verify timed out")?
}

/// Best-effort provider-level authentication for ACP providers.
///
/// Calls ACP `authenticate` and then attempts `session/new` to confirm auth state.
pub async fn authenticate_provider(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
    method_id: Option<String>,
) -> Result<AcpProviderAuthenticateProbe> {
    let auth_timeout = Duration::from_secs(5 * 60);

    timeout(auth_timeout, async move {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning ACP agent {} ({})", agent.provider_id, agent.command))?;

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": client.client_capabilities,
                "clientInfo": {
                    "name": client.client_name,
                    "title": client.client_title,
                    "version": client.client_version,
                }
            }),
        )
        .await?;

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

        if let Some(err) = init_resp.get("error") {
            let _ = child.kill().await;
            return Ok(AcpProviderAuthenticateProbe {
                status: "error".to_string(),
                auth_required: is_auth_required_error(err),
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let Some(method_id) = method_id
            .or_else(|| auth_methods.as_ref().and_then(default_auth_method_id))
        else {
            let _ = child.kill().await;
            anyhow::bail!("no authentication methods advertised by provider");
        };

        let auth_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "authenticate",
            json!({"methodId": method_id}),
        )
        .await?;
        if let Some(err) = auth_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            return Ok(AcpProviderAuthenticateProbe {
                status: if auth_required {
                    "auth_required".to_string()
                } else {
                    "error".to_string()
                },
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let cwd = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.clone())
            .to_string_lossy()
            .to_string();
        let mcp_servers = client
            .mcp_servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;

        let _ = child.kill().await;

        if let Some(err) = new_resp.get("error") {
            let auth_required = is_auth_required_error(err);
            return Ok(AcpProviderAuthenticateProbe {
                status: if auth_required {
                    "auth_required".to_string()
                } else {
                    "error".to_string()
                },
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        Ok(AcpProviderAuthenticateProbe {
            status: "ok".to_string(),
            auth_required: false,
            auth_methods,
            acp_error: None,
        })
    })
    .await
    .context("ACP authenticate timed out")?
}

async fn acp_probe_request(
    stdin: &mut tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    provider_id: &str,
    next_id: &mut u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let id = *next_id;
    *next_id += 1;
    let msg = json!({"jsonrpc":"2.0","id": id, "method": method, "params": params});
    let line = serde_json::to_string(&msg).context("serializing ACP request")?;
    stdin
        .write_all(line.as_bytes())
        .await
        .context("writing ACP request")?;
    stdin.write_all(b"\n").await.ok();
    stdin.flush().await.ok();

    loop {
        let line = stdout_reader
            .next_line()
            .await
            .context("reading ACP response line")?;
        let Some(line) = line else {
            anyhow::bail!("ACP agent exited during probe");
        };
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(mid) = parsed.get("id").and_then(jsonrpc_id_u64) {
            if mid == id {
                return Ok(parsed);
            }
            continue;
        }

        if parsed.get("method").and_then(|v| v.as_str()) == Some("session/request_permission") {
            if let Some(resp) = build_request_permission_response(provider_id, &parsed)? {
                stdin.write_all(resp.as_bytes()).await.ok();
                stdin.write_all(b"\n").await.ok();
                stdin.flush().await.ok();
            }
            continue;
        }
    }
}

fn content_text(block: &serde_json::Value) -> Option<String> {
    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
        return block
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    None
}

fn is_auth_required_error(err: &serde_json::Value) -> bool {
    let Some(obj) = err.as_object() else {
        return false;
    };

    let code_matches = obj
        .get("code")
        .and_then(|v| v.as_i64())
        .is_some_and(|code| code == -32001 || code == 401);

    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let msg_matches = message.contains("authrequired")
        || message.contains("auth_required")
        || message.contains("authentication required")
        || message.contains("unauthorized");

    let data_str = obj
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let data_matches = data_str.contains("authrequired") || data_str.contains("auth_required");

    code_matches || msg_matches || data_matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn normalizes_agent_message_chunk_and_buffers_text() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event_type,
            SessionEventType::AssistantChunk
        ));
        assert_eq!(state.assistant_buf, "hello");
    }

    #[test]
    fn normalizes_tool_call_and_terminal_tool_result() {
        let mut state = StreamState::default();
        let tool_call = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call_1",
                    "title": "Do thing",
                    "kind": "other",
                    "status": "pending"
                }
            }
        });
        let tool_done = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "status": "completed"
                }
            }
        });

        let ev1 = normalize_session_update(&tool_call, &mut state);
        assert_eq!(ev1.len(), 1);
        assert!(matches!(ev1[0].event_type, SessionEventType::ToolCall));

        let ev2 = normalize_session_update(&tool_done, &mut state);
        assert_eq!(ev2.len(), 2);
        assert!(matches!(
            ev2[0].event_type,
            SessionEventType::ToolCallUpdate
        ));
        assert!(matches!(ev2[1].event_type, SessionEventType::ToolResult));
    }

    #[test]
    fn normalizes_available_commands_update_as_notice() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [
                        { "name": "review", "description": "Review changes" }
                    ]
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, SessionEventType::Notice));
        assert_eq!(
            events[0]
                .payload_json
                .get("acp_update")
                .and_then(|v| v.get("sessionUpdate"))
                .and_then(|v| v.as_str()),
            Some("available_commands_update")
        );
        assert_eq!(state.assistant_buf, "");
    }

    #[test]
    fn surfaces_context_window_from_meta() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [],
                    "_meta": {
                        "context_window": {
                            "context_window_tokens": 1000,
                            "context_tokens_estimate": 200,
                            "remaining_fraction": 0.8
                        }
                    }
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]
                .payload_json
                .get("context_window")
                .and_then(|v| v.get("context_window_tokens"))
                .and_then(Value::as_i64),
            Some(1000)
        );
    }

    #[test]
    fn auto_approves_allow_once_permission() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess_1",
                "toolCall": { "toolCallId": "call_1", "status": "pending", "title": "edit", "kind": "edit" },
                "options": [
                    { "optionId": "reject", "name": "No", "kind": "reject_once" },
                    { "optionId": "allow", "name": "Yes", "kind": "allow_once" }
                ]
            }
        });

        let resp = build_request_permission_response("codex", &req)
            .unwrap()
            .unwrap();
        let resp: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(resp.get("id").and_then(|v| v.as_i64()), Some(7));
        let option_id = resp
            .get("result")
            .and_then(|v| v.get("outcome"))
            .and_then(|v| v.get("optionId"))
            .and_then(|v| v.as_str());
        assert_eq!(option_id, Some("allow"));
    }

    #[test]
    fn parses_jsonrpc_id_as_u64() {
        assert_eq!(jsonrpc_id_u64(&json!(7)), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!("7")), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!(7_i64)), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!(-1)), None);
        assert_eq!(jsonrpc_id_u64(&json!("not-a-number")), None);
    }
}
