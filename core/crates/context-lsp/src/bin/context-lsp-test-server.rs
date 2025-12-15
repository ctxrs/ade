use std::io::{Read, Write};

use lsp_types::{
    CodeAction, CodeActionKind, Command, Diagnostic, DiagnosticSeverity, Location, NumberOrString,
    Position, PublishDiagnosticsParams, Range, SymbolKind, TextEdit, TypeHierarchyItem, Uri, WorkspaceEdit,
};
use serde_json::{Value, json};

fn main() {
    // Minimal stdio LSP server used for workspace tests. Not shipped or used in production.
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
                write_response(
                    &mut output,
                    json!({"jsonrpc":"2.0","id": id,"result": null}),
                );
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
                .cloned()
                .unwrap_or(json!("file:///unknown.rs"));
            last_opened_uri = uri.as_str().map(|s| s.to_string());

            if let Some(uri_str) = uri.as_str() {
                if uri_str.contains("apply_edit.rs") {
                    write_response(
                        &mut output,
                        json!({
                            "jsonrpc":"2.0",
                            "id": 8888,
                            "method":"workspace/applyEdit",
                            "params": {
                                "label": "Test didOpen applyEdit",
                                "edit": {
                                    "changes": {
                                        (uri_str): [{
                                            "range": {
                                                "start": {"line": 0, "character": 0},
                                                "end": {"line": 0, "character": 0}
                                            },
                                            "newText": "// didOpen applyEdit\n"
                                        }]
                                    }
                                }
                            }
                        }),
                    );
                    let _ = read_lsp_message(&mut input);
                }
            }

            let diag = Diagnostic {
                range: Range {
                    start: Position { line: 1, character: 0 },
                    end: Position { line: 1, character: 1 },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("CTX_TEST".into())),
                code_description: None,
                source: Some("context-lsp-test".into()),
                message: "Intentional diagnostic from test server".into(),
                related_information: None,
                tags: None,
                data: None,
            };

            let params = PublishDiagnosticsParams {
                uri: uri.as_str().unwrap_or("file:///unknown.rs").parse().unwrap(),
                diagnostics: vec![diag],
                version: None,
            };
            write_response(
                &mut output,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params": params}),
            );
            continue;
        }

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        // Handle core request methods used by tests.
        if let Some(id) = id {
            if method == "workspace/executeCommand" {
                let uri = last_opened_uri
                    .clone()
                    .unwrap_or_else(|| "file:///unknown.rs".to_string());
                // Emit a server->client applyEdit request and wait for its response.
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

                write_response(
                    &mut output,
                    json!({"jsonrpc":"2.0","id": id,"result": {"ok": true}}),
                );
                continue;
            }

            let result = match method {
                "textDocument/definition" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let loc = Location {
                        uri,
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 1 },
                        },
                    };
                    json!([loc])
                }
                "textDocument/typeDefinition" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let loc = Location {
                        uri,
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 1 },
                        },
                    };
                    json!([loc])
                }
                "textDocument/implementation" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let loc = Location {
                        uri,
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 1 },
                        },
                    };
                    json!([loc])
                }
                "textDocument/references" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let loc = Location {
                        uri,
                        range: Range {
                            start: Position { line: 1, character: 0 },
                            end: Position { line: 1, character: 1 },
                        },
                    };
                    json!([loc])
                }
                "textDocument/hover" => {
                    json!({
                        "contents": {
                            "kind": "markdown",
                            "value": "Test hover"
                        }
                    })
                }
                "textDocument/signatureHelp" => {
                    json!({
                        "signatures": [{
                            "label": "f(x: i32) -> i32",
                            "documentation": { "kind": "markdown", "value": "Test signature" }
                        }],
                        "activeSignature": 0,
                        "activeParameter": 0
                    })
                }
                "textDocument/completion" => {
                    json!({
                        "isIncomplete": false,
                        "items": [{
                            "label": "completion_item",
                            "kind": 6
                        }]
                    })
                }
                "completionItem/resolve" => {
                    let mut item = msg.get("params").cloned().unwrap_or(json!({}));
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("detail".to_string(), json!("resolved"));
                    }
                    item
                }
                "codeAction/resolve" => {
                    let uri: lsp_types::Uri = last_opened_uri
                        .as_deref()
                        .unwrap_or("file:///unknown.rs")
                        .parse()
                        .unwrap();
                    let edit = WorkspaceEdit {
                        changes: Some(
                            std::iter::once((
                                uri,
                                vec![TextEdit {
                                    range: Range {
                                        start: Position { line: 0, character: 0 },
                                        end: Position { line: 0, character: 0 },
                                    },
                                    new_text: "// resolved code action\n".to_string(),
                                }],
                            ))
                            .collect(),
                        ),
                        document_changes: None,
                        change_annotations: None,
                    };
                    let mut ca = msg.get("params").cloned().unwrap_or(json!({}));
                    if let Some(obj) = ca.as_object_mut() {
                        obj.insert("edit".to_string(), json!(edit));
                    }
                    ca
                }
                "textDocument/inlayHint" => json!([
                    { "position": { "line": 0, "character": 0 }, "label": "hint" }
                ]),
                "textDocument/documentHighlight" => json!([
                    { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "kind": 1 }
                ]),
                "textDocument/selectionRange" => json!([
                    { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "parent": null }
                ]),
                "textDocument/prepareCallHierarchy" => {
                    let uri: lsp_types::Uri = last_opened_uri
                        .as_deref()
                        .unwrap_or("file:///unknown.rs")
                        .parse()
                        .unwrap();
                    json!([{
                        "name": "f",
                        "kind": 12,
                        "uri": uri,
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                        "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
                    }])
                }
                "callHierarchy/incomingCalls" => json!([
                    {
                        "from": msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({})),
                        "fromRanges": [{ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }]
                    }
                ]),
                "callHierarchy/outgoingCalls" => json!([
                    {
                        "to": msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({})),
                        "fromRanges": [{ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }]
                    }
                ]),
                "textDocument/codeLens" => json!([
                    {
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                        "command": { "title": "Run", "command": "run" },
                        "data": { "k": "v" }
                    }
                ]),
                "codeLens/resolve" => {
                    let mut lens = msg.get("params").cloned().unwrap_or(json!({}));
                    if let Some(obj) = lens.as_object_mut() {
                        obj.insert("command".to_string(), json!({ "title": "Run (resolved)", "command": "run" }));
                    }
                    lens
                }
                "textDocument/documentSymbol" => json!([]),
                "workspace/symbol" => json!([]),
                "textDocument/rename" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let edit = WorkspaceEdit {
                        changes: Some(
                            std::iter::once((
                                uri,
                                vec![TextEdit {
                                    range: Range {
                                        start: Position { line: 0, character: 0 },
                                        end: Position { line: 0, character: 0 },
                                    },
                                    new_text: "RENAMED_".to_string(),
                                }],
                            ))
                            .collect(),
                        ),
                        document_changes: None,
                        change_annotations: None,
                    };
                    json!(edit)
                }
                "textDocument/formatting" => json!([
                    TextEdit {
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 0 },
                        },
                        new_text: "// formatted\n".to_string(),
                    }
                ]),
                "textDocument/codeAction" => {
                    let uri: lsp_types::Uri = uri_from_params(&msg)
                        .unwrap_or_else(|| "file:///unknown.rs".parse().unwrap());
                    let edit = WorkspaceEdit {
                        changes: Some(
                            std::iter::once((
                                uri,
                                vec![TextEdit {
                                    range: Range {
                                        start: Position { line: 0, character: 0 },
                                        end: Position { line: 0, character: 0 },
                                    },
                                    new_text: "// fix\n".to_string(),
                                }],
                            ))
                            .collect(),
                        ),
                        document_changes: None,
                        change_annotations: None,
                    };
                    let ca = CodeAction {
                        title: "Apply quick fix".to_string(),
                        kind: Some(CodeActionKind::QUICKFIX),
                        diagnostics: None,
                        edit: Some(edit),
                        command: Some(Command {
                            title: "noop".to_string(),
                            command: "noop".to_string(),
                            arguments: None,
                        }),
                        is_preferred: Some(true),
                        disabled: None,
                        data: None,
                    };
                    json!([ca])
                }
                "textDocument/prepareRename" => json!({
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 3 }
                }),
                "textDocument/documentLink" => json!([
                    {
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
                        "target": null,
                        "data": { "k": "v" }
                    }
                ]),
                "documentLink/resolve" => {
                    let mut link = msg.get("params").cloned().unwrap_or(json!({}));
                    if let Some(obj) = link.as_object_mut() {
                        obj.insert("target".to_string(), json!("file:///resolved"));
                    }
                    link
                }
                "textDocument/semanticTokens/full" => json!({
                    "resultId": "1",
                    "data": [0,0,5,0,0]
                }),
                "textDocument/semanticTokens/full/delta" => json!({
                    "resultId": "2",
                    "edits": []
                }),
                "textDocument/foldingRange" => json!([
                    { "startLine": 0, "endLine": 1 }
                ]),
                "textDocument/linkedEditingRange" => json!({
                    "ranges": [
                        { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 4 } },
                        { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 11 } }
                    ],
                    "wordPattern": null
                }),
                "workspace/symbol/resolve" => {
                    let mut item = msg.get("params").cloned().unwrap_or(json!({}));
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("containerName".to_string(), json!("resolved_container"));
                    }
                    item
                }
                "textDocument/prepareTypeHierarchy" => {
                    let uri: Uri = last_opened_uri
                        .as_deref()
                        .unwrap_or("file:///unknown.rs")
                        .parse()
                        .unwrap();
                    let item = TypeHierarchyItem {
                        name: "T".to_string(),
                        kind: SymbolKind::STRUCT,
                        tags: None,
                        detail: None,
                        uri,
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 1 },
                        },
                        selection_range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 1 },
                        },
                        data: None,
                    };
                    json!([item])
                }
                "typeHierarchy/supertypes" => {
                    let item = msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({}));
                    json!([item])
                }
                "typeHierarchy/subtypes" => {
                    let item = msg.get("params").and_then(|p| p.get("item")).cloned().unwrap_or(json!({}));
                    json!([item])
                }
                _ => Value::Null,
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
    }
}

fn uri_from_params(msg: &Value) -> Option<lsp_types::Uri> {
    msg.get("params")
        .and_then(|p| {
            p.get("textDocument")
                .and_then(|td| td.get("uri"))
                .or_else(|| {
                    p.get("textDocumentPosition")
                        .and_then(|tdp| tdp.get("textDocument"))
                        .and_then(|td| td.get("uri"))
                })
                .or_else(|| {
                    p.get("textDocumentPosition")
                        .and_then(|tdp| tdp.get("textDocument"))
                        .and_then(|td| td.get("uri"))
                })
        })
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
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
    // Read until \r\n\r\n
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
