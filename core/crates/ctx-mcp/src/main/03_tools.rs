#[path = "03_tools/web_sessions.rs"]
mod web_sessions;

fn tool_ok(mut val: Value) -> Value {
    scrub_internal_fields(&mut val);
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

fn extract_error_message(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            return Some(error.to_string());
        }
    }
    Some(trimmed.to_string())
}

#[derive(Clone)]
struct ResolvedDaemonAccess {
    daemon_url: String,
    auth_token: String,
}

fn explicit_mcp_token() -> Result<Option<String>> {
    let Some(token) = ctx_env_opt("MCP_TOKEN") else {
        return Ok(None);
    };
    let trimmed = token.trim();
    if trimmed.is_empty() {
        bail!("CTX_MCP_TOKEN is empty");
    }
    Ok(Some(trimmed.to_string()))
}

fn resolve_daemon_access() -> Result<ResolvedDaemonAccess> {
    let daemon_url = ctx_env_opt("DAEMON_URL")
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .context(
            "missing daemon URL: CTX_DAEMON_URL must be set by the daemon for agent MCP tools",
        )?;
    let auth_token = explicit_mcp_token()?.context(
        "missing scoped ctx-mcp token: CTX_MCP_TOKEN must be set by the daemon for agent MCP tools",
    )?;
    reqwest::Url::parse(&daemon_url).with_context(|| format!("invalid daemon URL: {daemon_url}"))?;
    Ok(ResolvedDaemonAccess {
        daemon_url,
        auth_token,
    })
}

async fn daemon_get_json(client: &reqwest::Client, _daemon_url: &str, path: &str) -> Result<Value> {
    let access = resolve_daemon_access()?;
    let url = format!("{}{}", access.daemon_url.trim_end_matches('/'), path);
    let mut req = client.get(url);
    req = req.bearer_auth(access.auth_token);
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        let detail = extract_error_message(&text).unwrap_or_else(|| "unknown error".to_string());
        bail!(
            "HTTP {} {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            detail
        );
    }
    let value = serde_json::from_str(&text)
        .with_context(|| format!("parsing JSON response from {path}"))?;
    Ok(value)
}

async fn daemon_post_json(
    client: &reqwest::Client,
    _daemon_url: &str,
    path: &str,
    body: &Value,
) -> Result<Value> {
    let access = resolve_daemon_access()?;
    let url = format!("{}{}", access.daemon_url.trim_end_matches('/'), path);
    let mut req = client.post(url);
    req = req.bearer_auth(access.auth_token);
    let res = req.json(body).send().await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        let detail = extract_error_message(&text).unwrap_or_else(|| "unknown error".to_string());
        bail!(
            "HTTP {} {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            detail
        );
    }
    let value = serde_json::from_str(&text)
        .with_context(|| format!("parsing JSON response from {path}"))?;
    Ok(value)
}

fn tool_call_id_from_params(params: &Value) -> Option<String> {
    let meta = params.get("_meta").or_else(|| params.get("meta"));
    let args = params.get("arguments");
    let direct = meta
        .and_then(|m| {
            m.get("toolCallId")
                .or_else(|| m.get("tool_call_id"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            params
                .get("toolCallId")
                .or_else(|| params.get("tool_call_id"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            args.and_then(|value| {
                value
                    .get("toolCallId")
                    .or_else(|| value.get("tool_call_id"))
            })
            .and_then(|v| v.as_str())
        });
    direct
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

async fn merge_queue_submit_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id =
        ctx_env_opt("SESSION_ID").context("missing session context for merge queue submit")?;
    let worktree_id = ctx_env_opt("WORKTREE_ID");
    let target_branch = args
        .get("target_branch")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());

    let mut body = json!({});
    if let Some(obj) = body.as_object_mut() {
        obj.insert("session_id".to_string(), Value::String(session_id));
    }
    if let Some(worktree_id) = worktree_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("worktree_id".to_string(), Value::String(worktree_id));
        }
    }
    if let Some(target_branch) = target_branch {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("target_branch".to_string(), Value::String(target_branch));
        }
    }
    if let Some(message) = message {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("message".to_string(), Value::String(message));
        }
    }

    let mut response =
        daemon_post_json(client, daemon_url, "/api/merge-queue/entries", &body).await?;
    if let Some(obj) = response.as_object_mut() {
        obj.remove("id");
    }
    Ok(response)
}

async fn spawn_agent_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/spawn_agent");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn send_input_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let parent_session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{parent_session_id}/send_input");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn archive_agent_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/archive_agent");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn list_agents_call(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/list_agents");
    daemon_get_json(client, daemon_url, &path).await
}

async fn get_agent_call(client: &reqwest::Client, daemon_url: &str, args: &Value) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/get_agent");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn wait_agent_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/wait_agent");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn interrupt_agent_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{session_id}/interrupt_agent");
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn set_artifacts(
    client: &reqwest::Client,
    daemon_url: &str,
    session_id: &str,
    artifacts: Vec<Value>,
) -> Result<Value> {
    let path = format!("/api/sessions/{session_id}/artifacts");
    daemon_post_json(
        client,
        daemon_url,
        &path,
        &json!({ "artifacts": artifacts }),
    )
    .await
}

#[cfg(test)]
#[path = "03_tools/tests.rs"]
mod tests;

#[cfg(all(test, feature = "fuzz_tests"))]
#[path = "03_tools/fuzz_tests.rs"]
mod fuzz_tests;
