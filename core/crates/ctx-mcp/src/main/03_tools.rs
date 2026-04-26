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

fn bearer_token() -> Option<String> {
    ctx_env_opt("AUTH_TOKEN")
}

async fn daemon_get_json(client: &reqwest::Client, daemon_url: &str, path: &str) -> Result<Value> {
    let url = format!("{}{}", daemon_url.trim_end_matches('/'), path);
    let mut req = client.get(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
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
    daemon_url: &str,
    path: &str,
    body: &Value,
) -> Result<Value> {
    let url = format!("{}{}", daemon_url.trim_end_matches('/'), path);
    let mut req = client.post(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
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

async fn list_workspaces(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    let response = daemon_get_json(client, daemon_url, "/api/workspaces").await?;
    let Some(items) = response.as_array() else {
        return Ok(response);
    };
    let mapped: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let name = obj.get("name").cloned().unwrap_or(Value::Null);
            let root_path = obj.get("root_path").cloned().unwrap_or(Value::Null);
            Some(json!({
                "name": name,
                "root_path": root_path,
            }))
        })
        .collect();
    Ok(Value::Array(mapped))
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() || current.join(".jj").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn resolve_worktree_root() -> Option<String> {
    if let Some(root) = ctx_env_opt("WORKTREE_ROOT") {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let cwd = std::env::current_dir().ok()?;
    let root = find_repo_root(&cwd)?;
    Some(root.to_string_lossy().to_string())
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
    } else if let Some(worktree_root) = resolve_worktree_root() {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("worktree_root".to_string(), Value::String(worktree_root));
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

async fn oracle_call(client: &reqwest::Client, daemon_url: &str, args: &Value) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    if args.get("prompt").is_some() {
        bail!("inline prompt is not supported; use prompt_path");
    }

    let prompt_path = args
        .get("prompt_path")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .context("missing prompt_path")?;
    let response_path = args
        .get("response_path")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty());

    let cwd = std::env::current_dir().context("resolve current dir")?;
    let resolve_path = |path: &str| -> PathBuf {
        let candidate = PathBuf::from(path);
        if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        }
    };
    let path_to_string = |path: &Path| path.to_string_lossy().to_string();

    let prompt_path = resolve_path(prompt_path);
    let prompt_bytes = tokio::fs::read(&prompt_path)
        .await
        .with_context(|| format!("reading prompt_path {}", prompt_path.display()))?;
    let prompt = String::from_utf8(prompt_bytes.clone()).context("prompt_path must be utf-8")?;
    if prompt.trim().is_empty() {
        bail!("prompt file is empty");
    }

    let oracle_id = {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("{timestamp}-{session_id}")
    };
    let oracle_dir = cwd.join(".ctx/tmp/oracle").join(&oracle_id);
    tokio::fs::create_dir_all(&oracle_dir)
        .await
        .context("creating oracle temp directory")?;
    let input_copy_path = oracle_dir.join("input.md");
    tokio::fs::write(&input_copy_path, &prompt_bytes)
        .await
        .context("writing oracle prompt copy")?;

    let response_path = response_path
        .map(resolve_path)
        .unwrap_or_else(|| oracle_dir.join("output.md"));

    let mut body = json!({ "prompt": prompt });
    if let Some(model) = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("model".to_string(), Value::String(model.to_string()));
        }
    }
    if let Some(reasoning_effort) = args
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "reasoning_effort".to_string(),
                Value::String(reasoning_effort.to_string()),
            );
        }
    }
    if let Some(max_output_tokens) = args.get("max_output_tokens").and_then(|v| v.as_u64()) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_output_tokens.into()),
            );
        }
    }
    if let Some(timeout_ms) = args.get("timeout_ms").and_then(|v| v.as_u64()) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
        }
    }

    let path = format!("/api/mcp/sessions/{session_id}/oracle");
    let response = daemon_post_json(client, daemon_url, &path, &body).await?;
    let response_text = response
        .get("text")
        .and_then(|v| v.as_str())
        .context("missing oracle response text")?;
    tokio::fs::write(&response_path, response_text.as_bytes())
        .await
        .with_context(|| format!("writing oracle response {}", response_path.display()))?;

    let response_bytes = response_text.len() as u64;
    let prompt_bytes_len = prompt_bytes.len() as u64;

    Ok(json!({
        "prompt_path": path_to_string(&prompt_path),
        "input_copy_path": path_to_string(&input_copy_path),
        "response_path": path_to_string(&response_path),
        "prompt_bytes": prompt_bytes_len,
        "response_bytes": response_bytes
    }))
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

async fn get_agent_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
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
