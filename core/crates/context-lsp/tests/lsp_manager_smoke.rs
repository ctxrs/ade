use std::path::Path;
use std::time::Duration;

use context_lsp::{LspManager, LspManagerConfig};

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
    });

    let diags = mgr.diagnostics_for_file(root, &file).await.unwrap();
    assert_eq!(diags.len(), 1);
    assert!(
        diags[0].message.contains("Intentional diagnostic"),
        "unexpected message: {}",
        diags[0].message
    );
}

