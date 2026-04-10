use std::path::Path;
use std::time::Duration;

use ctx_lsp::{LspManager, LspManagerConfig};
use lsp_types::{Position, Range};
use serde_json::Value;

async fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, contents).await.unwrap();
}

fn test_server_bin() -> String {
    let bin = env!("CARGO_BIN_EXE_ctx-lsp-test-server");
    let path = Path::new(bin);
    if path.is_absolute() {
        return bin.to_string();
    }

    let test_srcdir =
        std::env::var("TEST_SRCDIR").expect("relative test server path requires TEST_SRCDIR");
    let test_workspace =
        std::env::var("TEST_WORKSPACE").expect("relative test server path requires TEST_WORKSPACE");
    Path::new(&test_srcdir)
        .join(test_workspace)
        .join(path)
        .to_string_lossy()
        .to_string()
}

#[tokio::test]
async fn test_server_produces_diagnostics() {
    let bin = test_server_bin();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("src/lib.rs");
    write_file(&file, "pub fn x() { }\n").await;

    let mgr = LspManager::new(LspManagerConfig {
        enabled: true,
        rust_command: bin.to_string(),
        rust_args: vec![],
        diagnostics_wait: Duration::from_secs(2),
        ..Default::default()
    });

    let diags = mgr.diagnostics_for_file(root, &file).await.unwrap();
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0].message.contains("Intentional diagnostic"),
        "unexpected message: {}",
        diags[0].message
    );
}

#[tokio::test]
async fn test_server_apply_edit_is_applied_to_disk() {
    let bin = test_server_bin();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("src/apply_edit.rs");
    write_file(&file, "pub fn x() { }\n").await;

    let mgr = LspManager::new(LspManagerConfig {
        enabled: true,
        rust_command: bin.to_string(),
        rust_args: vec![],
        diagnostics_wait: Duration::from_secs(2),
        ..Default::default()
    });

    let _ = mgr.diagnostics_for_file(root, &file).await.unwrap();

    let contents = tokio::fs::read_to_string(&file).await.unwrap();
    assert!(
        contents.starts_with("// didOpen applyEdit\n"),
        "expected applyEdit to modify file, got:\n{contents}"
    );
}

