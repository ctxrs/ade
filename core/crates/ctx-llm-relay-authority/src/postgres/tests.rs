#[test]
fn spend_limit_query_casts_neon_uuid_and_integer_types() {
    let source = include_str!("spend_limits.rs");
    assert!(source.contains("ctx_user_id::text as ctx_user_id"));
    assert!(source.contains("hard_limit_cents::bigint as hard_limit_cents"));
    assert!(source.contains("reservations.billing_subject_id = $1::uuid"));
}

#[test]
fn live_relay_reservation_caps_finalization_at_reserved_cents() {
    let postgres_source = include_str!("../postgres.rs");
    let reservations_source = include_str!("reservations.rs");
    assert!(postgres_source.contains(".bind(checked_i64(live_config.reservation_cents)?)"));
    assert!(!postgres_source.contains(".bind(checked_i64(request.grant.max_estimated_cents)?)"));
    assert!(reservations_source.contains("max_estimated_cents == reservation_cents"));
}

#[test]
fn relay_config_reads_do_not_require_lock_privileges() {
    let pricing_source = include_str!("pricing_policy.rs").to_ascii_lowercase();
    let spend_limit_source = include_str!("spend_limits.rs").to_ascii_lowercase();
    assert!(!pricing_source.contains("for share"));
    assert!(!spend_limit_source.contains("for update"));
    assert!(spend_limit_source.contains("pg_advisory_xact_lock"));
    assert!(
        spend_limit_source.contains("pg_advisory_xact_lock(\n          hashtextextended($1::text")
    );
    assert!(!spend_limit_source.contains("hashtextextended($2::text"));
    assert!(spend_limit_source.contains("acquire_spend_limit_advisory_lock_tx"));
}
