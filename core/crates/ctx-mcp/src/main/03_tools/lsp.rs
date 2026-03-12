use super::*;

pub(crate) async fn lsp_status(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/lsp/status").await
}

pub(crate) async fn lsp_install_server(
    client: &reqwest::Client,
    daemon_url: &str,
    server_id: &str,
) -> Result<Value> {
    let server_id = urlencoding::encode(server_id);
    let path = format!("/api/lsp/servers/{server_id}/install");
    daemon_post_json(client, daemon_url, &path, &json!({})).await
}

pub(crate) async fn lsp_catalog_list(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/lsp/catalog").await
}

pub(crate) async fn lsp_catalog_install(
    client: &reqwest::Client,
    daemon_url: &str,
    catalog_id: &str,
) -> Result<Value> {
    let catalog_id = urlencoding::encode(catalog_id);
    let path = format!("/api/lsp/catalog/{catalog_id}/install");
    daemon_post_json(client, daemon_url, &path, &json!({})).await
}

pub(crate) async fn lsp_diagnostics(
    client: &reqwest::Client,
    daemon_url: &str,
    session_id: Option<String>,
    root_path: Option<String>,
    path: String,
) -> Result<Value> {
    if session_id.is_none() && root_path.is_none() {
        anyhow::bail!("missing session context");
    }
    let body = json!({
        "session_id": session_id,
        "root_path": root_path,
        "path": path,
    });
    daemon_post_json(client, daemon_url, "/api/lsp/diagnostics", &body).await
}

pub(crate) async fn lsp_file_call(
    client: &reqwest::Client,
    daemon_url: &str,
    path: &str,
    arguments: &Value,
) -> Result<Value> {
    let body = lsp_file_call_body(arguments)?;
    daemon_post_json(client, daemon_url, path, &body).await
}

pub(crate) async fn lsp_pos_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
    arguments: &Value,
) -> Result<Value> {
    let line = arguments
        .get("line")
        .and_then(|v| v.as_u64())
        .context("missing line")?;
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

pub(crate) async fn lsp_item_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
    arguments: &Value,
) -> Result<Value> {
    let item = arguments.get("item").cloned().context("missing item")?;
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("item".into(), item);
    }
    daemon_post_json(client, daemon_url, endpoint, &body).await
}

pub(crate) async fn lsp_range_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
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
    daemon_post_json(client, daemon_url, endpoint, &body).await
}

pub(crate) async fn lsp_selection_ranges_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
    arguments: &Value,
) -> Result<Value> {
    let positions = arguments
        .get("positions")
        .cloned()
        .context("missing positions")?;
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("positions".into(), positions);
    }
    daemon_post_json(client, daemon_url, endpoint, &body).await
}

pub(crate) async fn lsp_execute_command_call(
    client: &reqwest::Client,
    daemon_url: &str,
    endpoint: &str,
    arguments: &Value,
) -> Result<Value> {
    let command = arguments
        .get("command")
        .and_then(|v| v.as_str())
        .context("missing command")?
        .to_string();
    let args = arguments
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("command".into(), json!(command));
        obj.insert("arguments".into(), args);
    }
    daemon_post_json(client, daemon_url, endpoint, &body).await
}

pub(crate) async fn lsp_execute_command_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let mut response = lsp_execute_command_call(
        client,
        daemon_url,
        "/api/lsp/execute_command/plan",
        arguments,
    )
    .await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

pub(crate) fn lsp_file_call_body(arguments: &Value) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID");
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if session_id.is_none() && root_path.is_none() {
        anyhow::bail!("missing session context");
    }
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

pub(crate) fn lsp_workspace_symbols_body(arguments: &Value) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID");
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if session_id.is_none() && root_path.is_none() {
        anyhow::bail!("missing session context");
    }
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .context("missing query")?
        .to_string();
    Ok(json!({
        "session_id": session_id,
        "root_path": root_path,
        "query": query
    }))
}

pub(crate) fn lsp_workspace_symbol_resolve_body(arguments: &Value) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID");
    let root_path = arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if session_id.is_none() && root_path.is_none() {
        anyhow::bail!("missing session context");
    }
    let item = arguments.get("item").cloned().context("missing item")?;
    Ok(json!({
        "session_id": session_id,
        "root_path": root_path,
        "item": item,
    }))
}

pub(crate) async fn lsp_workspace_symbols_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let body = lsp_workspace_symbols_body(arguments)?;
    daemon_post_json(client, daemon_url, "/api/lsp/workspace_symbols", &body).await
}

pub(crate) async fn lsp_workspace_symbol_resolve_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let body = lsp_workspace_symbol_resolve_body(arguments)?;
    daemon_post_json(
        client,
        daemon_url,
        "/api/lsp/workspace_symbols/resolve",
        &body,
    )
    .await
}

pub(crate) async fn lsp_semantic_tokens_delta_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let previous_result_id = arguments
        .get("previous_result_id")
        .and_then(|v| v.as_str())
        .context("missing previous_result_id")?
        .to_string();
    let mut body = lsp_file_call_body(arguments)?;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("previous_result_id".into(), json!(previous_result_id));
    }
    daemon_post_json(client, daemon_url, "/api/lsp/semantic_tokens/delta", &body).await
}

pub(crate) async fn lsp_code_actions_call(
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

pub(crate) async fn lsp_rename_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let line = arguments
        .get("line")
        .and_then(|v| v.as_u64())
        .context("missing line")?;
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
    let mut response = daemon_post_json(client, daemon_url, "/api/lsp/rename/plan", &body).await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

pub(crate) async fn lsp_format_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "path": path
    });
    let mut response = daemon_post_json(client, daemon_url, "/api/lsp/format/plan", &body).await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

pub(crate) async fn lsp_organize_imports_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let body = json!({
        "session_id": session_id,
        "path": path
    });
    let mut response =
        daemon_post_json(client, daemon_url, "/api/lsp/organize_imports/plan", &body).await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

pub(crate) async fn lsp_code_action_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let action = arguments.get("action").cloned().context("missing action")?;
    let body = json!({
        "session_id": session_id,
        "action": action
    });
    let mut response =
        daemon_post_json(client, daemon_url, "/api/lsp/code_actions/plan", &body).await?;
    map_edit_plan_summary(&mut response);
    Ok(response)
}

pub(crate) async fn lsp_code_actions_by_diagnostic_plan_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .context("missing path")?
        .to_string();
    let diagnostic = arguments
        .get("diagnostic")
        .cloned()
        .context("missing diagnostic")?;
    let body = json!({
        "session_id": session_id,
        "path": path,
        "diagnostic": diagnostic
    });
    let mut response = daemon_post_json(
        client,
        daemon_url,
        "/api/lsp/code_actions/by_diagnostic/plan",
        &body,
    )
    .await?;
    map_edit_plan_summaries(&mut response);
    Ok(response)
}
