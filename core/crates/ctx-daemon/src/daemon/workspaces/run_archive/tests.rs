use ctx_core::ids::{RunId, WorkspaceId};
use ctx_route_contracts::run_archive::{
    requested_batch_item_limit, RunArchiveRouteErrorKind, RunArchiveRouteParams,
};

#[test]
fn requested_batch_item_limit_defaults_and_accepts_boundaries() {
    assert_eq!(requested_batch_item_limit(None).unwrap(), 250);
    assert_eq!(requested_batch_item_limit(Some(1)).unwrap(), 1);
    assert_eq!(requested_batch_item_limit(Some(1_000)).unwrap(), 1_000);
}

#[test]
fn requested_batch_item_limit_rejects_out_of_range_values() {
    for max_items in [0, 1_001] {
        let error = requested_batch_item_limit(Some(max_items)).unwrap_err();
        assert_eq!(error.kind(), RunArchiveRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "max_items must be between 1 and 1000");
    }
}

#[test]
fn route_params_reject_invalid_workspace_id() {
    let params = RunArchiveRouteParams::new("not-a-uuid", RunId::new().0.to_string());

    let error = params.parse().unwrap_err();
    assert_eq!(error.kind(), RunArchiveRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid workspace id");
}

#[test]
fn route_params_reject_invalid_run_id() {
    let params = RunArchiveRouteParams::new(WorkspaceId::new().0.to_string(), "not-a-uuid");

    let error = params.parse().unwrap_err();
    assert_eq!(error.kind(), RunArchiveRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "invalid run id");
}
