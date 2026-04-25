use serde_json::{json, Value};

pub(super) fn tools_list_response() -> Value {
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
                "name": "subagent_init",
                "title": "Init Subagents",
                "description": "Spawns one or more subagents (max configurable, default 10) for the current session. Enqueue-only; use subagent_wait to await.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "worktree": { "type": "string", "enum": ["inherit", "new"], "description": "Worktree selection for spawned subagents." },
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
                                "required": ["prompt", "label"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["worktree", "agents"],
                    "additionalProperties": false
                }
            },
            {
                "name": "subagent_reply",
                "title": "Reply to Subagent",
                "description": "Sends a prompt to an existing subagent (enqueue-only). Use subagent_wait to await the response.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string", "description": "Subagent label." },
                        "prompt": { "type": "string" }
                    },
                    "required": ["label", "prompt"],
                    "additionalProperties": false
                }
            },
            {
                "name": "subagent_wait",
                "title": "Wait for Subagent Invocation",
                "description": "Waits for subagent runs to complete and returns results.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string", "description": "Subagent label." },
                        "labels": { "type": "array", "items": { "type": "string" }, "description": "Subagent labels." }
                    },
                    "additionalProperties": false
                }
            },
            {
                "name": "subagent_interrupt",
                "title": "Interrupt Subagent",
                "description": "Requests interruption for a subagent (or all subagents) in the current session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string", "description": "Subagent label." },
                        "all": { "type": "boolean", "description": "Interrupt all subagents." }
                    },
                    "additionalProperties": false
                }
            },
            {
                "name": "subagent_list",
                "title": "List Subagents",
                "description": "Lists subagents for the current session.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false
                }
            },
            {
                "name": "artifacts_set",
                "title": "Set Session Artifacts",
                "description": "Sets the ordered list of artifacts for the current session. Paths must stay inside the session worktree or that session's tool-output spool subtree. Video artifacts such as mp4, webm, and mov are supported.",
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

    if !super::dev_tools_enabled() {
        if let Some(tools) = resp.get_mut("tools").and_then(|v| v.as_array_mut()) {
            tools.retain(|tool| {
                let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
                name != "ping"
            });
        }
    }

    resp
}
