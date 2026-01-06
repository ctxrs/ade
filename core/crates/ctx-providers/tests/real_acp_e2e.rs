use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::mpsc;

use ctx_core::models::SessionEventType;
use ctx_providers::adapters::{ProviderAdapter, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_providers::tier1::Tier1AcpAdapter;

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("note.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn run_and_collect(
    adapter: &dyn ProviderAdapter,
    workdir: &Path,
    prompt: &str,
) -> Vec<NormalizedEvent> {
    let (tx, mut rx) = mpsc::channel::<NormalizedEvent>(1024);
    let handle = adapter
        .run(
            TurnInput {
                content: prompt.to_string(),
                model_id: "test-model".to_string(),
                attachments: vec![],
                context_blocks: vec![],
            },
            workdir.to_path_buf(),
            HashMap::new(),
            tx,
        )
        .await
        .unwrap();

    let events = tokio::time::timeout(Duration::from_secs(240), async move {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
        events
    })
    .await
    .expect("timed out waiting for provider events");

    let _ = handle.done.await;
    events
}

fn tool_call_id(ev: &NormalizedEvent) -> Option<String> {
    ev.payload_json
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn assert_has_correlated_tool_call(events: &[NormalizedEvent]) {
    if events
        .iter()
        .any(|e| matches!(e.event_type, SessionEventType::Error))
    {
        panic!("provider emitted error event(s): {events:#?}");
    }

    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done)),
        "expected at least one Done event, got {events:#?}"
    );

    let tool_calls: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolCall))
        .filter_map(tool_call_id)
        .collect();
    let tool_results: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolResult))
        .filter_map(tool_call_id)
        .collect();

    let correlated = tool_calls.intersection(&tool_results).next().cloned();
    assert!(
        correlated.is_some(),
        "expected at least one correlated ToolCall/ToolResult pair; calls={tool_calls:?} results={tool_results:?}\nfull events={events:#?}"
    );
}

const PROMPT: &str = r#"
In this repository:
1) Use your shell tool to run: ls -1
2) Prepend the line "// e2e-test" to the file note.txt (keep existing content).

Important: Do not guess tool outputs; you MUST use tools.
Reply with just: done
"#;

#[tokio::test]
#[ignore]
async fn codex_real_acp_produces_tool_events() {
    which::which("codex-acp").expect("codex-acp binary not found on PATH");
    let repo = setup_git_repo().await;

    let adapter = Tier1AcpAdapter::codex();
    let events = run_and_collect(&adapter, repo.path(), PROMPT).await;
    assert_has_correlated_tool_call(&events);
}

#[tokio::test]
#[ignore]
async fn claude_real_acp_produces_tool_events() {
    which::which("claude-code-acp").expect("claude-code-acp binary not found on PATH");
    let repo = setup_git_repo().await;

    let adapter = Tier1AcpAdapter::claude();
    let events = run_and_collect(&adapter, repo.path(), PROMPT).await;
    assert_has_correlated_tool_call(&events);
}

#[tokio::test]
#[ignore]
async fn gemini_real_acp_produces_tool_events() {
    which::which("gemini").expect("gemini binary not found on PATH");
    let repo = setup_git_repo().await;

    let adapter = Tier1AcpAdapter::gemini();
    let events = run_and_collect(&adapter, repo.path(), PROMPT).await;
    assert_has_correlated_tool_call(&events);
}
