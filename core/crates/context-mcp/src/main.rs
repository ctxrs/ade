use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser, Debug)]
#[command(name = "context-mcp")]
struct Cli {
    /// Run as an MCP stdio server (newline-delimited JSON-RPC).
    #[arg(long)]
    stdio: bool,
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

    let daemon_url =
        std::env::var("CONTEXT_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:4399".to_string());
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
        let id = msg.get("id").cloned();

        // Notifications have no response.
        if id.is_none() {
            continue;
        }

        let response = match method {
            "ping" => ok(id.unwrap(), json!({})),
            "initialize" => {
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let protocol_version = params
                    .get("protocolVersion")
                    .cloned()
                    .unwrap_or_else(|| json!("2025-11-25"));
                ok(
                    id.unwrap(),
                    json!({
                        "protocolVersion": protocol_version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": "context-mcp",
                            "title": "Context MCP",
                            "version": env!("CARGO_PKG_VERSION"),
                            "description": "Context daemon tools (bridge)"
                        }
                    }),
                )
            }
            "tools/list" => ok(
                id.unwrap(),
                json!({
                    "tools": [
                        {
                            "name": "context.ping",
                            "title": "Context Ping",
                            "description": "Returns ok=true if Context MCP is reachable.",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "context.list_workspaces",
                            "title": "List Workspaces",
                            "description": "Lists Context workspaces via the Context daemon HTTP API.",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "context.lsp_status",
                            "title": "LSP Status",
                            "description": "Returns the daemon LSP configuration and server availability (installed/missing).",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "context.lsp_diagnostics",
                            "title": "LSP Diagnostics",
                            "description": "Returns language-server diagnostics for a file in the current session worktree (requires daemon LSP enabled).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string", "description": "File path (relative to session worktree, or absolute)." },
                                    "root_path": { "type": "string", "description": "Optional explicit root path when no session_id is available." }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_definition",
                            "title": "LSP Definition",
                            "description": "Returns the definition location at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" },
                                    "line": { "type": "integer", "minimum": 0 },
                                    "character": { "type": "integer", "minimum": 0 }
                                },
                                "required": ["path", "line", "character"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_references",
                            "title": "LSP References",
                            "description": "Returns references at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" },
                                    "line": { "type": "integer", "minimum": 0 },
                                    "character": { "type": "integer", "minimum": 0 },
                                    "include_declaration": { "type": "boolean" }
                                },
                                "required": ["path", "line", "character"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_document_symbols",
                            "title": "LSP Document Symbols",
                            "description": "Returns document symbols for a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_workspace_symbols",
                            "title": "LSP Workspace Symbols",
                            "description": "Returns workspace symbols for a query.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "root_path": { "type": "string" },
                                    "query": { "type": "string" }
                                },
                                "required": ["query"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_code_actions",
                            "title": "LSP Code Actions",
                            "description": "Returns code actions for a selection.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" },
                                    "start_line": { "type": "integer", "minimum": 0 },
                                    "start_character": { "type": "integer", "minimum": 0 },
                                    "end_line": { "type": "integer", "minimum": 0 },
                                    "end_character": { "type": "integer", "minimum": 0 }
                                },
                                "required": ["path", "start_line", "start_character", "end_line", "end_character"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_rename_plan",
                            "title": "LSP Rename (Plan)",
                            "description": "Creates an edit plan for an LSP rename operation (requires daemon LSP edit plans enabled).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" },
                                    "line": { "type": "integer", "minimum": 0 },
                                    "character": { "type": "integer", "minimum": 0 },
                                    "new_name": { "type": "string" }
                                },
                                "required": ["path", "line", "character", "new_name"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_format_plan",
                            "title": "LSP Format (Plan)",
                            "description": "Creates an edit plan for formatting a document.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_organize_imports_plan",
                            "title": "LSP Organize Imports (Plan)",
                            "description": "Creates an edit plan for organizing imports (typically via LSP code actions).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.lsp_code_action_plan",
                            "title": "LSP Code Action (Plan)",
                            "description": "Creates an edit plan from an LSP CodeAction JSON object (expects a CodeAction with an edit).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." },
                                    "action": { "type": "object" }
                                },
                                "required": ["action"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.list_edit_plans",
                            "title": "List Edit Plans",
                            "description": "Lists pending edit plans for a track (or for the current session's track).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "track_id": { "type": "string" },
                                    "session_id": { "type": "string", "description": "Optional Context session id (defaults to $CONTEXT_SESSION_ID)." }
                                },
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.get_edit_plan",
                            "title": "Get Edit Plan",
                            "description": "Fetches an edit plan summary by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "plan_id": { "type": "string" }
                                },
                                "required": ["plan_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.apply_edit_plan",
                            "title": "Apply Edit Plan Patch",
                            "description": "Applies or rejects a patch from an edit plan. If patch is omitted, applies the entire remaining plan diff.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "plan_id": { "type": "string" },
                                    "action": { "type": "string", "enum": ["accept", "reject"] },
                                    "patch": { "type": "string" }
                                },
                                "required": ["plan_id", "action"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "context.discard_edit_plan",
                            "title": "Discard Edit Plan",
                            "description": "Discards an edit plan by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "plan_id": { "type": "string" }
                                },
                                "required": ["plan_id"],
                                "additionalProperties": false
                            }
                        }
                    ]
                }),
            ),
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                match name {
                    "context.ping" => ok(
                        id.unwrap(),
                        json!({
                            "content": [{"type":"text","text": "{\"ok\":true}"}],
                            "isError": false
                        }),
                    ),
                    "context.list_workspaces" => {
                        let _ = arguments; // currently unused
                        match list_workspaces(&client, &daemon_url).await {
                            Ok(val) => ok(
                                id.unwrap(),
                                json!({
                                    "content": [{"type":"text","text": serde_json::to_string_pretty(&val).unwrap_or_else(|_| "[]".into())}],
                                    "isError": false
                                }),
                            ),
                            Err(e) => ok(
                                id.unwrap(),
                                json!({
                                    "content": [{"type":"text","text": format!("error: {e}")}],
                                    "isError": true
                                }),
                            ),
                        }
                    }
                    "context.lsp_status" => {
                        let _ = arguments;
                        match lsp_status(&client, &daemon_url).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_diagnostics" => {
                        let path = arguments
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if path.is_empty() {
                            error(
                                id.unwrap(),
                                -32602,
                                "Invalid params",
                                Some(json!({"missing":"path"})),
                            )
                        } else {
                            let root_path = arguments
                                .get("root_path")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let session_id = arguments
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok());

                            match lsp_diagnostics(&client, &daemon_url, session_id, root_path, path).await {
                                Ok(val) => ok(
                                    id.unwrap(),
                                    json!({
                                        "content": [{"type":"text","text": serde_json::to_string_pretty(&val).unwrap_or_else(|_| "[]".into())}],
                                        "isError": false
                                    }),
                                ),
                                Err(e) => ok(
                                    id.unwrap(),
                                    json!({
                                        "content": [{"type":"text","text": format!("error: {e}")}],
                                        "isError": true
                                    }),
                                ),
                            }
                        }
                    }
                    "context.lsp_definition" => {
                        match lsp_pos_call(&client, &daemon_url, "/api/lsp/definition", &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_references" => {
                        match lsp_pos_call(&client, &daemon_url, "/api/lsp/references", &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_document_symbols" => {
                        match lsp_file_call(&client, &daemon_url, "/api/lsp/document_symbols", &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_workspace_symbols" => {
                        match lsp_workspace_symbols_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_code_actions" => {
                        match lsp_code_actions_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_rename_plan" => {
                        match lsp_rename_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_format_plan" => {
                        match lsp_format_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_organize_imports_plan" => {
                        match lsp_organize_imports_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.lsp_code_action_plan" => {
                        match lsp_code_action_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.list_edit_plans" => {
                        match list_edit_plans_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.get_edit_plan" => {
                        match get_edit_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.apply_edit_plan" => {
                        match apply_edit_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    "context.discard_edit_plan" => {
                        match discard_edit_plan_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        }
                    }
                    _ => error(id.unwrap(), -32601, "Method not found", Some(json!({"tool": name}))),
                }
            }
            _ => error(id.unwrap(), -32601, "Method not found", Some(json!({"method": method}))),
        };

        let line = serde_json::to_string(&response).context("serializing MCP response")?;
        out.write_all(line.as_bytes()).await?;
        out.write_all(b"\n").await?;
        out.flush().await?;
    }

    Ok(())
}

