use std::io::{Read, Write};

use serde_json::{Value, json};

fn main() {
    // Minimal stdio LSP server used by context-http integration tests.
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
                                "referencesProvider": true,
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
                                    "newText": format!("// rename: {new_name}\\n")
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
                            "newText": "/* formatted */\\n"
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
                        "edit": {
                            "changes": {
                                uri: [{
                                    "range": {
                                        "start": {"line": 0, "character": 0},
                                        "end": {"line": 0, "character": 0}
                                    },
                                    "newText": "/* organize imports */\\n"
                                }]
                            }
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
                                    "newText": "// TODO\\n"
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
