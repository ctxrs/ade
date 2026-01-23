#![recursion_limit = "256"]

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
#[command(name = "ctx-mcp")]
struct Cli {
    /// Run as an MCP stdio server (newline-delimited JSON-RPC).
    #[arg(long)]
    stdio: bool,
}

fn ctx_env(name: &str) -> std::result::Result<String, std::env::VarError> {
    std::env::var(format!("CTX_{name}"))
}

fn ctx_env_opt(name: &str) -> Option<String> {
    ctx_env(name).ok()
}

fn lsp_tools_enabled() -> bool {
    ctx_env_opt("MCP_ENABLE_LSP_TOOLS")
        .map(|raw| {
            let v = raw.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn is_lsp_related_tool(name: &str) -> bool {
    name.starts_with("lsp_")
        || matches!(
            name,
            "list_edit_plans" | "get_edit_plan" | "apply_edit_plan" | "discard_edit_plan"
        )
}

#[derive(Default)]
struct SubagentRefMap {
    by_subagent: HashMap<String, String>,
    by_session: HashMap<String, String>,
}

impl SubagentRefMap {
    fn subagent_id_for_session(&mut self, session_id: &str) -> String {
        if let Some(existing) = self.by_session.get(session_id) {
            return existing.clone();
        }
        let subagent_id = format!("subagent-{}", Uuid::new_v4());
        self.by_session
            .insert(session_id.to_string(), subagent_id.clone());
        self.by_subagent
            .insert(subagent_id.clone(), session_id.to_string());
        subagent_id
    }

    fn session_id_for_subagent(&self, subagent_id: &str) -> Option<String> {
        self.by_subagent.get(subagent_id).cloned()
    }
}

static SUBAGENT_REFS: OnceLock<Mutex<SubagentRefMap>> = OnceLock::new();

fn subagent_refs() -> &'static Mutex<SubagentRefMap> {
    SUBAGENT_REFS.get_or_init(|| Mutex::new(SubagentRefMap::default()))
}

fn subagent_id_for_session(session_id: &str) -> String {
    let mut map = subagent_refs().lock().expect("subagent ref map poisoned");
    map.subagent_id_for_session(session_id)
}

fn session_id_for_subagent(subagent_id: &str) -> Option<String> {
    let map = subagent_refs().lock().expect("subagent ref map poisoned");
    map.session_id_for_subagent(subagent_id)
}

#[derive(Default)]
struct SubagentGroupRefMap {
    by_group: HashMap<String, String>,
    by_invocation: HashMap<String, String>,
}

impl SubagentGroupRefMap {
    fn subagent_group_id_for_invocation(&mut self, invocation_id: &str) -> String {
        if let Some(existing) = self.by_invocation.get(invocation_id) {
            return existing.clone();
        }
        let group_id = format!("subagent-group-{}", Uuid::new_v4());
        self.by_invocation
            .insert(invocation_id.to_string(), group_id.clone());
        self.by_group
            .insert(group_id.clone(), invocation_id.to_string());
        group_id
    }

    fn invocation_id_for_subagent_group(&self, subagent_group_id: &str) -> Option<String> {
        self.by_group.get(subagent_group_id).cloned()
    }
}

static SUBAGENT_GROUP_REFS: OnceLock<Mutex<SubagentGroupRefMap>> = OnceLock::new();

fn subagent_group_refs() -> &'static Mutex<SubagentGroupRefMap> {
    SUBAGENT_GROUP_REFS.get_or_init(|| Mutex::new(SubagentGroupRefMap::default()))
}

fn subagent_group_id_for_invocation(invocation_id: &str) -> String {
    let mut map = subagent_group_refs()
        .lock()
        .expect("subagent group ref map poisoned");
    map.subagent_group_id_for_invocation(invocation_id)
}

fn invocation_id_for_subagent_group(subagent_group_id: &str) -> Option<String> {
    let map = subagent_group_refs()
        .lock()
        .expect("subagent group ref map poisoned");
    map.invocation_id_for_subagent_group(subagent_group_id)
}

#[derive(Default)]
struct EditPlanRefMap {
    by_edit_plan: HashMap<String, String>,
    by_plan: HashMap<String, String>,
}

impl EditPlanRefMap {
    fn edit_plan_id_for_internal(&mut self, plan_id: &str) -> String {
        if let Some(existing) = self.by_plan.get(plan_id) {
            return existing.clone();
        }
        let edit_plan_id = format!("edit-plan-{}", Uuid::new_v4());
        self.by_plan
            .insert(plan_id.to_string(), edit_plan_id.clone());
        self.by_edit_plan
            .insert(edit_plan_id.clone(), plan_id.to_string());
        edit_plan_id
    }

    fn internal_plan_id_for_edit_plan(&self, edit_plan_id: &str) -> Option<String> {
        self.by_edit_plan.get(edit_plan_id).cloned()
    }
}

static EDIT_PLAN_REFS: OnceLock<Mutex<EditPlanRefMap>> = OnceLock::new();

fn edit_plan_refs() -> &'static Mutex<EditPlanRefMap> {
    EDIT_PLAN_REFS.get_or_init(|| Mutex::new(EditPlanRefMap::default()))
}

