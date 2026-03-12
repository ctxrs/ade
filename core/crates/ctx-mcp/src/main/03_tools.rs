#[path = "03_tools/lsp.rs"]
mod lsp;

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

    let path = format!("/api/mcp/sessions/{}/oracle", session_id);
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

async fn agent_init_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let agents = args
        .get("agents")
        .and_then(|v| v.as_array())
        .context("missing agents")?;
    let worktree = args
        .get("worktree")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .context("missing worktree")?;
    let tool_call_id = args
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let path = format!("/api/mcp/sessions/{}/subagent_init", session_id);
    let mut body = json!({ "agents": agents, "worktree": worktree });
    if let Some(tool_call_id) = tool_call_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
        }
    }
    let mut response = daemon_post_json(client, daemon_url, &path, &body).await?;
    if let Some(obj) = response.as_object_mut() {
        if let Some(results) = obj.get_mut("results") {
            map_subagent_results(results);
        }
    }
    Ok(response)
}

async fn agent_reply_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let parent_session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .context("missing label")?;
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .context("missing prompt")?;
    let path = format!("/api/mcp/sessions/{}/subagent_reply", parent_session_id);
    let response = daemon_post_json(
        client,
        daemon_url,
        &path,
        &json!({ "label": label, "prompt": prompt }),
    )
    .await?;
    Ok(response)
}

async fn subagent_list_call(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{}/subagent_list", session_id);
    daemon_get_json(client, daemon_url, &path).await
}

async fn subagent_wait_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{}/subagent_wait", session_id);
    let mut response = daemon_post_json(client, daemon_url, &path, args).await?;
    if let Some(obj) = response.as_object_mut() {
        if let Some(results) = obj.get_mut("results") {
            map_subagent_results(results);
        }
    }
    Ok(response)
}

async fn subagent_interrupt_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/mcp/sessions/{}/subagent_interrupt", session_id);
    daemon_post_json(client, daemon_url, &path, args).await
}

async fn set_artifacts(
    client: &reqwest::Client,
    daemon_url: &str,
    session_id: &str,
    artifacts: Vec<Value>,
) -> Result<Value> {
    let path = format!("/api/sessions/{}/artifacts", session_id);
    daemon_post_json(
        client,
        daemon_url,
        &path,
        &json!({ "artifacts": artifacts }),
    )
    .await
}

async fn list_edit_plans_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let _ = arguments;
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let snapshot = daemon_get_json(
        client,
        daemon_url,
        &format!(
            "/api/sessions/{}/snapshot?limit=1&include_events=0",
            session_id
        ),
    )
    .await?;
    let worktree_id = snapshot
        .get("summary")
        .and_then(|v| v.get("session"))
        .and_then(|v| v.get("worktree_id"))
        .or_else(|| {
            snapshot
                .get("head")
                .and_then(|v| v.get("session"))
                .and_then(|v| v.get("worktree_id"))
        })
        .and_then(|v| v.as_str().or_else(|| v.get("0").and_then(|x| x.as_str())))
        .context("missing worktree context")?;
    let mut response = daemon_get_json(
        client,
        daemon_url,
        &format!("/api/worktrees/{}/edit_plans", worktree_id),
    )
    .await?;
    map_edit_plan_summaries(&mut response);
    Ok(response)
}

async fn get_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let edit_plan_id = arguments
        .get("edit_plan_id")
        .and_then(|v| v.as_str())
        .context("missing edit_plan_id")?;
    let plan_id = internal_plan_id_for_edit_plan(edit_plan_id).context("unknown edit_plan_id")?;
    let mut response =
        daemon_get_json(client, daemon_url, &format!("/api/edit_plans/{}", plan_id)).await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

