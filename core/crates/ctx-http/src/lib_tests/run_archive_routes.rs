use super::*;
use chrono::Utc;
use ctx_core::models::{
    ArchiveVisibility, ExecutionEnvironment, Message, MessageDelivery, MessageRole,
    RetentionPolicyRef, RunArchiveIngestBatch, RunArchiveIngestCursor, RunArchiveIngestScope,
    RunArchiveState, RunRecord, RunStatus, SessionEventType,
};

#[tokio::test]
async fn run_archive_routes_build_and_acknowledge_org_visible_batch() {
    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let task = store
        .create_task(workspace.id, "team archive".to_string(), None)
        .await
        .unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            git_repo.path().to_string_lossy().into_owned(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "fake-model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let now = Utc::now();
    let run_id = ctx_core::ids::RunId::new();
    let org_id = ctx_core::ids::OrgId::new();
    let account_id = ctx_core::ids::AccountId::new();
    let turn_id = ctx_core::ids::TurnId::new();
    store
        .upsert_run(RunRecord {
            id: run_id,
            session_id: session.id,
            task_id: task.id,
            workspace_id: workspace.id,
            worktree_id: session.worktree_id,
            parent_run_id: None,
            account_id: Some(account_id),
            org_id: Some(org_id),
            run_grant_id: None,
            status: RunStatus::Completed,
            archive_state: RunArchiveState::Archived,
            archive_visibility: ArchiveVisibility::OrgEvidence,
            retention_policy: Some(RetentionPolicyRef {
                policy_key: "team-default".into(),
                legal_hold_key: None,
            }),
            created_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            archived_at: Some(now),
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .insert_message(Message {
            id: ctx_core::ids::MessageId::new(),
            session_id: session.id,
            task_id: task.id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: None,
            role: MessageRole::Assistant,
            content: "reviewed /Users/example-user/project/.env".into(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: None,
            created_at: now,
        })
        .await
        .unwrap();
    store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            json!({
                "kind": "archive_evidence",
                "api_key": "sk-test-placeholder-1234567890"
            }),
        )
        .await
        .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_batch?max_items=25",
            workspace.id.0, run_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let batch: Option<RunArchiveIngestBatch> = serde_json::from_slice(&body).unwrap();
    let batch = batch.expect("org-visible run should produce archive batch");
    assert_eq!(batch.run.id, run_id);
    assert_eq!(batch.run.workspace_id, workspace.id);
    assert_eq!(batch.scope, RunArchiveIngestScope::Evidence);
    assert_eq!(batch.messages.len(), 1);
    let serialized = serde_json::to_string(&batch).unwrap();
    assert!(!serialized.contains("/Users/example-user"));
    assert!(!serialized.contains("sk-test-placeholder-1234567890"));

    let mut tampered_batch = batch.clone();
    tampered_batch.to.session_event_seq += 100;
    tampered_batch.to.audit_event_seq += 100;
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_ack",
            workspace.id.0, run_id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&tampered_batch).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_ack",
            workspace.id.0, run_id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&batch).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let cursor: RunArchiveIngestCursor = serde_json::from_slice(&body).unwrap();
    assert_eq!(cursor.run_id, run_id);
    assert_eq!(cursor.watermark, batch.to);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/runs/{}/archive/ingest_batch",
            workspace.id.0, run_id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let batch: Option<RunArchiveIngestBatch> = serde_json::from_slice(&body).unwrap();
    assert!(batch.is_none());
}