fn edit_plan_id_for_internal(plan_id: &str) -> String {
    let mut map = edit_plan_refs().lock().expect("edit plan ref map poisoned");
    map.edit_plan_id_for_internal(plan_id)
}

fn internal_plan_id_for_edit_plan(edit_plan_id: &str) -> Option<String> {
    let map = edit_plan_refs().lock().expect("edit plan ref map poisoned");
    map.internal_plan_id_for_edit_plan(edit_plan_id)
}

#[derive(Default)]
struct InteractiveSessionRefMap {
    by_session: HashMap<String, String>,
    by_ref: HashMap<String, String>,
}

impl InteractiveSessionRefMap {
    fn session_ref_for_session_id(&mut self, session_id: &str) -> String {
        if let Some(existing) = self.by_session.get(session_id) {
            return existing.clone();
        }
        let session_ref = format!("session-ref-{}", Uuid::new_v4());
        self.by_session
            .insert(session_id.to_string(), session_ref.clone());
        self.by_ref
            .insert(session_ref.clone(), session_id.to_string());
        session_ref
    }

    fn session_id_for_ref(&self, session_ref: &str) -> Option<String> {
        self.by_ref.get(session_ref).cloned()
    }
}

static INTERACTIVE_SESSION_REFS: OnceLock<Mutex<InteractiveSessionRefMap>> = OnceLock::new();

fn interactive_session_refs() -> &'static Mutex<InteractiveSessionRefMap> {
    INTERACTIVE_SESSION_REFS.get_or_init(|| Mutex::new(InteractiveSessionRefMap::default()))
}

fn session_ref_for_session_id(session_id: &str) -> String {
    let mut map = interactive_session_refs()
        .lock()
        .expect("interactive session ref map poisoned");
    map.session_ref_for_session_id(session_id)
}

fn session_id_for_ref(session_ref: &str) -> Option<String> {
    let map = interactive_session_refs()
        .lock()
        .expect("interactive session ref map poisoned");
    map.session_id_for_ref(session_ref)
}

const INTERNAL_KEYS: [&str; 16] = [
    "child_session_id",
    "ctx_session_id",
    "host_id",
    "invocation_id",
    "message_id",
    "parent_session_id",
    "parent_turn_id",
    "plan_id",
    "provider_session_ref",
    "run_id",
    "session_id",
    "task_id",
    "tool_call_id",
    "turn_id",
    "workspace_id",
    "worktree_id",
];

fn scrub_internal_fields(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                scrub_internal_fields(item);
            }
        }
        Value::Object(obj) => {
            for key in INTERNAL_KEYS {
                obj.remove(key);
            }
            for item in obj.values_mut() {
                scrub_internal_fields(item);
            }
        }
        _ => {}
    }
}

fn map_subagent_result(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let session_id = obj
        .remove("session_id")
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(session_id) = session_id {
        let subagent_id = subagent_id_for_session(&session_id);
        obj.insert("subagent_id".to_string(), Value::String(subagent_id));
    }
    if let Some(provider_id) = obj.remove("provider_id") {
        obj.insert("provider".to_string(), provider_id);
    }
    if let Some(model_id) = obj.remove("model_id") {
        obj.insert("model".to_string(), model_id);
    }
}

fn map_subagent_results(value: &mut Value) {
    let Some(items) = value.as_array_mut() else {
        return;
    };
    for item in items {
        map_subagent_result(item);
    }
}

fn map_subagent_invocation_summary(invocation: &Value) -> Option<Value> {
    let obj = invocation.as_object()?;
    let id = obj.get("id")?.as_str()?;
    let group_id = subagent_group_id_for_invocation(id);
    let status = obj.get("status").cloned().unwrap_or(Value::Null);
    let created_at = obj.get("created_at").cloned().unwrap_or(Value::Null);
    let updated_at = obj.get("updated_at").cloned().unwrap_or(Value::Null);
    let count = obj
        .get("children")
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    Some(json!({
        "subagent_group_id": group_id,
        "status": status,
        "created_at": created_at,
        "updated_at": updated_at,
        "subagent_count": count,
    }))
}

