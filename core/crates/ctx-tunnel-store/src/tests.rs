use chrono::TimeZone;

use super::*;
use crate::retention::RETENTION_DELETE_INACTIVE_TUNNELS_SQL;

#[test]
fn rejects_non_postgres_database_urls() {
    assert!(matches!(
        validate_postgres_database_url("sqlite:///tmp/control-plane.sqlite"),
        Err(TunnelStoreError::InvalidDatabaseUrl)
    ));
    assert!(validate_postgres_database_url("postgres://user:pw@example/db").is_ok());
    assert!(validate_postgres_database_url("postgresql://user:pw@example/db").is_ok());
}

#[test]
fn validates_relay_registration_urls_and_capacity() {
    let valid = RelayRegistration {
        relay_id: "relay-1".to_string(),
        region: "us".to_string(),
        public_base_url: "https://relay-1.tunnel.ctx.rs".to_string(),
        internal_base_url: "http://127.0.0.1:8787".to_string(),
        max_active_tunnels: 100,
    };
    assert!(valid.validate().is_ok());

    let mut invalid = valid.clone();
    invalid.public_base_url = "not-url".to_string();
    assert!(matches!(
        invalid.validate(),
        Err(TunnelStoreError::InvalidUrl {
            field: "public_base_url",
            ..
        })
    ));

    let mut invalid = valid;
    invalid.max_active_tunnels = 0;
    assert!(matches!(
        invalid.validate(),
        Err(TunnelStoreError::InvalidInput(_))
    ));
}

#[test]
fn register_relay_preserves_ops_disabled_and_draining_statuses() {
    assert!(REGISTER_RELAY_SQL
        .contains("public.mobile_tunnel_relay_node.status in ('disabled', 'draining')"));
    assert!(REGISTER_RELAY_SQL.contains("then public.mobile_tunnel_relay_node.status"));
}

#[test]
fn validates_create_tunnel_request() {
    let valid = CreateTunnelRequest {
        tunnel_id: "tun_1".to_string(),
        user_id: "user_1".to_string(),
        billing_subject_id: None,
        relay_id: "relay-1".to_string(),
        public_base_url: "https://tunnel.ctx.rs/t/tun_1".to_string(),
    };
    assert!(valid.validate().is_ok());

    let mut invalid = valid;
    invalid.user_id = " ".to_string();
    assert!(matches!(
        invalid.validate(),
        Err(TunnelStoreError::InvalidInput(_))
    ));
}

#[test]
fn validates_retention_cleanup_policy() {
    let now = Utc.with_ymd_and_hms(2026, 4, 30, 12, 0, 0).unwrap();
    let policy = RetentionCleanupPolicy::new(90, 180, now).unwrap();
    assert_eq!(
        policy.event_cutoff().unwrap(),
        Utc.with_ymd_and_hms(2026, 1, 30, 12, 0, 0).unwrap()
    );
    assert_eq!(
        policy.inactive_tunnel_cutoff().unwrap(),
        Utc.with_ymd_and_hms(2025, 11, 1, 12, 0, 0).unwrap()
    );

    assert!(matches!(
        RetentionCleanupPolicy::new(0, 90, now),
        Err(TunnelStoreError::InvalidInput(_))
    ));
    assert!(matches!(
        RetentionCleanupPolicy::new(90, -1, now),
        Err(TunnelStoreError::InvalidInput(_))
    ));
}

#[test]
fn inactive_tunnel_cleanup_preserves_recent_audit_events() {
    assert!(RETENTION_DELETE_INACTIVE_TUNNELS_SQL.contains("status = 'revoked'"));
    assert!(RETENTION_DELETE_INACTIVE_TUNNELS_SQL.contains("disabled_at is not null"));
    assert!(RETENTION_DELETE_INACTIVE_TUNNELS_SQL.contains("not exists"));
    assert!(RETENTION_DELETE_INACTIVE_TUNNELS_SQL.contains("e.created_at >= $1"));
}

#[test]
fn retention_cleanup_evaluates_inactive_tunnels_before_event_deletion() {
    let source = include_str!("retention.rs");
    let inactive_delete = source
        .find("sqlx::query(RETENTION_DELETE_INACTIVE_TUNNELS_SQL)")
        .expect("inactive tunnel delete query must be present");
    let event_delete = source
        .find("sqlx::query(RETENTION_DELETE_EVENTS_SQL)")
        .expect("event delete query must be present");
    assert!(inactive_delete < event_delete);
}
