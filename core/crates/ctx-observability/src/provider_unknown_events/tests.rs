use anyhow::Result;
use serde_json::json;

use super::*;

fn test_context() -> ProviderUnknownEventContext {
    ProviderUnknownEventContext {
        provider_id: "codex".to_string(),
        execution_environment: Some("host".to_string()),
        session_root_kind: Some("worktree".to_string()),
        operation: "turn".to_string(),
    }
}

fn test_observation() -> ProviderUnknownEventObservation {
    ProviderUnknownEventObservation {
        protocol: "crp",
        event_type: "tool.progress".to_string(),
        parse_error: "unknown variant `tool.progress`".to_string(),
        raw: json!({
            "type": "tool.progress",
            "session_id": "session-raw",
            "turn_id": "turn-raw",
            "message": "Bearer secret-token",
            "token": "secret-token",
            "nested": {
                "apiKey": "secret-api-key"
            }
        }),
        raw_truncated: false,
        crp_channel: Some("data".to_string()),
        crp_seq: 42,
        timeline_notice_emitted: false,
    }
}

#[test]
fn telemetry_event_is_metadata_only() {
    let event = build_telemetry_event(&test_context(), &test_observation(), 0.01);
    assert_eq!(event.event_name, EVENT_NAME);
    assert_eq!(event.properties.get("provider_id"), Some(&json!("codex")));
    assert_eq!(
        event.properties.get("event_type"),
        Some(&json!("tool.progress"))
    );
    assert_eq!(
        event.properties.get("parse_error_kind"),
        Some(&json!("unknown_variant"))
    );
    assert_eq!(event.properties.get("diagnostic_only"), Some(&json!(true)));
    assert!(!event.properties.contains_key("raw"));
    assert!(!event.properties.contains_key("session_id"));
    assert!(!event.properties.contains_key("turn_id"));
    assert!(!event.properties.contains_key("parse_error"));
    assert!(!event.properties.contains_key("parse_error_hash"));
}

#[test]
fn telemetry_event_omits_unsafe_event_type_values() {
    let mut observation = test_observation();
    observation.event_type = "user prompt: fix secret-token".to_string();

    let event = build_telemetry_event(&test_context(), &observation, 0.01);

    assert_eq!(event.properties.get("event_type_safe"), Some(&json!(false)));
    assert_eq!(event.properties.get("event_type_length"), Some(&json!(29)));
    assert!(!event.properties.contains_key("event_type"));
    assert!(!event.properties.contains_key("event_type_hash"));
}

#[test]
fn telemetry_event_treats_path_like_event_types_as_unsafe() {
    let mut observation = test_observation();
    observation.event_type = "command/exec/outputDelta".to_string();

    let event = build_telemetry_event(&test_context(), &observation, 0.01);

    assert_eq!(event.properties.get("event_type_safe"), Some(&json!(false)));
    assert!(!event.properties.contains_key("event_type"));
    assert!(!event.properties.contains_key("event_type_hash"));
}

#[test]
fn diagnostic_line_redacts_secret_fields_and_strings() -> Result<()> {
    let line = diagnostic_log_line(&test_context(), &test_observation())?;
    assert!(line.contains("provider_unknown_event_observed"));
    assert!(!line.contains("secret-token"));
    assert!(!line.contains("secret-api-key"));
    assert!(line.contains("[REDACTED]"));
    Ok(())
}

#[tokio::test]
async fn local_cleanup_enforces_byte_cap() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let dir = logs::logs_dir(tempdir.path()).join(LOG_DIR_NAME);
    tokio::fs::create_dir_all(&dir).await?;
    let old_path = dir.join(format!("{LOG_PREFIX}2026-01-01{LOG_SUFFIX}"));
    let new_path = dir.join(format!("{LOG_PREFIX}2026-01-02{LOG_SUFFIX}"));
    tokio::fs::write(&old_path, "a".repeat(64)).await?;
    tokio::fs::write(&new_path, "b".repeat(64)).await?;

    cleanup_local_diagnostic_logs(
        &dir,
        LocalLogConfig {
            max_bytes: 80,
            retention_days: 0,
        },
    )
    .await?;

    assert!(!old_path.exists());
    assert!(new_path.exists());
    Ok(())
}
