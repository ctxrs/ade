use std::path::Path;
use std::time::Duration;

use context_lsp::{LspManager, LspManagerConfig};
use lsp_types::{Position, Range};

async fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, contents).await.unwrap();
}

#[tokio::test]
async fn test_server_produces_diagnostics() {
    let bin = env!("CARGO_BIN_EXE_context-lsp-test-server");

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
async fn test_server_supports_semantic_actions() {
    let bin = env!("CARGO_BIN_EXE_context-lsp-test-server");

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
        .definition(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(def.is_array());

    let tdef = mgr
        .type_definition(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(tdef.is_array());

    let impls = mgr
        .implementation(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(impls.is_array());

    let refs = mgr
        .references(root, &file, Position { line: 0, character: 0 }, true)
        .await
        .unwrap();
    assert_eq!(refs.len(), 1);

    let hover = mgr
        .hover(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(hover.is_object());

    let sig = mgr
        .signature_help(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(sig.is_object());

    let comp = mgr
        .completion(root, &file, Position { line: 0, character: 0 })
        .await
        .unwrap();
    assert!(comp.is_object());

    let edit = mgr
        .rename(root, &file, Position { line: 0, character: 0 }, "new".into())
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
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 1 },
            },
            vec![],
        )
        .await
        .unwrap();
    assert!(actions.is_array());
}
