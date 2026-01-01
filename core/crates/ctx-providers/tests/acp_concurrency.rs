use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;

use ctx_core::models::SessionEventType;
use ctx_providers::acp::{AcpAgentConfig, AcpClientConfig, AcpPromptRequest, AcpSessionPool};
use ctx_providers::events::NormalizedEvent;

async fn collect_until_done(mut rx: mpsc::Receiver<NormalizedEvent>) -> Vec<NormalizedEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
        if matches!(
            events.last().map(|e| &e.event_type),
            Some(SessionEventType::Done)
        ) {
            break;
        }
    }
    events
}

fn has_fragment(events: &[NormalizedEvent], fragment: &str) -> bool {
    events.iter().any(|ev| {
        matches!(ev.event_type, SessionEventType::AssistantChunk)
            && ev
                .payload_json
                .get("content_fragment")
                .and_then(|v| v.as_str())
                .map(|s| s.contains(fragment))
                .unwrap_or(false)
    })
}

fn acp_session_id(events: &[NormalizedEvent]) -> Option<String> {
    events.iter().find_map(|ev| {
        if matches!(ev.event_type, SessionEventType::Init) {
            ev.payload_json
                .get("acp_session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn acp_concurrency_routes_updates_per_session() {
    if which::which("node").is_err() {
        eprintln!("node not found; skipping ACP concurrency test");
        return;
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_mock_agent.mjs");

    let agent = AcpAgentConfig {
        provider_id: "mock".to_string(),
        command: "node".to_string(),
        args: vec![fixture.to_string_lossy().to_string()],
    };
    let pool = Arc::new(AcpSessionPool::new(agent));

    let client = AcpClientConfig {
        client_name: "ctx-test".to_string(),
        client_title: "ctx test".to_string(),
        client_version: "0.0.0".to_string(),
        client_capabilities: json!({}),
        mcp_servers: vec![],
    };

    let workdir = tempfile::tempdir().unwrap();
    let prompt = vec![json!({"type":"text","text":"hello"})];

    let (tx1, rx1) = mpsc::channel::<NormalizedEvent>(64);
    let (tx2, rx2) = mpsc::channel::<NormalizedEvent>(64);

    let (_cancel_tx1, cancel_rx1) = tokio::sync::oneshot::channel();
    let (_cancel_tx2, cancel_rx2) = tokio::sync::oneshot::channel();

    let req1 = AcpPromptRequest {
        session_key: "session-1".to_string(),
        client: client.clone(),
        prompt: prompt.clone(),
        workdir: workdir.path().to_path_buf(),
        env: HashMap::new(),
        event_sink: tx1,
        cancel_rx: cancel_rx1,
    };
    let req2 = AcpPromptRequest {
        session_key: "session-2".to_string(),
        client: client.clone(),
        prompt: prompt.clone(),
        workdir: workdir.path().to_path_buf(),
        env: HashMap::new(),
        event_sink: tx2,
        cancel_rx: cancel_rx2,
    };

    let pool1 = Arc::clone(&pool);
    let pool2 = Arc::clone(&pool);

    let prompt1 = tokio::spawn(async move { pool1.prompt(req1).await });
    let prompt2 = tokio::spawn(async move { pool2.prompt(req2).await });

    let events1 = tokio::spawn(collect_until_done(rx1));
    let events2 = tokio::spawn(collect_until_done(rx2));

    let join_all = async move {
        let r1 = prompt1.await.unwrap();
        let r2 = prompt2.await.unwrap();
        let e1 = events1.await.unwrap();
        let e2 = events2.await.unwrap();
        (r1, r2, e1, e2)
    };

    let (r1, r2, e1, e2) = tokio::time::timeout(Duration::from_secs(2), join_all)
        .await
        .expect("concurrent prompts should complete quickly");

    r1.expect("session-1 prompt failed");
    r2.expect("session-2 prompt failed");
    let session_id_1 = acp_session_id(&e1).expect("missing acp_session_id for session-1");
    let session_id_2 = acp_session_id(&e2).expect("missing acp_session_id for session-2");
    let fragment1 = format!("hello-{session_id_1}");
    let fragment2 = format!("hello-{session_id_2}");
    assert!(has_fragment(&e1, &fragment1));
    assert!(has_fragment(&e2, &fragment2));
}
