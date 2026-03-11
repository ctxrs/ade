use serde_json::json;

use super::CreateSessionReq;

#[test]
fn create_session_req_accepts_execution_environment() {
    let req: CreateSessionReq = serde_json::from_value(json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "execution_environment": "worktree",
    }))
    .unwrap();

    assert_eq!(req.execution_environment.as_deref(), Some("worktree"));
}

#[test]
fn create_session_req_accepts_legacy_env_target_alias() {
    let req: CreateSessionReq = serde_json::from_value(json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "env_target": "local",
    }))
    .unwrap();

    assert_eq!(req.execution_environment.as_deref(), Some("local"));
}