async fn apply_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let edit_plan_id = arguments
        .get("edit_plan_id")
        .and_then(|v| v.as_str())
        .context("missing edit_plan_id")?;
    let plan_id = internal_plan_id_for_edit_plan(edit_plan_id).context("unknown edit_plan_id")?;
    let action = arguments
        .get("action")
        .and_then(|v| v.as_str())
        .context("missing action")?;
    let patch = if let Some(p) = arguments.get("patch").and_then(|v| v.as_str()) {
        p.to_string()
    } else {
        let plan =
            daemon_get_json(client, daemon_url, &format!("/api/edit_plans/{}", plan_id)).await?;
        plan.get("diff")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let body = json!({
        "action": action,
        "patch": patch
    });
    let mut response = daemon_post_json(
        client,
        daemon_url,
        &format!("/api/edit_plans/{}/apply", plan_id),
        &body,
    )
    .await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

async fn discard_edit_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let edit_plan_id = arguments
        .get("edit_plan_id")
        .and_then(|v| v.as_str())
        .context("missing edit_plan_id")?;
    let plan_id = internal_plan_id_for_edit_plan(edit_plan_id).context("unknown edit_plan_id")?;
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
    let mut response = res.json::<Value>().await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

#[allow(dead_code)]
async fn session_create_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let kind = arguments.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if kind != "web" {
        anyhow::bail!("unsupported session kind: {}", kind);
    }
    let target = arguments.get("target").context("missing target")?;
    let url = target
        .get("url")
        .and_then(|v| v.as_str())
        .context("missing target.url")?;
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let viewport = arguments.get("viewport");
    let fps = arguments.get("fps");

    let mut body = serde_json::Map::new();
    body.insert("url".to_string(), json!(url));
    body.insert("session_id".to_string(), json!(session_id));
    if let Some(viewport) = viewport {
        body.insert("viewport".to_string(), viewport.clone());
    }
    if let Some(fps) = fps {
        body.insert("fps".to_string(), fps.clone());
    }

    let mut response = daemon_post_json(
        client,
        daemon_url,
        "/api/sessions/web",
        &Value::Object(body),
    )
    .await?;
    map_interactive_session(&mut response);
    Ok(response)
}

#[allow(dead_code)]
async fn session_list_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    if let Some(kind) = arguments.get("kind").and_then(|v| v.as_str()) {
        if kind != "web" {
            return Ok(json!([]));
        }
    }
    let mut response = daemon_get_json(client, daemon_url, "/api/sessions/web").await?;
    map_interactive_sessions(&mut response);
    Ok(response)
}

#[allow(dead_code)]
async fn session_info_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_ref = arguments
        .get("session_ref")
        .and_then(|v| v.as_str())
        .context("missing session_ref")?;
    let session_id = session_id_for_ref(session_ref).context("unknown session_ref")?;
    daemon_get_json(
        client,
        daemon_url,
        &format!("/api/sessions/web/{}", session_id),
    )
    .await
    .map(|mut response| {
        map_interactive_session(&mut response);
        response
    })
}

#[allow(dead_code)]
async fn session_run_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
    is_eval: bool,
) -> Result<Value> {
    let session_ref = arguments
        .get("session_ref")
        .and_then(|v| v.as_str())
        .context("missing session_ref")?;
    let session_id = session_id_for_ref(session_ref).context("unknown session_ref")?;
    let mut body = serde_json::Map::new();
    if let Some(code) = arguments.get("code") {
        body.insert("code".to_string(), code.clone());
    }
    if let Some(script_path) = arguments.get("script_path") {
        body.insert("script_path".to_string(), script_path.clone());
    }
    if let Some(timeout_ms) = arguments.get("timeout_ms") {
        body.insert("timeout_ms".to_string(), timeout_ms.clone());
    }
    let endpoint = if is_eval { "eval" } else { "run" };
    daemon_post_json(
        client,
        daemon_url,
        &format!("/api/sessions/web/{}/{}", session_id, endpoint),
        &Value::Object(body),
    )
    .await
}