fn map_subagent_invocation_detail(invocation: &Value) -> Option<Value> {
    let obj = invocation.as_object()?;
    let id = obj.get("id")?.as_str()?;
    let group_id = subagent_group_id_for_invocation(id);
    let status = obj.get("status").cloned().unwrap_or(Value::Null);
    let created_at = obj.get("created_at").cloned().unwrap_or(Value::Null);
    let updated_at = obj.get("updated_at").cloned().unwrap_or(Value::Null);
    let mut subagents = Vec::new();
    if let Some(children) = obj.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let Some(child_obj) = child.as_object() else {
                continue;
            };
            let session_id = child_obj.get("child_session_id").and_then(|v| v.as_str());
            let mut mapped = serde_json::Map::new();
            if let Some(session_id) = session_id {
                let subagent_id = subagent_id_for_session(session_id);
                mapped.insert("subagent_id".to_string(), Value::String(subagent_id));
            }
            if let Some(status) = child_obj.get("status") {
                mapped.insert("status".to_string(), status.clone());
            }
            if let Some(label) = child_obj.get("label") {
                mapped.insert("label".to_string(), label.clone());
            }
            if let Some(harness) = child_obj.get("harness") {
                mapped.insert("provider".to_string(), harness.clone());
            }
            if let Some(model) = child_obj.get("model") {
                mapped.insert("model".to_string(), model.clone());
            }
            if !mapped.is_empty() {
                subagents.push(Value::Object(mapped));
            }
        }
    }
    Some(json!({
        "subagent_group_id": group_id,
        "status": status,
        "created_at": created_at,
        "updated_at": updated_at,
        "subagents": subagents,
    }))
}

fn map_edit_plan_summary(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let plan_id = obj.remove("id").or_else(|| obj.remove("plan_id"));
    let Some(plan_id) = plan_id else {
        return;
    };
    if let Some(id) = plan_id.as_str() {
        let edit_plan_id = edit_plan_id_for_internal(id);
        obj.insert("edit_plan_id".to_string(), Value::String(edit_plan_id));
    } else {
        obj.insert("edit_plan_id".to_string(), plan_id);
    }
}

fn map_edit_plan_summaries(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            map_edit_plan_summary(item);
        }
        return;
    }
    map_edit_plan_summary(value);
}

fn map_interactive_session(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(session_id) = obj.remove("id") {
        if let Some(session_id) = session_id.as_str() {
            let session_ref = session_ref_for_session_id(session_id);
            obj.insert("session_ref".to_string(), Value::String(session_ref));
        } else {
            obj.insert("session_ref".to_string(), session_id);
        }
    }
    obj.remove("ctx_session_id");
    obj.remove("workspace_id");
    obj.remove("worktree_id");
}

