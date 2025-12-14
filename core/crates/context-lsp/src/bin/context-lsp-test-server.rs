use std::io::{Read, Write};

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, PublishDiagnosticsParams, Range};
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
                                "textDocumentSync": 1
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

        // Ignore all other notifications/requests.
        if let Some(id) = msg.get("id").cloned() {
            write_response(
                &mut output,
                json!({
                    "jsonrpc":"2.0",
                    "id": id,
                    "result": null
                }),
            );
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

