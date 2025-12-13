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

    let daemon_url = std::env::var("CONTEXT_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:4399".to_string());

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
                        match list_workspaces(&daemon_url).await {
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

async fn list_workspaces(daemon_url: &str) -> Result<Value> {
    let url = format!("{}/api/workspaces", daemon_url.trim_end_matches('/'));
    let res = reqwest::get(url).await?.error_for_status()?;
    Ok(res.json::<Value>().await?)
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

