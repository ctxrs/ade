use super::{TelemetryDelivery, TelemetryEvent, TelemetryOriginRuntime, TelemetryPlane};

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
