pub(in crate::api::providers) fn auth_notice_code(payload: &serde_json::Value) -> &str {
    payload
        .get("code")
        .or_else(|| payload.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

pub(in crate::api::providers) fn is_auth_success_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
    )
}

pub(in crate::api::providers) fn is_auth_failure_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_failed" | "auth_error" | "provider_session_ref_claim_failed"
    )
}