fn tool_ok(val: Value) -> Value {
    json!({
        "content": [{"type":"text","text": serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".into())}],
        "isError": false
    })
}

fn tool_err(e: anyhow::Error) -> Value {
    json!({
        "content": [{"type":"text","text": format!("error: {e}")}],
        "isError": true
    })
}

fn bearer_token() -> Option<String> {
    std::env::var("CONTEXT_DESKTOP_TOKEN")
        .or_else(|_| std::env::var("CONTEXT_DAEMON_TOKEN"))
        .or_else(|_| std::env::var("CONTEXT_AUTH_TOKEN"))
        .ok()
}

async fn daemon_get_json(client: &reqwest::Client, daemon_url: &str, path: &str) -> Result<Value> {
    let url = format!("{}{}", daemon_url.trim_end_matches('/'), path);
    let mut req = client.get(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
    let res = req.send().await?.error_for_status()?;
    Ok(res.json::<Value>().await?)
}

async fn daemon_post_json(
    client: &reqwest::Client,
    daemon_url: &str,
    path: &str,
    body: &Value,
) -> Result<Value> {
    let url = format!("{}{}", daemon_url.trim_end_matches('/'), path);
    let mut req = client.post(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
    let res = req.json(body).send().await?.error_for_status()?;
    Ok(res.json::<Value>().await?)
}

async fn list_workspaces(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/workspaces").await
}

async fn lsp_status(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/lsp/status").await
}

async fn lsp_diagnostics(
    client: &reqwest::Client,
    daemon_url: &str,
    session_id: Option<String>,
    root_path: Option<String>,
    path: String,
) -> Result<Value> {
    let body = json!({
        "session_id": session_id,
        "root_path": root_path,
        "path": path,
    });
    daemon_post_json(client, daemon_url, "/api/lsp/diagnostics", &body).await
}

fn ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": data
        }
    })
}

