use std::io::{Read, Write};

use lsp_types::{
    CodeAction, CodeActionKind, Command, Diagnostic, DiagnosticSeverity, Location, NumberOrString,
    Position, PublishDiagnosticsParams, Range, TextEdit, WorkspaceEdit,
};
use serde_json::{Value, json};

fn main() {
    // Minimal stdio LSP server used for workspace tests. Not shipped or used in production.
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();

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
                                "documentSymbolProvider": true,
                                "workspaceSymbolProvider": true,
                                "renameProvider": true,
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
