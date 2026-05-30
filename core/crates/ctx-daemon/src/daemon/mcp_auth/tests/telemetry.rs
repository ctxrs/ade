use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_mcp_auth::{McpAuthCapabilities, McpAuthContext, McpAuthRegistry};
use ctx_observability::ops_events::OpsEvents;
use serde_json::Value;
use tokio::time::{sleep, Duration};

use super::super::{
    emit_mcp_token_denied_with_ops, issue_provider_session_mcp_token_with_capabilities_parts,
    revoke_provider_session_mcp_token_parts,
};

#[tokio::test]
async fn mcp_token_events_preserve_issue_replacement_revoke_and_denial_shape() {
    let data_dir = tempfile::tempdir().expect("create tempdir");
    let ops_events = OpsEvents::new(data_dir.path().to_path_buf());
    let mcp_auth = McpAuthRegistry::new();
    let session_id = SessionId::new();
    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();

    let first_token = issue_provider_session_mcp_token_with_capabilities_parts(
        &mcp_auth,
        &ops_events,
        session_id,
        workspace_id,
        worktree_id,
        McpAuthCapabilities::provider_session(),
    )
    .await;
    let second_token = issue_provider_session_mcp_token_with_capabilities_parts(
        &mcp_auth,
        &ops_events,
        session_id,
        workspace_id,
        worktree_id,
        McpAuthCapabilities::provider_session(),
    )
    .await;
    assert_ne!(first_token, second_token);

    assert!(
        revoke_provider_session_mcp_token_parts(&mcp_auth, &ops_events, &second_token).await,
        "explicit revoke should remove the active token"
    );
    assert!(
        !revoke_provider_session_mcp_token_parts(&mcp_auth, &ops_events, &second_token).await,
        "revoke miss should not emit another event"
    );

    let denied_context = McpAuthContext::provider_session(
        session_id,
        workspace_id,
        worktree_id,
        McpAuthCapabilities::provider_turn_default(),
    );
    emit_mcp_token_denied_with_ops(
        &ops_events,
        denied_context,
        "POST",
        "/api/sessions/example/subagents",
        "scope_or_capability_mismatch",
    );

    let events = read_ops_events(data_dir.path(), 5).await;
    let names: Vec<_> = events
        .iter()
        .map(|event| event["event"].as_str().expect("event name"))
        .collect();
    assert_eq!(
        names,
        [
            "mcp_token_issued",
            "mcp_token_revoked",
            "mcp_token_issued",
            "mcp_token_revoked",
            "mcp_token_denied",
        ]
    );

    assert_mcp_event_scope(&events[0], "info", session_id, workspace_id, worktree_id);
    assert_eq!(events[0]["meta"]["detail"]["reason"], "provider_session");
    assert_eq!(events[1]["meta"]["detail"]["reason"], "replaced");
    assert_eq!(events[1]["meta"]["detail"]["count"], 1);
    assert_eq!(events[3]["meta"]["detail"]["reason"], "explicit");
    assert_eq!(events[4]["level"], "warn");
    assert_eq!(events[4]["meta"]["detail"]["method"], "POST");
    assert_eq!(
        events[4]["meta"]["detail"]["path"],
        "/api/sessions/example/subagents"
    );
    assert_eq!(
        events[4]["meta"]["detail"]["reason"],
        "scope_or_capability_mismatch"
    );
    assert_eq!(
        events[4]["meta"]["capabilities"],
        serde_json::json!(["subagents", "artifacts", "merge_queue_submit"])
    );

    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        read_ops_events(data_dir.path(), 5).await.len(),
        5,
        "revoke miss must not emit a sixth event"
    );
}

fn assert_mcp_event_scope(
    event: &Value,
    level: &str,
    session_id: SessionId,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) {
    assert_eq!(event["level"], level);
    assert_eq!(event["session_id"], session_id.0.to_string());
    assert_eq!(event["worktree_id"], worktree_id.0.to_string());
    assert_eq!(event["meta"]["workspace_id"], workspace_id.0.to_string());
    assert_eq!(
        event["meta"]["capabilities"],
        serde_json::json!(["subagents", "artifacts"])
    );
}

async fn read_ops_events(data_root: &std::path::Path, expected_len: usize) -> Vec<Value> {
    let log_dir = data_root.join("logs");
    for _ in 0..100 {
        let events = read_current_ops_events(&log_dir).await;
        if events.len() >= expected_len {
            return events;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "timed out waiting for {expected_len} ops events; found {:?}",
        read_current_ops_events(&log_dir).await
    );
}

async fn read_current_ops_events(log_dir: &std::path::Path) -> Vec<Value> {
    let Ok(mut entries) = tokio::fs::read_dir(log_dir).await else {
        return Vec::new();
    };
    let mut events = Vec::new();
    while let Some(entry) = entries.next_entry().await.expect("read log entry") {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with("ops-events-") || !file_name.ends_with(".jsonl") {
            continue;
        }
        let text = tokio::fs::read_to_string(entry.path())
            .await
            .expect("read ops log");
        events.extend(
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("parse ops event")),
        );
    }
    events
}