async fn lsp_file_call(
    client: &reqwest::Client,
    daemon_url: &str,
    path: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok());
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let file_path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "root_path": root_path,
        "path": file_path,
    });
    daemon_post_json(client, daemon_url, path, &body).await
}

async fn lsp_pos_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
    arguments: &Value,
) -> Result<Value> {
    let line = arguments.get("line").and_then(|v| v.as_u64()).context("missing line")?;
    let character = arguments
        .get("character")
        .and_then(|v| v.as_u64())
        .context("missing character")?;
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("line".into(), json!(line));
        obj.insert("character".into(), json!(character));
        if let Some(include) = arguments.get("include_declaration") {
            obj.insert("include_declaration".into(), include.clone());
        }
    }
    daemon_post_json(client, daemon_url, endpoint, &body).await
}

fn lsp_file_call_body(arguments: &Value) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok());
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let file_path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    Ok(json!({
        "session_id": session_id,
        "root_path": root_path,
        "path": file_path
    }))
}

async fn lsp_workspace_symbols_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok());
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .context("missing query")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "root_path": root_path,
        "query": query
    });
    daemon_post_json(client, daemon_url, "/api/lsp/workspace_symbols", &body).await
}

async fn lsp_code_actions_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let start_line = arguments
        .get("start_line")
        .and_then(|v| v.as_u64())
        .context("missing start_line")?;
    let start_character = arguments
        .get("start_character")
        .and_then(|v| v.as_u64())
        .context("missing start_character")?;
    let end_line = arguments
        .get("end_line")
        .and_then(|v| v.as_u64())
        .context("missing end_line")?;
    let end_character = arguments
        .get("end_character")
        .and_then(|v| v.as_u64())
        .context("missing end_character")?;
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("start_line".into(), json!(start_line));
        obj.insert("start_character".into(), json!(start_character));
        obj.insert("end_line".into(), json!(end_line));
        obj.insert("end_character".into(), json!(end_character));
    }
    daemon_post_json(client, daemon_url, "/api/lsp/code_actions", &body).await
}