#[tokio::test]
async fn test_server_supports_semantic_actions() {
    let bin = test_server_bin();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("src/lib.rs");
    write_file(&file, "pub fn x() { }\n").await;

    let mgr = LspManager::new(LspManagerConfig {
        enabled: true,
        rust_command: bin.to_string(),
        rust_args: vec![],
        diagnostics_wait: Duration::from_secs(1),
        ..Default::default()
    });

    let def = mgr
        .definition(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(def.is_array());

    let tdef = mgr
        .type_definition(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(tdef.is_array());

    let impls = mgr
        .implementation(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(impls.is_array());

    let refs = mgr
        .references(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
            true,
        )
        .await
        .unwrap();
    assert_eq!(refs.len(), 1);

    let hover = mgr
        .hover(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(hover.is_object());

    let sig = mgr
        .signature_help(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(sig.is_object());

    let comp = mgr
        .completion(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(comp.is_object());

    let edit = mgr
        .rename(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
            "new".into(),
        )
        .await
        .unwrap();
    assert!(edit.changes.is_some());

    let fmt = mgr.format_document(root, &file).await.unwrap();
    assert!(!fmt.is_empty());

    let actions = mgr
        .code_actions(
            root,
            &file,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            },
            vec![],
        )
        .await
        .unwrap();
    assert!(actions.is_array());

    let folds = mgr.folding_ranges(root, &file).await.unwrap();
    assert!(folds.is_array());

    let linked = mgr
        .linked_editing_range(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(linked.is_object());

    let sem = mgr.semantic_tokens_full(root, &file).await.unwrap();
    assert!(sem.is_object());

    let sem_delta = mgr
        .semantic_tokens_delta(root, &file, "1".to_string())
        .await
        .unwrap();
    assert!(sem_delta.is_object());

    let resolved = mgr
        .workspace_symbol_resolve(root, Value::Object(Default::default()))
        .await
        .unwrap();
    assert!(resolved.is_object());
}

#[tokio::test]
async fn test_server_supports_agent_text_only_extensions() {
    let bin = test_server_bin();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("src/lib.rs");
    write_file(&file, "pub fn x() { }\n").await;

    let mgr = LspManager::new(LspManagerConfig {
        enabled: true,
        rust_command: bin.to_string(),
        rust_args: vec![],
        diagnostics_wait: Duration::from_secs(1),
        execute_commands_enabled: true,
        execute_command_allowlist: vec!["ctx.test.fixAll".to_string()],
        ..Default::default()
    });

    // Streaming diagnostics subscription (broadcast).
    let mut diag_rx = mgr
        .subscribe_diagnostics_for_file(root, &file)
        .await
        .unwrap();
    mgr.sync_document_text(root, &file, "pub fn x() { }\n".to_string())
        .await
        .unwrap();
    let diag = tokio::time::timeout(std::time::Duration::from_secs(1), diag_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(diag
        .diagnostics
        .first()
        .map(|d| d.message.contains("Intentional diagnostic"))
        .unwrap_or(false));

    let comp = mgr
        .completion(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    let item = comp
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let resolved = mgr.completion_resolve(root, &file, item).await.unwrap();
    assert!(resolved.is_object());

    let actions = mgr
        .code_actions(
            root,
            &file,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            },
            vec![],
        )
        .await
        .unwrap();
    let action = actions
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let resolved_action = mgr.code_action_resolve(root, &file, action).await.unwrap();
    assert!(resolved_action.is_object());

    let inlays = mgr
        .inlay_hints(
            root,
            &file,
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            },
        )
        .await
        .unwrap();
    assert!(inlays.is_array());

    let highlights = mgr
        .document_highlight(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(highlights.is_array());

    let sel = mgr
        .selection_ranges(
            root,
            &file,
            vec![Position {
                line: 0,
                character: 0,
            }],
        )
        .await
        .unwrap();
    assert!(sel.is_array());

    let ch = mgr
        .call_hierarchy_prepare(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    let item = ch
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let incoming = mgr
        .call_hierarchy_incoming(root, &file, item.clone())
        .await
        .unwrap();
    assert!(incoming.is_array());
    let outgoing = mgr
        .call_hierarchy_outgoing(root, &file, item)
        .await
        .unwrap();
    assert!(outgoing.is_array());

    let lenses = mgr.code_lens(root, &file).await.unwrap();
    let lens = lenses
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let resolved_lens = mgr.code_lens_resolve(root, &file, lens).await.unwrap();
    assert!(resolved_lens.is_object());

    let (_result, edit) = mgr
        .execute_command_for_file(root, &file, "ctx.test.fixAll".into(), vec![])
        .await
        .unwrap();
    assert!(edit.is_some(), "expected applyEdit captured");

    let prep = mgr
        .prepare_rename(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    assert!(prep.is_object() || prep.is_null());

    let links = mgr.document_links(root, &file).await.unwrap();
    assert!(links.is_array());
    let link = links
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let resolved_link = mgr.document_link_resolve(root, &file, link).await.unwrap();
    assert!(resolved_link.is_object());

    let tokens = mgr.semantic_tokens_full(root, &file).await.unwrap();
    assert!(tokens.is_object());

    let th = mgr
        .type_hierarchy_prepare(
            root,
            &file,
            Position {
                line: 0,
                character: 0,
            },
        )
        .await
        .unwrap();
    let item = th
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    let sups = mgr
        .type_hierarchy_supertypes(root, &file, item.clone())
        .await
        .unwrap();
    assert!(sups.is_array());
    let subs = mgr
        .type_hierarchy_subtypes(root, &file, item)
        .await
        .unwrap();
    assert!(subs.is_array());
}
