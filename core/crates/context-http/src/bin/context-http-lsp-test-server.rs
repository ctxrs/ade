use std::io::{Read, Write};

use serde_json::{Value, json};

fn main() {
    // Minimal stdio LSP server used by context-http integration tests.
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut last_opened_uri: Option<String> = None;

    loop {
        let msg = match read_lsp_message(&mut input) {
            Ok(v) => v,
            Err(_) => break,
        };

        if msg.get("method").and_then(|v| v.as_str()) == Some("initialize") {
            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": {
	                            "capabilities": {
	                                "textDocumentSync": 1,
	                                "definitionProvider": true,
	                                "typeDefinitionProvider": true,
	                                "implementationProvider": true,
	                                "referencesProvider": true,
	                                "hoverProvider": true,
	                                "signatureHelpProvider": true,
	                                "completionProvider": true,
	                                "documentLinkProvider": { "resolveProvider": true },
	                                "inlayHintProvider": true,
	                                "documentHighlightProvider": true,
	                                "selectionRangeProvider": true,
	                                "callHierarchyProvider": true,
	                                "typeHierarchyProvider": true,
	                                "semanticTokensProvider": { "full": true, "legend": { "tokenTypes": [], "tokenModifiers": [] } },
	                                "codeLensProvider": { "resolveProvider": true },
	                                "executeCommandProvider": { "commands": ["context.test.fixAll"] },
	                                "documentSymbolProvider": true,
	                                "workspaceSymbolProvider": true,
	                                "renameProvider": { "prepareProvider": true },
	                                "documentFormattingProvider": true,
	                                "codeActionProvider": true
	                            }
	                        }
	                    }),
	                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("shutdown") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(&mut output, json!({"jsonrpc":"2.0","id": id,"result": null}));
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("exit") {
            break;
        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/didOpen") {
	            let uri = msg
	                .get("params")
	                .and_then(|p| p.get("textDocument"))
	                .and_then(|d| d.get("uri"))
	                .and_then(|u| u.as_str())
	                .unwrap_or("file:///unknown.rs")
	                .to_string();
	            last_opened_uri = Some(uri.clone());

            let params = json!({
                "uri": uri,
                "diagnostics": [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 1}
                    },
                    "severity": 1,
                    "code": "CTX_HTTP_TEST",
                    "source": "context-http-lsp-test",
                    "message": "Intentional diagnostic from http test server"
                }],
                "version": null
            });
            write_response(&mut output, json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params": params}));
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/didChange") {
            let uri = last_opened_uri
                .clone()
                .unwrap_or_else(|| "file:///unknown.rs".to_string());
            let params = json!({
                "uri": uri,
                "diagnostics": [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 1}
                    },
                    "severity": 1,
                    "code": "CTX_HTTP_TEST",
                    "source": "context-http-lsp-test",
                    "message": "Intentional diagnostic from http test server"
                }],
                "version": null
            });
            write_response(
                &mut output,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params": params}),
            );
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/definition") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            }
                        }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/typeDefinition") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/implementation") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/references") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/prepareRename") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 3}
                        }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/documentLink") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
                            "target": null,
                            "data": { "k": "v" }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("documentLink/resolve") {
            if let Some(id) = msg.get("id").cloned() {
                let mut link = msg.get("params").cloned().unwrap_or(json!({}));
                if let Some(obj) = link.as_object_mut() {
                    obj.insert("target".to_string(), json!("file:///resolved"));
                }
                write_response(
                    &mut output,
                    json!({ "jsonrpc":"2.0", "id": id, "result": link }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/semanticTokens/full") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": { "resultId": "1", "data": [0,0,5,0,0] }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/semanticTokens/full/delta") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": { "resultId": "2", "edits": [] }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/foldingRange") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{ "startLine": 0, "endLine": 1 }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/linkedEditingRange") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "ranges": [
                                { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 4 } },
                                { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 11 } }
                            ],
                            "wordPattern": null
                        }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/prepareTypeHierarchy") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = last_opened_uri
                    .clone()
                    .unwrap_or_else(|| "file:///unknown.rs".to_string());
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "name": "T",
                            "kind": 23,
                            "uri": uri,
                            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                            "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("typeHierarchy/supertypes") {
            if let Some(id) = msg.get("id").cloned() {
                let item = msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({}));
                write_response(
                    &mut output,
                    json!({ "jsonrpc":"2.0", "id": id, "result": [item] }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("typeHierarchy/subtypes") {
            if let Some(id) = msg.get("id").cloned() {
                let item = msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({}));
                write_response(
                    &mut output,
                    json!({ "jsonrpc":"2.0", "id": id, "result": [item] }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/hover") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "contents": { "kind": "markdown", "value": "Test hover (http)" }
                        }
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/signatureHelp") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "signatures": [{
                                "label": "f(x: i32) -> i32",
                                "documentation": { "kind": "markdown", "value": "Test signature (http)" }
                            }],
                            "activeSignature": 0,
                            "activeParameter": 0
                        }
                    }),
                );
            }
            continue;
        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/completion") {
	            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "isIncomplete": false,
                            "items": [{ "label": "completion_item", "kind": 6 }]
                        }
                    }),
                );
            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("completionItem/resolve") {
	            if let Some(id) = msg.get("id").cloned() {
	                let mut item = msg.get("params").cloned().unwrap_or(json!({}));
	                if let Some(obj) = item.as_object_mut() {
	                    obj.insert("detail".to_string(), json!("resolved (http)"));
	                }
	                write_response(
	                    &mut output,
	                    json!({ "jsonrpc":"2.0", "id": id, "result": item }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("codeAction/resolve") {
	            if let Some(id) = msg.get("id").cloned() {
	                let uri = last_opened_uri
	                    .as_deref()
	                    .unwrap_or("file:///unknown.rs");
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": {
	                            "title": "Resolved action",
	                            "kind": "quickfix",
	                            "edit": {
	                                "changes": {
	                                    (uri): [{
	                                        "range": {
	                                            "start": {"line": 0, "character": 0},
	                                            "end": {"line": 0, "character": 0}
	                                        },
	                                        "newText": "// resolved (http)\n"
	                                    }]
	                                }
	                            }
	                        }
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/inlayHint") {
	            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{ "position": { "line": 0, "character": 0 }, "label": "hint" }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/documentHighlight") {
	            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "range": {
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            },
	                            "kind": 1
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/selectionRange") {
	            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "range": {
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 3}
	                            },
	                            "parent": null
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/prepareCallHierarchy") {
	            if let Some(id) = msg.get("id").cloned() {
	                let uri = last_opened_uri
	                    .as_deref()
	                    .unwrap_or("file:///unknown.rs");
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "name": "f",
	                            "kind": 12,
	                            "uri": uri,
	                            "range": {
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            },
	                            "selectionRange": {
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            }
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("callHierarchy/incomingCalls") {
	            if let Some(id) = msg.get("id").cloned() {
	                let item = msg
	                    .get("params")
	                    .and_then(|p| p.get("item"))
	                    .cloned()
	                    .unwrap_or(json!({}));
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "from": item,
	                            "fromRanges": [{
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            }]
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("callHierarchy/outgoingCalls") {
	            if let Some(id) = msg.get("id").cloned() {
	                let item = msg
	                    .get("params")
	                    .and_then(|p| p.get("item"))
	                    .cloned()
	                    .unwrap_or(json!({}));
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "to": item,
	                            "fromRanges": [{
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            }]
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/codeLens") {
	            if let Some(id) = msg.get("id").cloned() {
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
	                            "range": {
	                                "start": {"line": 0, "character": 0},
	                                "end": {"line": 0, "character": 1}
	                            },
	                            "command": { "title": "Run", "command": "run" },
	                            "data": { "k": "v" }
	                        }]
	                    }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("codeLens/resolve") {
	            if let Some(id) = msg.get("id").cloned() {
	                let mut lens = msg.get("params").cloned().unwrap_or(json!({}));
	                if let Some(obj) = lens.as_object_mut() {
	                    obj.insert("command".to_string(), json!({ "title": "Run (resolved)", "command": "run" }));
	                }
	                write_response(
	                    &mut output,
	                    json!({ "jsonrpc":"2.0", "id": id, "result": lens }),
	                );
	            }
	            continue;
	        }

	        if msg.get("method").and_then(|v| v.as_str()) == Some("workspace/executeCommand") {
	            if let Some(id) = msg.get("id").cloned() {
	                let uri = last_opened_uri
	                    .clone()
	                    .unwrap_or_else(|| "file:///unknown.rs".to_string());
	                write_response(
	                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": 9999,
	                        "method":"workspace/applyEdit",
	                        "params": {
	                            "label": "Test executeCommand",
	                            "edit": {
	                                "changes": {
	                                    (uri): [{
	                                        "range": {
	                                            "start": {"line": 0, "character": 0},
	                                            "end": {"line": 0, "character": 0}
	                                        },
	                                        "newText": "// execCommand\n"
	                                    }]
	                                }
	                            }
	                        }
	                    }),
	                );
	                let _ = read_lsp_message(&mut input);

	                write_response(&mut output, json!({"jsonrpc":"2.0","id": id,"result": {"ok": true}}));
	            }
	            continue;
	        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/documentSymbol") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({"jsonrpc":"2.0","id": id,"result": []}),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("workspace/symbol") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": [{
                            "name": "TestSymbol",
                            "kind": 12,
                            "location": {
                                "uri": "file:///unknown.rs",
                                "range": {
                                    "start": {"line": 0, "character": 0},
                                    "end": {"line": 0, "character": 1}
                                }
                            }
                        }]
                    }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("workspace/symbol/resolve") {
            if let Some(id) = msg.get("id").cloned() {
                let mut item = msg.get("params").cloned().unwrap_or(json!({}));
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("containerName".to_string(), json!("resolved_container"));
                }
                write_response(
                    &mut output,
                    json!({ "jsonrpc":"2.0", "id": id, "result": item }),
                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/rename") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                let new_name = msg
                    .get("params")
                    .and_then(|p| p.get("newName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("renamed");
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
	                            "changes": {
	                                uri: [{
	                                    "range": {
	                                        "start": {"line": 0, "character": 0},
	                                        "end": {"line": 0, "character": 0}
	                                    },
	                                    "newText": format!("// rename: {new_name}\n")
	                                }]
	                            }
	                        }
	                    }),
	                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/formatting") {
            if let Some(id) = msg.get("id").cloned() {
                write_response(
                    &mut output,
	                    json!({
	                        "jsonrpc":"2.0",
	                        "id": id,
	                        "result": [{
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 0}
	                            },
	                            "newText": "/* formatted */\n"
	                        }]
	                    }),
	                );
            }
            continue;
        }

        if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/codeAction") {
            if let Some(id) = msg.get("id").cloned() {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|d| d.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///unknown.rs");
                let only = msg
                    .get("params")
                    .and_then(|p| p.get("context"))
                    .and_then(|c| c.get("only"))
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let wants_org = only.iter().any(|v| v.as_str() == Some("source.organizeImports"));
	                let result = if wants_org {
	                    json!([{
	                        "title": "Organize imports",
	                        "kind": "source.organizeImports",
                        "command": {
                            "title": "Organize imports",
                            "command": "context.test.organizeImports",
                            "arguments": [{
                                "edit": {
	                                    "changes": {
	                                        uri: [{
	                                            "range": {
	                                                "start": {"line": 0, "character": 0},
	                                                "end": {"line": 0, "character": 0}
	                                            },
	                                            "newText": "/* organize imports */\n"
	                                        }]
	                                    }
	                                }
	                            }]
	                        }
	                    }])
	                } else {
	                    json!([{
	                        "title": "Insert TODO",
	                        "kind": "quickfix",
	                        "edit": {
	                            "changes": {
	                                uri: [{
	                                    "range": {
	                                        "start": {"line": 0, "character": 0},
	                                        "end": {"line": 0, "character": 0}
	                                    },
	                                    "newText": "// TODO\n"
	                                }]
	                            }
	                        }
	                    }])
	                };
                write_response(
                    &mut output,
                    json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": result
                    }),
                );
            }
            continue;
        }

        if let Some(id) = msg.get("id").cloned() {
            write_response(&mut output, json!({"jsonrpc":"2.0","id": id,"result": null}));
        }
    }
}

fn write_response(out: &mut impl Write, msg: Value) {
    let body = serde_json::to_vec(&msg).unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let _ = out.write_all(header.as_bytes());
    let _ = out.write_all(&body);
    let _ = out.flush();
}

fn read_lsp_message(input: &mut impl Read) -> anyhow::Result<Value> {
    let mut header = Vec::new();
    let mut buf = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if input.read_exact(&mut buf).is_err() {
            anyhow::bail!("EOF");
        }
        header.push(buf[0]);
        if header.len() > 16 * 1024 {
            anyhow::bail!("header too large");
        }
    }
    let header_str = String::from_utf8_lossy(&header);
    let mut content_len: Option<usize> = None;
    for line in header_str.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_len = Some(rest.trim().parse()?);
        }
    }
    let len = content_len.ok_or_else(|| anyhow::anyhow!("missing content-length"))?;
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}