async fn lsp_rename_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok())
        .context("missing session_id (set CONTEXT_SESSION_ID or pass session_id)")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let line = arguments.get("line").and_then(|v| v.as_u64()).context("missing line")?;
    let character = arguments
        .get("character")
        .and_then(|v| v.as_u64())
        .context("missing character")?;
    let new_name = arguments
        .get("new_name")
        .and_then(|v| v.as_str())
        .context("missing new_name")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "path": path,
        "line": line,
        "character": character,
        "new_name": new_name
    });
    daemon_post_json(client, daemon_url, "/api/lsp/rename/plan", &body).await
}

async fn lsp_format_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok())
        .context("missing session_id (set CONTEXT_SESSION_ID or pass session_id)")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "path": path
    });
    daemon_post_json(client, daemon_url, "/api/lsp/format/plan", &body).await
}

async fn lsp_organize_imports_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok())
        .context("missing session_id (set CONTEXT_SESSION_ID or pass session_id)")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "path": path
    });
    daemon_post_json(client, daemon_url, "/api/lsp/organize_imports/plan", &body).await
}

async fn lsp_code_action_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok())
        .context("missing session_id (set CONTEXT_SESSION_ID or pass session_id)")?;
    let action = arguments.get("action").cloned().context("missing action")?;
    let body = json!({
        "session_id": session_id,
        "action": action
    });
    daemon_post_json(client, daemon_url, "/api/lsp/code_actions/plan", &body).await
}

async fn list_edit_plans_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let track_id = arguments.get("track_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    let track_id = if let Some(tid) = track_id {
        tid
    } else {
        let session_id = arguments
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| std::env::var("CONTEXT_SESSION_ID").ok())
            .context("missing track_id and session_id")?;
        let sess = daemon_get_json(
            client,
            daemon_url,
            &format!("/api/sessions/{}", session_id),
        )
        .await?;
        let tid = sess
            .get("track_id")
            .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str())))
            .context("session missing track_id")?;
        tid.to_string()
    };
    daemon_get_json(
        client,
        daemon_url,
        &format!("/api/tracks/{}/edit_plans", track_id),
    )
    .await
}

async fn get_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let plan_id = arguments
        .get("plan_id")
        .and_then(|v| v.as_str())
        .context("missing plan_id")?;
    daemon_get_json(
        client,
        daemon_url,
        &format!("/api/edit_plans/{}", plan_id),
    )
    .await
}

async fn apply_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let plan_id = arguments
        .get("plan_id")
        .and_then(|v| v.as_str())
        .context("missing plan_id")?;
    let action = arguments
        .get("action")
        .and_then(|v| v.as_str())
        .context("missing action")?;
    let patch = if let Some(p) = arguments.get("patch").and_then(|v| v.as_str()) {
        p.to_string()
    } else {
        let plan = daemon_get_json(
            client,
            daemon_url,
            &format!("/api/edit_plans/{}", plan_id),
        )
        .await?;
        plan.get("diff")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let body = json!({
        "action": action,
        "patch": patch
    });
    daemon_post_json(
        client,
        daemon_url,
        &format!("/api/edit_plans/{}/apply", plan_id),
        &body,
    )
    .await
}

async fn discard_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let plan_id = arguments
        .get("plan_id")
        .and_then(|v| v.as_str())
        .context("missing plan_id")?;
    let url = format!(
        "{}/api/edit_plans/{}/discard",
        daemon_url.trim_end_matches('/'),
        plan_id
    );
    let mut req = client.post(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
    let res = req.send().await?.error_for_status()?;
    if res.status().as_u16() == 204 {
        return Ok(json!({"discarded": true}));
    }
    Ok(res.json::<Value>().await?)
}
