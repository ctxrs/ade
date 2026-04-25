#[path = "tool_catalog.rs"]
mod tool_catalog;

fn removed_lsp_tool_message(name: &str) -> Option<String> {
    if name.starts_with("lsp_")
        || matches!(
            name,
            "list_edit_plans" | "get_edit_plan" | "apply_edit_plan" | "discard_edit_plan"
        )
    {
        return Some(format!(
            "tool removed: {name} (this daemon no longer supports that legacy MCP tool; recover the last implementation from commit 795129c6a if needed)"
        ));
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    if !cli.stdio {
        anyhow::bail!("only --stdio transport is implemented");
    }
    let server_version = build_identity::current_build_version()
        .context("loading ctx-mcp build identity")?;

    let daemon_url = ctx_env("DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:4399".to_string());
    let client = reqwest::Client::new();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut out = tokio::io::BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("invalid json: {e}");
                continue;
            }
        };

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        // Notifications have no response.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };

        let response = match method {
            "ping" => ok(id.clone(), json!({})),
            "initialize" => {
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let protocol_version = params
                    .get("protocolVersion")
                    .cloned()
                    .unwrap_or_else(|| json!("2025-11-25"));
                ok(
                    id.clone(),
                    json!({
                        "protocolVersion": protocol_version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": "ctx-mcp",
                            "title": "ctx MCP",
                            "version": server_version.clone(),
                            "description": "ctx daemon tools (bridge)"
                        }
                    }),
                )
            }
            "tools/list" => ok(id.clone(), tool_catalog::tools_list_response()),
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let raw_name = name.to_string();
                // Tool names must be [a-zA-Z0-9_-] to satisfy Codex MCP validation.
                // Accept ctx_* and ctx.* aliases but normalize to unprefixed underscore names.
                let name = if let Some(rest) = raw_name.strip_prefix("ctx.") {
                    rest.replace('.', "_")
                } else if let Some(rest) = raw_name.strip_prefix("ctx_") {
                    rest.to_string()
                } else {
                    raw_name
                };

                if !dev_tools_enabled() && name.as_str() == "ping" {
                    ok(
                        id.clone(),
                        tool_err(anyhow::anyhow!(
                            "tool disabled: {name} (ping is dev-only; set CTX_MCP_DEV_MODE=1 to enable)"
                        )),
                    )
                } else if let Some(message) = removed_lsp_tool_message(name.as_str()) {
                    ok(id.clone(), tool_err(anyhow::anyhow!(message)))
                } else {
                    let mut arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                    if let Some(tool_call_id) = tool_call_id_from_params(&params) {
                        if let Some(obj) = arguments.as_object_mut() {
                            obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
                        } else {
                            arguments = json!({ "tool_call_id": tool_call_id });
                        }
                    }
                    match name.as_str() {
                        "ping" => ok(
                            id.clone(),
                            json!({
                                "content": [{"type":"text","text": "{\"ok\":true}"}],
                                "isError": false
                            }),
                        ),
                        "list_workspaces" => {
                            let _ = arguments; // currently unused
                            match list_workspaces(&client, &daemon_url).await {
                                Ok(val) => ok(
                                    id.clone(),
                                    json!({
                                        "content": [{"type":"text","text": serde_json::to_string_pretty(&val).unwrap_or_else(|_| "[]".into())}],
                                        "isError": false
                                    }),
                                ),
                                Err(e) => ok(
                                    id.clone(),
                                    json!({
                                        "content": [{"type":"text","text": format!("error: {e}")}],
                                        "isError": true
                                    }),
                                ),
                            }
                        }
                        "merge_queue_submit" => {
                            match merge_queue_submit_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "subagent_init" => {
                            match agent_init_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "subagent_reply" => {
                            match agent_reply_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "subagent_wait" => {
                            match subagent_wait_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "subagent_interrupt" => {
                            match subagent_interrupt_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "subagent_list" => match subagent_list_call(&client, &daemon_url).await {
                            Ok(val) => ok(id.clone(), tool_ok(val)),
                            Err(e) => ok(id.clone(), tool_err(e)),
                        },
                        "artifacts_set" => {
                            let normalized =
                                (|| -> std::result::Result<(String, Vec<Value>), Value> {
                                    let session_id =
                                        ctx_env_opt("SESSION_ID").ok_or_else(|| {
                                            error(
                                                id.clone(),
                                                -32602,
                                                "Invalid params",
                                                Some(json!({"missing":"session_context"})),
                                            )
                                        })?;
                                    let items = arguments
                                        .get("artifacts")
                                        .and_then(|v| v.as_array())
                                        .ok_or_else(|| {
                                            error(
                                                id.clone(),
                                                -32602,
                                                "Invalid params",
                                                Some(json!({"missing":"artifacts"})),
                                            )
                                        })?;

                                    let mut normalized = Vec::with_capacity(items.len());
                                    for (idx, item) in items.iter().enumerate() {
                                        let obj = item.as_object().ok_or_else(|| {
                                        error(
                                            id.clone(),
                                            -32602,
                                            "Invalid params",
                                            Some(json!({"index": idx, "message": "artifact must be an object"})),
                                        )
                                    })?;
                                        let abs = obj
                                            .get("absoluteFilePath")
                                            .or_else(|| obj.get("absolute_file_path"))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                        let absolute_file_path = abs
                                        .filter(|s| !s.trim().is_empty())
                                        .ok_or_else(|| {
                                        error(
                                            id.clone(),
                                            -32602,
                                            "Invalid params",
                                            Some(
                                                json!({"index": idx, "missing":"absoluteFilePath"}),
                                            ),
                                        )
                                    })?;
                                        let name = obj
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                        let mime_type = obj
                                            .get("mimeType")
                                            .or_else(|| obj.get("mime_type"))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());

                                        normalized.push(json!({
                                            "absolute_file_path": absolute_file_path,
                                            "name": name,
                                            "mime_type": mime_type,
                                        }));
                                    }

                                    Ok((session_id, normalized))
                                })();

                            match normalized {
                                Ok((session_id, normalized)) => {
                                    match set_artifacts(
                                        &client,
                                        &daemon_url,
                                        &session_id,
                                        normalized,
                                    )
                                    .await
                                    {
                                        Ok(val) => ok(id.clone(), tool_ok(val)),
                                        Err(e) => ok(id.clone(), tool_err(e)),
                                    }
                                }
                                Err(err) => err,
                            }
                        }
                        "oracle" => match oracle_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.clone(), tool_ok(val)),
                            Err(e) => ok(id.clone(), tool_err(e)),
                        },
                        // TODO: Re-enable web session MCP tool handlers.
                        /*
                        "session_create" => {
                            match session_create_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "session_list" => {
                            match session_list_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "session_info" => {
                            match session_info_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "session_run" => {
                            match session_run_call(&client, &daemon_url, &arguments, false).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "session_eval" => {
                            match session_run_call(&client, &daemon_url, &arguments, true).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "session_close" => {
                            match session_close_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        */
                        _ => error(
                            id.clone(),
                            -32601,
                            "Method not found",
                            Some(json!({"tool": name})),
                        ),
                    }
                }
            }
            _ => error(
                id.clone(),
                -32601,
                "Method not found",
                Some(json!({"method": method})),
            ),
        };

        let line = serde_json::to_string(&response).context("serializing MCP response")?;
        out.write_all(line.as_bytes()).await?;
        out.write_all(b"\n").await?;
        out.flush().await?;
    }

    Ok(())
}
