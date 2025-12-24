use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use flate2::Compression;
use serde_json::json;
use tower::ServiceExt;

use context_http::api;
use context_http::daemon::AppState;
use context_store::Store;

fn make_tar_gz_with_file(dst: &Path, inner_path: &str, contents: &[u8]) {
    let tar_gz = std::fs::File::create(dst).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, inner_path, contents).unwrap();
    tar.finish().unwrap();
}

#[tokio::test]
async fn lsp_catalog_install_from_file_url_updates_config() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());

    let data_dir = tempfile::tempdir().unwrap();
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let db_path = db_dir.join("db.sqlite");
    let store = Store::open(&db_path).await.unwrap();

    // Provide a local catalog entry that downloads from file:// and extracts a tar.gz.
    let lsp_dir = data_dir.path().join("lsp");
    tokio::fs::create_dir_all(&lsp_dir).await.unwrap();
    let archive_path = lsp_dir.join("fake_ls.tar.gz");
    make_tar_gz_with_file(
        &archive_path,
        "fake/bin/fake-ls",
        b"#!/usr/bin/env bash\necho ok\n",
    );
    let archive_url = url::Url::from_file_path(&archive_path).unwrap().to_string();
    let catalog_path = lsp_dir.join("catalog.json");
    let catalog = json!({
        "version": 1,
        "servers": [{
            "id": "fake-ls",
            "title": "Fake LS",
            "language_id": "rust",
            "args": ["--stdio"],
            "extensions": ["rs"],
            "install": {
                "kind": "url_binary",
                "version": "0.0.1",
                "targets": {
                    "linux-x64": { "url": archive_url, "archive": "tar_gz", "bin_path": "bin/fake-ls" },
                    "linux-arm64": { "url": archive_url, "archive": "tar_gz", "bin_path": "bin/fake-ls" },
                    "darwin-x64": { "url": archive_url, "archive": "tar_gz", "bin_path": "bin/fake-ls" },
                    "darwin-arm64": { "url": archive_url, "archive": "tar_gz", "bin_path": "bin/fake-ls" }
                }
            }
        }]
    });
    tokio::fs::write(
        &catalog_path,
        serde_json::to_string_pretty(&catalog).unwrap(),
    )
    .await
    .unwrap();

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        store.clone(),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.start_workspace_index_listener();
    let app = api::router(state.clone());

    // List catalog.
    let req = Request::builder()
        .method("GET")
        .uri("/api/lsp/catalog")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let catalog_list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(catalog_list
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e.get("id").and_then(|v| v.as_str()) == Some("fake-ls")));

    // Trigger install.
    let req = Request::builder()
        .method("POST")
        .uri("/api/lsp/catalog/fake-ls/install")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let install_id = resp["install_id"].as_str().unwrap();

    // Wait for install to finish.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/providers/install/{install_id}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let state_kind = info["state"].as_str().unwrap_or("");
        if state_kind != "running" {
            assert_eq!(state_kind, "succeeded", "install failed: {info:#}");
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("install did not finish: {info:#}");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Managed config updated (points rust to extracted fake-ls path).
    let cfg_path = lsp_dir.join("lsp_servers.json");
    let txt = tokio::fs::read_to_string(&cfg_path).await.unwrap();
    let cfg: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let cmd = cfg["servers"]["rust"]["command"].as_str().unwrap_or("");
    assert!(cmd.contains("fake-ls"), "unexpected command: {cmd}");

    // Catalog status now reports installed+enabled.
    let req = Request::builder()
        .method("GET")
        .uri("/api/lsp/catalog")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let catalog_list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entry = catalog_list
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some("fake-ls"))
        .unwrap();
    assert_eq!(entry["installed"].as_bool(), Some(true));
    assert_eq!(entry["enabled"].as_bool(), Some(true));
}
