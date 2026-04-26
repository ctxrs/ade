use super::{
    load_or_create_install_id, telemetry_state_path, TelemetryDelivery, TelemetryEvent,
    TelemetryOriginRuntime, TelemetryPlane, TelemetryStateFile,
};

#[test]
fn session_interrupt_latency_event_sets_bounded_fields() {
    let event = TelemetryEvent::session_interrupt_latency(
        "codex-crp".to_string(),
        "gpt-5.2-codex".to_string(),
        Some("host".to_string()),
        Some("worktree".to_string()),
        1320,
        "1s_to_3s".to_string(),
    );

    assert_eq!(event.event_name, "session_interrupt_latency");
    assert_eq!(event.plane, TelemetryPlane::Product);
    assert_eq!(event.delivery, TelemetryDelivery::Remote);
    assert_eq!(event.origin_runtime, TelemetryOriginRuntime::Daemon);
    assert_eq!(
        event.properties.get("duration_ms"),
        Some(&serde_json::json!(1320))
    );
    assert_eq!(
        event.properties.get("duration_bucket"),
        Some(&serde_json::json!("1s_to_3s"))
    );
    assert_eq!(
        event.properties.get("status"),
        Some(&serde_json::json!("interrupted"))
    );
    assert_eq!(
        event.properties.get("success"),
        Some(&serde_json::json!(true))
    );
}

#[test]
fn local_only_marks_delivery_without_mutating_plane() {
    let event = TelemetryEvent::daemon_incident("renderer_backlog_sample").local_only();
    assert_eq!(event.event_name, "renderer_backlog_sample");
    assert_eq!(event.plane, TelemetryPlane::Incident);
    assert_eq!(event.delivery, TelemetryDelivery::LocalOnly);
}

#[tokio::test]
async fn load_or_create_install_id_creates_and_persists_missing_state() {
    let temp = tempfile::tempdir().expect("tempdir");

    let install_id = load_or_create_install_id(temp.path())
        .await
        .expect("missing telemetry state should create an install id");

    assert!(!install_id.trim().is_empty());

    let raw = tokio::fs::read_to_string(telemetry_state_path(temp.path()))
        .await
        .expect("telemetry state should be written");
    let stored: TelemetryStateFile =
        serde_json::from_str(&raw).expect("telemetry state should be valid json");
    assert_eq!(stored.install_id, install_id);
}

#[tokio::test]
async fn load_or_create_install_id_fails_closed_on_malformed_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = telemetry_state_path(temp.path());
    tokio::fs::write(&path, "{ not json")
        .await
        .expect("write malformed telemetry state");

    let install_id = load_or_create_install_id(temp.path()).await;
    assert!(
        install_id.is_none(),
        "malformed telemetry state should not be replaced with a new install id"
    );

    let raw = tokio::fs::read_to_string(&path)
        .await
        .expect("malformed telemetry state should be preserved for inspection");
    assert_eq!(raw, "{ not json");
}
