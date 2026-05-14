use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::{ArtifactId, MessageId, SessionId};
use ctx_core::models::{Message, MessageDelivery, MessageRole, VcsKind};
use ctx_daemon::daemon::DaemonState;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderRecommendedAction, ProviderUsability, ProviderUsabilityStatus,
};
use ctx_providers::fake::FakeProviderAdapter;
use serde_json::json;
use uuid::Uuid;

mod common;

struct SessionFixture {
    session_id: SessionId,
}

fn request_shutdown(state: &Arc<DaemonState>) {
    let _ = state.core.shutdown_tx.send(());
}

async fn setup_state() -> (tempfile::TempDir, Arc<DaemonState>, common::TestServer) {
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
        .upsert_provider_status("fake".into(), status)
        .await;
    let server = common::spawn_http_server(common::router(state.clone())).await;
    (data_dir, state, server)
}

async fn create_workspace_session(
    state: &Arc<DaemonState>,
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
        session_id: session.id,
    }
}

#[tokio::test]
async fn artifact_route_is_session_scoped() {
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

    let resp = server
        .client
        .get(format!(
            "{}/api/sessions/{}/artifacts/{}",
            server.base_url, workspace_b.session_id.0, artifact_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"artifact-body");
    request_shutdown(&state);
}

#[tokio::test]
async fn quicktime_artifact_upload_is_accepted() {
    let (_data_dir, state, server) = setup_state().await;
    let repo = common::init_git_repo(&[("README.md", "a")]).await;
    let workspace = create_workspace_session(&state, "a", repo.path()).await;

    let artifact_path = repo.path().join("artifact.mov");
    tokio::fs::write(&artifact_path, b"quicktime-body")
        .await
        .unwrap();

    let resp = server
        .client
        .post(format!(
            "{}/api/sessions/{}/artifacts",
            server.base_url, workspace.session_id.0
        ))
        .json(&json!({
            "artifacts": [
                {
                    "absolute_file_path": artifact_path.to_string_lossy().to_string(),
                    "mime_type": "video/quicktime"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let artifacts: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(artifacts[0]["mime_type"].as_str(), Some("video/quicktime"));
    assert_eq!(artifacts[0]["name"].as_str(), Some("artifact.mov"));
    request_shutdown(&state);
}

#[tokio::test]
async fn message_delete_route_is_session_scoped() {
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
    let message_id = message.id;

    let resp = server
        .client
        .delete(format!(
            "{}/api/sessions/{}/messages/{}",
            server.base_url, workspace_b.session_id.0, message_id.0
        ))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    assert_eq!(status, StatusCode::NO_CONTENT, "unexpected body: {body}");
    assert!(store.get_message(message_id).await.unwrap().is_none());
    request_shutdown(&state);
}

#[tokio::test]
async fn subagent_invocation_route_is_session_scoped() {
    let (_data_dir, state, server) = setup_state().await;
    let repo_a = common::init_git_repo(&[("README.md", "a")]).await;
    let repo_b = common::init_git_repo(&[("README.md", "b")]).await;
    let _workspace_a = create_workspace_session(&state, "a", repo_a.path()).await;
    let workspace_b = create_workspace_session(&state, "b", repo_b.path()).await;

    let resp = server
        .client
        .post(format!(
            "{}/api/mcp/sessions/{}/spawn_agent",
            server.base_url, workspace_b.session_id.0
        ))
        .json(&json!({
            "worktree": "inherit",
            "prompt": "hello",
            "task_label": "Worker",
            "harness": "fake",
            "model": "fake-model"
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

    let resp = server
        .client
        .get(format!(
            "{}/api/sessions/{}/subagent_invocations/{}",
            server.base_url, workspace_b.session_id.0, invocation_id
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
    request_shutdown(&state);
}
