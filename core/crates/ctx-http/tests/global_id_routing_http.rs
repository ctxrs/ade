use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::{ArtifactId, MessageId, SessionId};
use ctx_core::models::{Message, MessageDelivery, MessageRole, VcsKind};
use ctx_http::daemon::AppState;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderRecommendedAction, ProviderUsability, ProviderUsabilityStatus,
};
use ctx_providers::fake::FakeProviderAdapter;
use serde_json::json;
use uuid::Uuid;

mod common;

struct SessionFixture {
    workspace_id: ctx_core::ids::WorkspaceId,
    session_id: SessionId,
}

async fn setup_state() -> (tempfile::TempDir, Arc<AppState>, common::TestServer) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let mut status = FakeProviderAdapter::new().inspect().await.unwrap();
    status.usability = ProviderUsability {
        usable: true,
        status: ProviderUsabilityStatus::Ready,
        reason_code: None,
        reason: None,
        blocking_provider_ids: Vec::new(),
        recommended_action: ProviderRecommendedAction::None,
    };
    state
        .providers
        .statuses
        .lock()
        .await
        .insert("fake".into(), status);
    let server = common::spawn_http_server(common::router(state.clone())).await;
    (data_dir, state, server)
}

async fn create_workspace_session(
    state: &Arc<AppState>,
    name: &str,
    repo_root: &Path,
) -> SessionFixture {
    let workspace = state
        .global_store()
        .create_workspace(
            name.to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let vcs = ctx_fs::vcs::driver_for_path(repo_root).await.unwrap();
    let base_commit = vcs.rev_parse_head(repo_root).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            repo_root.to_string_lossy().to_string(),
            base_commit,
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, name.to_string(), None)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".into(),
            "fake-model".into(),
            "assistant".into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

    SessionFixture {
        workspace_id: workspace.id,
        session_id: session.id,
    }
}

#[tokio::test]
async fn artifact_route_uses_global_routing_index() {
    let (_data_dir, state, server) = setup_state().await;
    let repo_a = common::init_git_repo(&[("README.md", "a")]).await;
    let repo_b = common::init_git_repo(&[("README.md", "b")]).await;
    let _workspace_a = create_workspace_session(&state, "a", repo_a.path()).await;
    let workspace_b = create_workspace_session(&state, "b", repo_b.path()).await;

    let artifact_path = repo_b.path().join("artifact.txt");
    tokio::fs::write(&artifact_path, b"artifact-body")
        .await
        .unwrap();

    let resp = server
        .client
        .post(format!(
            "{}/api/sessions/{}/artifacts",
            server.base_url, workspace_b.session_id.0
        ))
        .json(&json!({
            "artifacts": [
                { "absolute_file_path": artifact_path.to_string_lossy().to_string() }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let artifacts: serde_json::Value = resp.json().await.unwrap();
    let artifact_id = artifacts[0]["id"].as_str().unwrap();
    let artifact_id = ArtifactId(Uuid::parse_str(artifact_id).unwrap());

    let routed_workspace = state
        .global_store()
        .get_workspace_id_for_artifact(artifact_id)
        .await
        .unwrap();
    assert_eq!(routed_workspace, Some(workspace_b.workspace_id));

    let resp = server
        .client
        .get(format!(
            "{}/api/artifacts/{}",
            server.base_url, artifact_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"artifact-body");
}

#[tokio::test]
async fn message_delete_route_uses_global_routing_index() {
    let (_data_dir, state, server) = setup_state().await;
    let repo_a = common::init_git_repo(&[("README.md", "a")]).await;
    let repo_b = common::init_git_repo(&[("README.md", "b")]).await;
    let _workspace_a = create_workspace_session(&state, "a", repo_a.path()).await;
    let workspace_b = create_workspace_session(&state, "b", repo_b.path()).await;
    let store = state
        .store_for_session(workspace_b.session_id)
        .await
        .unwrap();
    let session = store
        .get_session(workspace_b.session_id)
        .await
        .unwrap()
        .unwrap();
    let message = store
        .insert_message(Message {
            id: MessageId::new(),
            session_id: session.id,
            task_id: session.task_id,
            run_id: None,
            turn_id: None,
            turn_sequence: None,
            order_seq: None,
            role: MessageRole::User,
            content: "queued".to_string(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Queued,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_message_index(message.id, workspace_b.workspace_id)
        .await
        .unwrap();
    let message_id = message.id;

    let routed_workspace = state
        .global_store()
        .get_workspace_id_for_message(message_id)
        .await
        .unwrap();
    assert_eq!(routed_workspace, Some(workspace_b.workspace_id));
    let routed_store = state.store_for_message(message_id).await.unwrap();
    let routed_message = routed_store.get_message(message_id).await.unwrap().unwrap();
    assert!(matches!(routed_message.delivery, MessageDelivery::Queued));
    assert!(routed_message.delivered_at.is_none());

    let resp = server
        .client
        .delete(format!("{}/api/messages/{}", server.base_url, message_id.0))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert_eq!(status, StatusCode::NO_CONTENT, "unexpected body: {body}");
    assert!(state
        .global_store()
        .get_workspace_id_for_message(message_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn subagent_invocation_route_uses_global_routing_index() {
    let (_data_dir, state, server) = setup_state().await;
    let repo_a = common::init_git_repo(&[("README.md", "a")]).await;
    let repo_b = common::init_git_repo(&[("README.md", "b")]).await;
    let _workspace_a = create_workspace_session(&state, "a", repo_a.path()).await;
    let workspace_b = create_workspace_session(&state, "b", repo_b.path()).await;

    let resp = server
        .client
        .post(format!(
            "{}/api/mcp/sessions/{}/subagent_init",
            server.base_url, workspace_b.session_id.0
        ))
        .json(&json!({
            "worktree": "inherit",
            "agents": [
                { "prompt": "hello", "label": "Worker", "harness": "fake", "model": "fake-model" }
            ]
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    if status != StatusCode::OK {
        panic!("unexpected status {status} body: {body}");
    }

    let resp = server
        .client
        .get(format!(
            "{}/api/sessions/{}/subagent_invocations",
            server.base_url, workspace_b.session_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let invocations: serde_json::Value = resp.json().await.unwrap();
    let invocation_id = invocations[0]["id"].as_str().unwrap().to_string();

    let routed_workspace = state
        .global_store()
        .get_workspace_id_for_subagent_invocation(&invocation_id)
        .await
        .unwrap();
    assert_eq!(routed_workspace, Some(workspace_b.workspace_id));

    let resp = server
        .client
        .get(format!(
            "{}/api/subagent_invocations/{}",
            server.base_url, invocation_id
        ))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    if status != StatusCode::OK {
        panic!("unexpected status {status} body: {body}");
    }
    let invocation: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(invocation["id"].as_str().unwrap(), invocation_id);
}