#[allow(dead_code)]
async fn session_close_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_ref = arguments
        .get("session_ref")
        .and_then(|v| v.as_str())
        .context("missing session_ref")?;
    let session_id = session_id_for_ref(session_ref).context("unknown session_ref")?;
    let url = format!(
        "{}/api/sessions/web/{}/close",
        daemon_url.trim_end_matches('/'),
        session_id
    );
    let mut req = client.post(url);
    if let Some(token) = bearer_token() {
        req = req.bearer_auth(token);
    }
    let res = req.send().await?.error_for_status()?;
    if res.status().as_u16() == 204 {
        return Ok(json!({"closed": true}));
    }
    Ok(res.json::<Value>().await?)
}

#[cfg(all(test, feature = "fuzz_tests"))]
mod fuzz_tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use serde_json::{Map, Value};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    const ITERATIONS: usize = 200;
    const MAX_DEPTH: u8 = 3;

    fn random_string(rng: &mut StdRng, max_len: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_-";
        let len = rng.gen_range(1..=max_len);
        (0..len)
            .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
            .collect()
    }

    fn random_value(rng: &mut StdRng, depth: u8) -> Value {
        if depth == 0 {
            return match rng.gen_range(0..4) {
                0 => Value::String(random_string(rng, 12)),
                1 => Value::Number(rng.gen_range(0..=9999).into()),
                2 => Value::Bool(rng.gen_bool(0.5)),
                _ => Value::Null,
            };
        }

        match rng.gen_range(0..4) {
            0 => Value::String(random_string(rng, 24)),
            1 => {
                let mut map = Map::new();
                let entries = rng.gen_range(0..=3);
                for _ in 0..entries {
                    map.insert(random_string(rng, 10), random_value(rng, depth - 1));
                }
                Value::Object(map)
            }
            2 => {
                let items = rng.gen_range(0..=4);
                Value::Array((0..items).map(|_| random_value(rng, depth - 1)).collect())
            }
            _ => Value::Number(rng.gen_range(0..=9999).into()),
        }
    }

    fn random_lsp_arguments(rng: &mut StdRng) -> Value {
        if rng.gen_bool(0.2) {
            return random_value(rng, MAX_DEPTH);
        }

        let mut map = Map::new();
        if rng.gen_bool(0.7) {
            map.insert("path".into(), Value::String(random_string(rng, 32)));
        }
        if rng.gen_bool(0.6) {
            map.insert("query".into(), Value::String(random_string(rng, 20)));
        }
        if rng.gen_bool(0.6) {
            map.insert(
                "item".into(),
                random_value(rng, MAX_DEPTH.saturating_sub(1)),
            );
        }
        if rng.gen_bool(0.5) {
            map.insert("session_id".into(), Value::String(random_string(rng, 12)));
        }
        if rng.gen_bool(0.3) {
            map.insert(
                "root_path".into(),
                Value::String(format!("/tmp/{}", random_string(rng, 8))),
            );
        }

        let extras = rng.gen_range(0..=3);
        for _ in 0..extras {
            map.insert(random_string(rng, 8), random_value(rng, MAX_DEPTH));
        }

        Value::Object(map)
    }

    #[test]
    fn fuzz_lsp_argument_body_builders_do_not_panic() {
        let mut rng = StdRng::seed_from_u64(0xC0DE_2025);

        for idx in 0..ITERATIONS {
            let arguments = random_lsp_arguments(&mut rng);

            let result = catch_unwind(AssertUnwindSafe(|| lsp_file_call_body(&arguments)));
            assert!(result.is_ok(), "panic in lsp_file_call_body: {idx}");

            let result = catch_unwind(AssertUnwindSafe(|| lsp_workspace_symbols_body(&arguments)));
            assert!(result.is_ok(), "panic in lsp_workspace_symbols_body: {idx}");

            let result = catch_unwind(AssertUnwindSafe(|| {
                lsp_workspace_symbol_resolve_body(&arguments)
            }));
            assert!(
                result.is_ok(),
                "panic in lsp_workspace_symbol_resolve_body: {idx}"
            );
        }
    }
}
