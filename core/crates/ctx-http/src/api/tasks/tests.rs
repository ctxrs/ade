use serde_json::json;

use ctx_core::models::ExecutionEnvironment;

use super::CreateSessionReq;

#[test]
fn create_session_req_accepts_execution_environment() {
    let req: CreateSessionReq = serde_json::from_value(json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "execution_environment": "container_host_mounted",
    }))
    .unwrap();

    assert_eq!(
        req.execution_environment,
        Some(ExecutionEnvironment::ContainerHostMounted)
    );
}

#[test]
fn create_session_req_rejects_legacy_env_target_alias() {
    let err = serde_json::from_value::<CreateSessionReq>(json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "env_target": "local"
    }))
    .unwrap_err();

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn create_session_req_rejects_legacy_execution_environment_values() {
    let err = serde_json::from_value::<CreateSessionReq>(json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "execution_environment": "worktree"
    }))
    .unwrap_err();

    assert!(err.to_string().contains("unknown variant"));
}
