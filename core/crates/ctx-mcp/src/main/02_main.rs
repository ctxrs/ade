#[path = "tool_catalog.rs"]
mod tool_catalog;

use crate::lsp::*;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    if !cli.stdio {
        anyhow::bail!("only --stdio transport is implemented");
    }

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
                            "version": env!("CARGO_PKG_VERSION"),
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
                } else if !lsp_tools_enabled() && is_lsp_related_tool(name.as_str()) {
                    ok(
                        id.clone(),
                        tool_err(anyhow::anyhow!(
                            "tool disabled: {name} (LSP/edit-plan MCP tools are disabled; set CTX_MCP_ENABLE_LSP_TOOLS=1 to enable)"
                        )),
                    )
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
                        "lsp_status" => {
                            let _ = arguments;
                            match lsp_status(&client, &daemon_url).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_install_server" => {
                            let server_id = arguments
                                .get("server_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if server_id.is_empty() {
                                error(
                                    id.clone(),
                                    -32602,
                                    "Invalid params",
                                    Some(json!({"missing":"server_id"})),
                                )
                            } else {
                                match lsp_install_server(&client, &daemon_url, &server_id).await {
                                    Ok(val) => ok(id.clone(), tool_ok(val)),
                                    Err(e) => ok(id.clone(), tool_err(e)),
                                }
                            }
                        }
                        "lsp_catalog_list" => {
                            let _ = arguments;
                            match lsp_catalog_list(&client, &daemon_url).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_catalog_install" => {
                            let catalog_id = arguments
                                .get("catalog_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if catalog_id.is_empty() {
                                error(
                                    id.clone(),
                                    -32602,
                                    "Invalid params",
                                    Some(json!({"missing":"catalog_id"})),
                                )
                            } else {
                                match lsp_catalog_install(&client, &daemon_url, &catalog_id).await {
                                    Ok(val) => ok(id.clone(), tool_ok(val)),
                                    Err(e) => ok(id.clone(), tool_err(e)),
                                }
                            }
                        }
                        "lsp_diagnostics" => {
                            let path = arguments
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if path.is_empty() {
                                error(
                                    id.clone(),
                                    -32602,
                                    "Invalid params",
                                    Some(json!({"missing":"path"})),
                                )
                            } else {
                                let root_path = arguments
                                    .get("root_path")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let session_id = ctx_env_opt("SESSION_ID");
                                match lsp_diagnostics(
                                    &client,
                                    &daemon_url,
                                    session_id,
                                    root_path,
                                    path,
                                )
                                .await
                                {
                                    Ok(val) => ok(id.clone(), tool_ok(val)),
                                    Err(e) => ok(id.clone(), tool_err(e)),
                                }
                            }
                        }
                        "lsp_definition" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/definition",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_type_definition" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/type_definition",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_implementation" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/implementation",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_references" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/references",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_hover" => {
                            match lsp_pos_call(&client, &daemon_url, "/api/lsp/hover", &arguments)
                                .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_signature_help" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/signature_help",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_completion" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/completion",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_completion_resolve" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/completion/resolve",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_action_resolve" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/code_action/resolve",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_inlay_hints" => {
                            match lsp_range_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/inlay_hints",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_document_highlight" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/document_highlight",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_selection_ranges" => {
                            match lsp_selection_ranges_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/selection_ranges",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_call_hierarchy_prepare" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/call_hierarchy/prepare",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_call_hierarchy_incoming" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/call_hierarchy/incoming",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_call_hierarchy_outgoing" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/call_hierarchy/outgoing",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_lens" => {
                            match lsp_file_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/code_lens",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_lens_resolve" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/code_lens/resolve",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_prepare_rename" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/prepare_rename",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_document_links" => {
                            match lsp_file_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/document_links",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_document_link_resolve" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/document_links/resolve",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_semantic_tokens_full" => {
                            match lsp_file_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/semantic_tokens/full",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_semantic_tokens_delta" => {
                            match lsp_semantic_tokens_delta_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_folding_ranges" => {
                            match lsp_file_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/folding_ranges",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_linked_editing_range" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/linked_editing_range",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_type_hierarchy_prepare" => {
                            match lsp_pos_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/type_hierarchy/prepare",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_type_hierarchy_supertypes" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/type_hierarchy/supertypes",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_type_hierarchy_subtypes" => {
                            match lsp_item_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/type_hierarchy/subtypes",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_actions_by_diagnostic_plan" => {
                            match lsp_code_actions_by_diagnostic_plan_call(
                                &client,
                                &daemon_url,
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_execute_command" => {
                            match lsp_execute_command_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/execute_command",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_execute_command_plan" => {
                            match lsp_execute_command_plan_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_document_symbols" => {
                            match lsp_file_call(
                                &client,
                                &daemon_url,
                                "/api/lsp/document_symbols",
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_workspace_symbols" => {
                            match lsp_workspace_symbols_call(&client, &daemon_url, &arguments).await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_workspace_symbol_resolve" => {
                            match lsp_workspace_symbol_resolve_call(
                                &client,
                                &daemon_url,
                                &arguments,
                            )
                            .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_actions" => {
                            match lsp_code_actions_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_rename_plan" => {
                            match lsp_rename_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_format_plan" => {
                            match lsp_format_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_organize_imports_plan" => {
                            match lsp_organize_imports_plan_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "lsp_code_action_plan" => {
                            match lsp_code_action_plan_call(&client, &daemon_url, &arguments).await
                            {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "list_edit_plans" => {
                            match list_edit_plans_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "get_edit_plan" => {
                            match get_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "apply_edit_plan" => {
                            match apply_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
                        "discard_edit_plan" => {
                            match discard_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.clone(), tool_ok(val)),
                                Err(e) => ok(id.clone(), tool_err(e)),
                            }
                        }
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
