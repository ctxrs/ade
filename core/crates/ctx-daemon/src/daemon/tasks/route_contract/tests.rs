use super::*;
use ctx_core::models::ExecutionEnvironment;

#[test]
fn route_error_kind_preserves_non_http_internal_status_categories() {
    let storage_error =
        anyhow::anyhow!("Insufficient storage capacity for creating an isolated task worktree");
    assert_eq!(
        route_error_kind_for_internal_error(&storage_error),
        TaskRouteErrorKind::InsufficientStorage
    );

    let policy_error = ctx_settings_service::HostExecutionPolicy::SandboxOnly
        .validate_execution_environment(ExecutionEnvironment::Host)
        .expect_err("host execution should be denied");
    assert_eq!(
        route_error_kind_for_internal_error(&policy_error),
        TaskRouteErrorKind::Forbidden
    );

    assert_eq!(
        route_error_kind_for_internal_error(&anyhow::anyhow!("plain internal failure")),
        TaskRouteErrorKind::Internal
    );
}