fn map_interactive_sessions(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            map_interactive_session(item);
        }
        return;
    }
    map_interactive_session(value);
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
                            "name": "ctx-mcp",
                            "title": "ctx MCP",
                            "version": env!("CARGO_PKG_VERSION"),
                            "description": "ctx daemon tools (bridge)"
                        }
                    }),
                )
            }
            "tools/list" => {
                let mut resp = json!({
                    "tools": [
                        {
                            "name": "ping",
                            "title": "ctx Ping",
                            "description": "Returns ok=true if ctx MCP is reachable.",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "list_workspaces",
                            "title": "List Workspaces",
                            "description": "Lists ctx workspaces via the ctx daemon HTTP API.",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "merge_queue_submit",
                            "title": "Merge Queue Submit",
                            "description": "Submit the current worktree to the merge queue and wait for completion.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "target_branch": { "type": "string" },
                                    "message": { "type": "string" }
                                },
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "agent_init",
                            "title": "Init Subagents",
                            "description": "Spawns one or more subagents (max configurable, default 10) for the current session. response_mode defaults to enqueue.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "response_mode": { "type": "string", "enum": ["enqueue", "await"], "description": "Optional response mode (default enqueue)." },
                                    "agents": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "prompt": { "type": "string" },
                                                "label": { "type": "string" },
                                                "harness": { "type": "string" },
                                                "model": { "type": "string" },
                                                "reasoning_effort": { "type": "string" }
                                            },
                                            "required": ["prompt"],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["agents"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "agent_reply",
                            "title": "Reply to Subagent",
                            "description": "Sends a prompt to an existing subagent and waits for its response.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "subagent_id": { "type": "string", "description": "Subagent id returned by agent_init." },
                                    "prompt": { "type": "string" }
                                },
                                "required": ["subagent_id", "prompt"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "subagent_invocations_list",
                            "title": "List Subagent Invocations",
                            "description": "Lists subagent invocations for the current session.",
                            "inputSchema": {
                                "type": "object",
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "subagent_invocation_get",
                            "title": "Get Subagent Invocation",
                            "description": "Fetches a subagent invocation by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "subagent_group_id": { "type": "string", "description": "Subagent group id returned by agent_init." }
                                },
                                "required": ["subagent_group_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "subagent_wait",
                            "title": "Wait for Subagent Invocation",
                            "description": "Waits for a subagent invocation to complete and returns results.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "subagent_group_id": { "type": "string", "description": "Subagent group id returned by agent_init." }
                                },
                                "required": ["subagent_group_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "artifacts_set",
                            "title": "Set Session Artifacts",
                            "description": "Sets the ordered list of artifacts for the current session. mp4/webm supported; .mov (video/quicktime) not supported.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "artifacts": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "absoluteFilePath": { "type": "string", "description": "Absolute file path to the artifact." },
                                                "name": { "type": "string", "description": "Optional display name." },
                                                "mimeType": { "type": "string", "description": "Optional MIME type override." }
                                            },
                                            "required": ["absoluteFilePath"],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["artifacts"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "oracle",
                            "title": "Oracle (High-Reasoning Advice)",
                            "description": "Calls a high-reasoning model for architecture/strategy advice. The oracle has NO access to your repo, tools, or files; provide all relevant context in the prompt. Prefer a single comprehensive call; follow-ups are allowed but can be slow.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "prompt_path": { "type": "string", "description": "Path to a file containing the full self-contained problem statement + the question for the oracle." },
                                    "response_path": { "type": "string", "description": "Optional file path to write the oracle response." },
                                    "model": { "type": "string", "description": "Optional model override (defaults to daemon oracle settings)." },
                                    "reasoning_effort": { "type": "string", "description": "Optional reasoning effort override (defaults to daemon oracle settings)." },
                                    "max_output_tokens": { "type": "integer", "minimum": 1, "description": "Optional output token cap (defaults to daemon oracle settings)." },
                                    "timeout_ms": { "type": "integer", "minimum": 1, "description": "Optional request timeout override (defaults to daemon oracle settings)." }
                                },
                                "required": ["prompt_path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_status",
                            "title": "LSP Status",
                            "description": "Returns the daemon LSP configuration and server availability (installed/missing).",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "lsp_install_server",
                            "title": "Install LSP Server (Managed)",
                            "description": "Triggers a managed install of an LSP server into the daemon data dir (may require restarting the daemon to take effect).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "server_id": { "type": "string", "description": "One of: typescript, python, html, css, json, yaml, bash, dockerfile." }
                                },
                                "required": ["server_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_catalog_list",
                            "title": "List LSP Catalog",
                            "description": "Lists curated LSP servers known to the daemon (installable/enabled without UI).",
                            "inputSchema": { "type": "object", "additionalProperties": false }
                        },
                        {
                            "name": "lsp_catalog_install",
                            "title": "Install LSP Server (Catalog)",
                            "description": "Installs and enables an LSP server from the daemon catalog (may require restarting the daemon to take effect).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "catalog_id": { "type": "string", "description": "Catalog entry id (e.g. rust-analyzer, gopls, taplo, marksman)." }
                                },
                                "required": ["catalog_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_diagnostics",
                            "title": "LSP Diagnostics",
                            "description": "Returns language-server diagnostics for a file in the current session worktree (requires daemon LSP enabled).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "File path (relative to session worktree, or absolute)." },
                                    "root_path": { "type": "string", "description": "Optional explicit root path when targeting a specific folder." }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_definition",
                            "title": "LSP Definition",
                            "description": "Returns the definition location at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_type_definition",
                            "title": "LSP Type Definition",
                            "description": "Returns the type definition location at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_implementation",
                            "title": "LSP Implementation",
                            "description": "Returns implementations at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_references",
                            "title": "LSP References",
                            "description": "Returns references at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_hover",
                            "title": "LSP Hover",
                            "description": "Returns hover information at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_signature_help",
                            "title": "LSP Signature Help",
                            "description": "Returns signature help at a position in a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                                "name": "lsp_completion",
                                "title": "LSP Completion",
                                "description": "Returns completion items at a position in a file.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_completion_resolve",
                                "title": "LSP Completion Resolve",
                                "description": "Resolves additional fields for a completion item (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_code_action_resolve",
                                "title": "LSP Code Action Resolve",
                                "description": "Resolves additional fields for a CodeAction (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_inlay_hints",
                                "title": "LSP Inlay Hints",
                                "description": "Returns inlay hints for a visible range (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_document_highlight",
                                "title": "LSP Document Highlight",
                                "description": "Returns document highlights at a position (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_selection_ranges",
                                "title": "LSP Selection Ranges",
                                "description": "Returns selection ranges for one or more positions (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "positions": {
                                            "type": "array",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "line": { "type": "integer", "minimum": 0 },
                                                    "character": { "type": "integer", "minimum": 0 }
                                                },
                                                "required": ["line", "character"],
                                                "additionalProperties": false
                                            }
                                        }
                                    },
                                    "required": ["path", "positions"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_call_hierarchy_prepare",
                                "title": "LSP Call Hierarchy (Prepare)",
                                "description": "Prepares call hierarchy items at a position.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_call_hierarchy_incoming",
                                "title": "LSP Call Hierarchy (Incoming)",
                                "description": "Returns incoming calls for a CallHierarchyItem.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_call_hierarchy_outgoing",
                                "title": "LSP Call Hierarchy (Outgoing)",
                                "description": "Returns outgoing calls for a CallHierarchyItem.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_code_lens",
                                "title": "LSP Code Lens",
                                "description": "Returns code lenses for a file (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" }
                                    },
                                    "required": ["path"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_code_lens_resolve",
                                "title": "LSP Code Lens Resolve",
                                "description": "Resolves additional fields for a code lens (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_prepare_rename",
                                "title": "LSP Prepare Rename",
                                "description": "Preflights a rename at a position and returns the target range (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_document_links",
                                "title": "LSP Document Links",
                                "description": "Returns document links for a file (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" }
                                    },
                                    "required": ["path"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_document_link_resolve",
                                "title": "LSP Document Link Resolve",
                                "description": "Resolves a document link target (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                        {
                            "name": "lsp_semantic_tokens_full",
                            "title": "LSP Semantic Tokens (Full)",
                            "description": "Returns semantic tokens for a file (intended for agent consumption; server-dependent).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_semantic_tokens_delta",
                            "title": "LSP Semantic Tokens (Delta)",
                            "description": "Returns semantic tokens delta for a file given a previous result id (server-dependent).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" },
                                    "previous_result_id": { "type": "string" }
                                },
                                "required": ["path", "previous_result_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_folding_ranges",
                            "title": "LSP Folding Ranges",
                            "description": "Returns folding ranges for a file (server-dependent).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_linked_editing_range",
                            "title": "LSP Linked Editing Range",
                            "description": "Returns linked editing ranges at a position (server-dependent).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                                "name": "lsp_type_hierarchy_prepare",
                                "title": "LSP Type Hierarchy Prepare",
                                "description": "Prepares a type hierarchy item at a position (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
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
                                "name": "lsp_type_hierarchy_supertypes",
                                "title": "LSP Type Hierarchy Supertypes",
                                "description": "Returns supertypes for a type hierarchy item (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_type_hierarchy_subtypes",
                                "title": "LSP Type Hierarchy Subtypes",
                                "description": "Returns subtypes for a type hierarchy item (server-dependent).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "item": { "type": "object" }
                                    },
                                    "required": ["path", "item"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_code_actions_by_diagnostic_plan",
                                "title": "LSP Code Actions (By Diagnostic) (Plan)",
                                "description": "Creates ranked edit plans for quick-fixes for a specific diagnostic (requires daemon LSP edit plans enabled).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "diagnostic": { "type": "object" }
                                    },
                                    "required": ["path", "diagnostic"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_execute_command",
                                "title": "LSP Execute Command",
                                "description": "Executes an allowlisted LSP command and returns any captured WorkspaceEdit (disabled by default; see daemon config).",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "root_path": { "type": "string" },
                                        "command": { "type": "string" },
                                        "arguments": { "type": "array" }
                                    },
                                    "required": ["path", "command"],
                                    "additionalProperties": false
                                }
                            },
                            {
                                "name": "lsp_execute_command_plan",
                                "title": "LSP Execute Command (Plan)",
                                "description": "Creates an edit plan from an allowlisted executeCommand that produces a WorkspaceEdit.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "command": { "type": "string" },
                                        "arguments": { "type": "array" }
                                    },
                                    "required": ["path", "command"],
                                    "additionalProperties": false
                                }
                            },
                        {
                            "name": "lsp_document_symbols",
                            "title": "LSP Document Symbols",
                            "description": "Returns document symbols for a file.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "root_path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_workspace_symbols",
                            "title": "LSP Workspace Symbols",
                            "description": "Returns workspace symbols for a query.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "root_path": { "type": "string" },
                                    "query": { "type": "string" }
                                },
                                "required": ["query"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_workspace_symbol_resolve",
                            "title": "LSP Workspace Symbol Resolve",
                            "description": "Resolves a workspace symbol item into a fully detailed representation (server-dependent).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "root_path": { "type": "string" },
                                    "item": { "type": "object" }
                                },
                                "required": ["item"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_code_actions",
                            "title": "LSP Code Actions",
                            "description": "Returns code actions for a selection.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_rename_plan",
                            "title": "LSP Rename (Plan)",
                            "description": "Creates an edit plan for an LSP rename operation (requires daemon LSP edit plans enabled).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
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
                            "name": "lsp_format_plan",
                            "title": "LSP Format (Plan)",
                            "description": "Creates an edit plan for formatting a document.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_organize_imports_plan",
                            "title": "LSP Organize Imports (Plan)",
                            "description": "Creates an edit plan for organizing imports (typically via LSP code actions).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" }
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "lsp_code_action_plan",
                            "title": "LSP Code Action (Plan)",
                            "description": "Creates an edit plan from an LSP CodeAction JSON object (expects a CodeAction with an edit).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "action": { "type": "object" }
                                },
                                "required": ["action"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "list_edit_plans",
                            "title": "List Edit Plans",
                            "description": "Lists pending edit plans for a worktree (or for the current session's worktree).",
                            "inputSchema": {
                                "type": "object",
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "get_edit_plan",
                            "title": "Get Edit Plan",
                            "description": "Fetches an edit plan summary by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "edit_plan_id": { "type": "string" }
                                },
                                "required": ["edit_plan_id"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "apply_edit_plan",
                            "title": "Apply Edit Plan Patch",
                            "description": "Applies or rejects a patch from an edit plan. If patch is omitted, applies the entire remaining plan diff.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "edit_plan_id": { "type": "string" },
                                    "action": { "type": "string", "enum": ["accept", "reject"] },
                                    "patch": { "type": "string" }
                                },
                                "required": ["edit_plan_id", "action"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "discard_edit_plan",
                            "title": "Discard Edit Plan",
                            "description": "Discards an edit plan by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "edit_plan_id": { "type": "string" }
                                },
                                "required": ["edit_plan_id"],
                                "additionalProperties": false
                            }
                        },
                        // TODO: Re-enable web session MCP tool definitions.
                        /*
                        {
                            "name": "session_create",
                            "title": "Create Session",
                            "description": "Creates a new session (currently supports kind=web).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string", "description": "Session kind (web)." },
                                    "target": {
                                        "type": "object",
                                        "properties": {
                                            "url": { "type": "string" }
                                        },
                                        "required": ["url"],
                                        "additionalProperties": true
                                    },
                                    "viewport": {
                                        "type": "object",
                                        "properties": {
                                            "width": { "type": "integer", "minimum": 1 },
                                            "height": { "type": "integer", "minimum": 1 }
                                        },
                                        "additionalProperties": false
                                    },
                                    "fps": { "type": "integer", "minimum": 1 },
                                },
                                "required": ["kind", "target"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "session_list",
                            "title": "List Sessions",
                            "description": "Lists active sessions (currently web only).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string" }
                                },
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "session_info",
                            "title": "Get Session Info",
                            "description": "Fetches session details by id.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_ref": { "type": "string" }
                                },
                                "required": ["session_ref"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "session_run",
                            "title": "Run Session Script",
                            "description": "Runs a script against a session (default timeout 5m).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_ref": { "type": "string" },
                                    "code": { "type": "string" },
                                    "script_path": { "type": "string" },
                                    "timeout_ms": { "type": "integer", "minimum": 1 }
                                },
                                "required": ["session_ref"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "session_eval",
                            "title": "Eval Session Script",
                            "description": "Evaluates code against a session (default timeout 5m).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_ref": { "type": "string" },
                                    "code": { "type": "string" },
                                    "script_path": { "type": "string" },
                                    "timeout_ms": { "type": "integer", "minimum": 1 }
                                },
                                "required": ["session_ref"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "session_close",
                            "title": "Close Session",
                            "description": "Closes a session and tears down resources.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "session_ref": { "type": "string" }
                                },
                                "required": ["session_ref"],
                                "additionalProperties": false
                            }
                        }
                        */
                    ]
                });

                if !lsp_tools_enabled() {
                    if let Some(tools) = resp.get_mut("tools").and_then(|v| v.as_array_mut()) {
                        tools.retain(|tool| {
                            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            !is_lsp_related_tool(name)
                        });
                    }
                }

                ok(id.unwrap(), resp)
            }
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

                if !lsp_tools_enabled() && is_lsp_related_tool(name.as_str()) {
                    ok(
                        id.unwrap(),
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
                            id.unwrap(),
                            json!({
                                "content": [{"type":"text","text": "{\"ok\":true}"}],
                                "isError": false
                            }),
                        ),
                        "list_workspaces" => {
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
                        "merge_queue_submit" => {
                            match merge_queue_submit_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "agent_init" => {
                            match agent_init_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "agent_reply" => {
                            match agent_reply_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "subagent_invocations_list" => {
                            match subagent_invocations_list_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "subagent_invocation_get" => {
                            match subagent_invocation_get_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "subagent_wait" => {
                            match subagent_wait_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "artifacts_set" => {
                            let normalized =
                                (|| -> std::result::Result<(String, Vec<Value>), Value> {
                                    let session_id =
                                        ctx_env_opt("SESSION_ID").ok_or_else(|| {
                                            error(
                                                id.clone().unwrap(),
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
                                                id.clone().unwrap(),
                                                -32602,
                                                "Invalid params",
                                                Some(json!({"missing":"artifacts"})),
                                            )
                                        })?;

                                    let mut normalized = Vec::with_capacity(items.len());
                                    for (idx, item) in items.iter().enumerate() {
                                        let obj = item.as_object().ok_or_else(|| {
                                        error(
                                            id.clone().unwrap(),
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
                                            id.clone().unwrap(),
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
                                        Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                        Err(e) => ok(id.unwrap(), tool_err(e)),
                                    }
                                }
                                Err(err) => err,
                            }
                        }
                        "oracle" => match oracle_call(&client, &daemon_url, &arguments).await {
                            Ok(val) => ok(id.unwrap(), tool_ok(val)),
                            Err(e) => ok(id.unwrap(), tool_err(e)),
                        },
                        "lsp_status" => {
                            let _ = arguments;
                            match lsp_status(&client, &daemon_url).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                    id.unwrap(),
                                    -32602,
                                    "Invalid params",
                                    Some(json!({"missing":"server_id"})),
                                )
                            } else {
                                match lsp_install_server(&client, &daemon_url, &server_id).await {
                                    Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                    Err(e) => ok(id.unwrap(), tool_err(e)),
                                }
                            }
                        }
                        "lsp_catalog_list" => {
                            let _ = arguments;
                            match lsp_catalog_list(&client, &daemon_url).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                    id.unwrap(),
                                    -32602,
                                    "Invalid params",
                                    Some(json!({"missing":"catalog_id"})),
                                )
                            } else {
                                match lsp_catalog_install(&client, &daemon_url, &catalog_id).await {
                                    Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                    Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                    Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                    Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_hover" => {
                            match lsp_pos_call(&client, &daemon_url, "/api/lsp/hover", &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_semantic_tokens_delta" => {
                            match lsp_semantic_tokens_delta_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_execute_command_plan" => {
                            match lsp_execute_command_plan_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_workspace_symbols" => {
                            match lsp_workspace_symbols_call(&client, &daemon_url, &arguments).await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
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
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_code_actions" => {
                            match lsp_code_actions_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_rename_plan" => {
                            match lsp_rename_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_format_plan" => {
                            match lsp_format_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_organize_imports_plan" => {
                            match lsp_organize_imports_plan_call(&client, &daemon_url, &arguments)
                                .await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "lsp_code_action_plan" => {
                            match lsp_code_action_plan_call(&client, &daemon_url, &arguments).await
                            {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "list_edit_plans" => {
                            match list_edit_plans_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "get_edit_plan" => {
                            match get_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "apply_edit_plan" => {
                            match apply_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "discard_edit_plan" => {
                            match discard_edit_plan_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        // TODO: Re-enable web session MCP tool handlers.
                        /*
                        "session_create" => {
                            match session_create_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "session_list" => {
                            match session_list_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "session_info" => {
                            match session_info_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "session_run" => {
                            match session_run_call(&client, &daemon_url, &arguments, false).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "session_eval" => {
                            match session_run_call(&client, &daemon_url, &arguments, true).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "session_close" => {
                            match session_close_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        */
                        _ => error(
                            id.unwrap(),
                            -32601,
                            "Method not found",
                            Some(json!({"tool": name})),
                        ),
                    }
                }
            }
            _ => error(
                id.unwrap(),
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

async fn merge_queue_submit_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id =
        ctx_env_opt("SESSION_ID").context("missing session context for merge queue submit")?;
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
    let response_mode = args
        .get("response_mode")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let tool_call_id = args
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let path = format!("/api/mcp/sessions/{}/agent_init", session_id);
    let mut body = json!({ "agents": agents });
    if let Some(response_mode) = response_mode {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("response_mode".to_string(), Value::String(response_mode));
        }
    }
    if let Some(tool_call_id) = tool_call_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
        }
    }
    let mut response = daemon_post_json(client, daemon_url, &path, &body).await?;
    if let Some(obj) = response.as_object_mut() {
        if let Some(invocation_id) = obj
            .remove("invocation_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
        {
            let group_id = subagent_group_id_for_invocation(&invocation_id);
            obj.insert("subagent_group_id".to_string(), Value::String(group_id));
        }
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
    let subagent_id = args
        .get("subagent_id")
        .and_then(|v| v.as_str())
        .context("missing subagent_id")?;
    let session_id = session_id_for_subagent(subagent_id).context("unknown subagent_id")?;
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .context("missing prompt")?;
    let path = format!("/api/mcp/sessions/{}/agent_reply", parent_session_id);
    let mut response = daemon_post_json(
        client,
        daemon_url,
        &path,
        &json!({ "session_id": session_id, "prompt": prompt }),
    )
    .await?;
    if let Some(obj) = response.as_object_mut() {
        if let Some(session_id) = obj
            .remove("session_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
        {
            let mapped = subagent_id_for_session(&session_id);
            obj.insert("subagent_id".to_string(), Value::String(mapped));
        }
    }
    Ok(response)
}

async fn subagent_invocations_list_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let _ = args;
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let path = format!("/api/sessions/{}/subagent_invocations", session_id);
    let response = daemon_get_json(client, daemon_url, &path).await?;
    let mut mapped = Vec::new();
    if let Some(items) = response.as_array() {
        for item in items {
            if let Some(value) = map_subagent_invocation_summary(item) {
                mapped.push(value);
            }
        }
    }
    Ok(Value::Array(mapped))
}

async fn subagent_invocation_get_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let group_id = args
        .get("subagent_group_id")
        .and_then(|v| v.as_str())
        .context("missing subagent_group_id")?;
    let invocation_id =
        invocation_id_for_subagent_group(group_id).context("unknown subagent_group_id")?;
    let invocation_id = urlencoding::encode(&invocation_id);
    let path = format!("/api/subagent_invocations/{invocation_id}");
    let response = daemon_get_json(client, daemon_url, &path).await?;
    map_subagent_invocation_detail(&response).context("invalid subagent invocation response")
}

async fn subagent_wait_call(
    client: &reqwest::Client,
    daemon_url: &str,
    args: &Value,
) -> Result<Value> {
    let session_id = ctx_env_opt("SESSION_ID").context("missing session context")?;
    let group_id = args
        .get("subagent_group_id")
        .and_then(|v| v.as_str())
        .context("missing subagent_group_id")?;
    let invocation_id =
        invocation_id_for_subagent_group(group_id).context("unknown subagent_group_id")?;
    let path = format!("/api/mcp/sessions/{}/subagent_wait", session_id);
    let mut response = daemon_post_json(
        client,
        daemon_url,
        &path,
        &json!({ "invocation_id": invocation_id }),
    )
    .await?;
    if let Some(obj) = response.as_object_mut() {
        if let Some(invocation_id) = obj
            .remove("invocation_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
        {
            let mapped = subagent_group_id_for_invocation(&invocation_id);
            obj.insert("subagent_group_id".to_string(), Value::String(mapped));
        }
        if let Some(results) = obj.get_mut("results") {
            map_subagent_results(results);
        }
    }
    Ok(response)
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

async fn lsp_status(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/lsp/status").await
}

async fn lsp_install_server(
    client: &reqwest::Client,
    daemon_url: &str,
    server_id: &str,
) -> Result<Value> {
    let server_id = urlencoding::encode(server_id);
    let path = format!("/api/lsp/servers/{server_id}/install");
    daemon_post_json(client, daemon_url, &path, &json!({})).await
}

async fn lsp_catalog_list(client: &reqwest::Client, daemon_url: &str) -> Result<Value> {
    daemon_get_json(client, daemon_url, "/api/lsp/catalog").await
}

async fn lsp_catalog_install(
    client: &reqwest::Client,
    daemon_url: &str,
    catalog_id: &str,
) -> Result<Value> {
    let catalog_id = urlencoding::encode(catalog_id);
    let path = format!("/api/lsp/catalog/{catalog_id}/install");
    daemon_post_json(client, daemon_url, &path, &json!({})).await
}

async fn lsp_diagnostics(
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
    let body = lsp_file_call_body(arguments)?;
    daemon_post_json(client, daemon_url, path, &body).await
}

async fn lsp_pos_call(
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

async fn lsp_item_call(
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

async fn lsp_range_call(
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

async fn lsp_selection_ranges_call(
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

async fn lsp_execute_command_call(
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

async fn lsp_execute_command_plan_call(
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

fn lsp_file_call_body(arguments: &Value) -> Result<Value> {
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

fn lsp_workspace_symbols_body(arguments: &Value) -> Result<Value> {
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

fn lsp_workspace_symbol_resolve_body(arguments: &Value) -> Result<Value> {
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

async fn lsp_workspace_symbols_call(
    client: &reqwest::Client,
    daemon_url: &str,
    arguments: &Value,
) -> Result<Value> {
    let body = lsp_workspace_symbols_body(arguments)?;
    daemon_post_json(client, daemon_url, "/api/lsp/workspace_symbols", &body).await
}

async fn lsp_workspace_symbol_resolve_call(
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

async fn lsp_semantic_tokens_delta_call(
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

async fn lsp_format_plan_call(
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

async fn lsp_organize_imports_plan_call(
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

async fn lsp_code_action_plan_call(
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

async fn lsp_code_actions_by_diagnostic_plan_call(
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
