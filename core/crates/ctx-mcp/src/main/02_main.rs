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
                            "name": "subagent_init",
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
                            "name": "subagent_reply",
                            "title": "Reply to Subagent",
                            "description": "Sends a prompt to an existing subagent and waits for its response.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "subagent_id": { "type": "string", "description": "Subagent id returned by subagent_init." },
                                    "prompt": { "type": "string" }
                                },
                                "required": ["subagent_id", "prompt"],
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
                                    "subagent_group_id": { "type": "string", "description": "Subagent group id returned by subagent_init." }
                                },
                                "required": ["subagent_group_id"],
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

                if !dev_tools_enabled() {
                    if let Some(tools) = resp.get_mut("tools").and_then(|v| v.as_array_mut()) {
                        tools.retain(|tool| {
                            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            name != "ping"
                        });
                    }
                }

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

                if !dev_tools_enabled() && name.as_str() == "ping" {
                    ok(
                        id.unwrap(),
                        tool_err(anyhow::anyhow!(
                            "tool disabled: {name} (ping is dev-only; set CTX_MCP_DEV_MODE=1 to enable)"
                        )),
                    )
                } else if !lsp_tools_enabled() && is_lsp_related_tool(name.as_str()) {
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
                        "subagent_init" => {
                            match agent_init_call(&client, &daemon_url, &arguments).await {
                                Ok(val) => ok(id.unwrap(), tool_ok(val)),
                                Err(e) => ok(id.unwrap(), tool_err(e)),
                            }
                        }
                        "subagent_reply" => {
                            match agent_reply_call(&client, &daemon_url, &arguments).await {
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
                        "subagent_list" => {
                            match subagent_list_call(&client, &daemon_url).await {
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
