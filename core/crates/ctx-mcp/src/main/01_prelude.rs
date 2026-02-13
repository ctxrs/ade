
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

fn dev_tools_enabled() -> bool {
    ctx_env_opt("MCP_DEV_MODE")
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

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

fn edit_plan_id_for_internal(plan_id: &str) -> String {
    let mut map = lock_or_recover(edit_plan_refs(), "edit plan ref map");
    map.edit_plan_id_for_internal(plan_id)
}

fn internal_plan_id_for_edit_plan(edit_plan_id: &str) -> Option<String> {
    let map = lock_or_recover(edit_plan_refs(), "edit plan ref map");
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
    let mut map = lock_or_recover(interactive_session_refs(), "interactive session ref map");
    map.session_ref_for_session_id(session_id)
}

fn session_id_for_ref(session_ref: &str) -> Option<String> {
    let map = lock_or_recover(interactive_session_refs(), "interactive session ref map");
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
    obj.remove("session_id");
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
