use super::*;

#[test]
fn extract_auth_url_detects_urls_in_line() {
    let line = "Open this URL to continue: https://claude.ai/oauth/authorize?foo=bar";
    assert_eq!(
        extract_auth_url(line).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn extract_auth_url_detects_urls_in_browser_open_marker_line() {
    let line = "CTX_CLAUDE_AUTH_URL:https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(line).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_reconstructs_wrapped_url_lines() {
    let wrapped =
        "Open this URL: https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A\n64111%2Fauth%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(wrapped).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_stops_before_duplicate_full_url() {
    let url = "https://claude.ai/oauth/authorize?code=true&client_id=test&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&state=abc";
    let duplicated = format!("{url} {url}");
    assert_eq!(extract_auth_url(&duplicated).as_deref(), Some(url));
}

#[test]
fn normalize_claude_login_line_handles_osc_hyperlink_plus_visible_duplicate_url() {
    let url = "https://claude.ai/oauth/authorize?code=true&client_id=test&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&state=abc";
    let raw = format!("\u{1b}]8;;{url}\u{7}{url}\u{1b}]8;;\u{7}\r");
    let normalized = normalize_claude_login_line(&raw);
    assert_eq!(extract_auth_url(&normalized).as_deref(), Some(url));
}

#[test]
fn extract_auth_url_reconstructs_wrapped_scheme_prefix() {
    let wrapped =
        "Open this URL: ht\ntps://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc";
    assert_eq!(
        extract_auth_url(wrapped).as_deref(),
        Some(
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc"
        )
    );
}

#[test]
fn extract_auth_url_from_value_detects_embedded_url_in_message() {
    let payload = serde_json::json!({
        "message": "Visit this link to sign in: https://accounts.google.com/o/oauth2/auth?foo=bar"
    });
    assert_eq!(
        extract_auth_url_from_value(&payload).as_deref(),
        Some("https://accounts.google.com/o/oauth2/auth?foo=bar")
    );
}

#[test]
fn normalize_claude_login_line_strips_ansi_sequences() {
    let raw = "\u{1b}[90mOpen URL:\u{1b}[0m https://claude.ai/oauth/authorize?foo=bar\r";
    let normalized = normalize_claude_login_line(raw);
    assert_eq!(
        extract_auth_url(&normalized).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn normalize_claude_login_line_extracts_url_from_osc8_sequence() {
    let raw = "\u{1b}]8;;https://claude.ai/oauth/authorize?foo=bar\u{7}Sign in\u{1b}]8;;\u{7}\r";
    let normalized = normalize_claude_login_line(raw);
    assert_eq!(
        extract_auth_url(&normalized).as_deref(),
        Some("https://claude.ai/oauth/authorize?foo=bar")
    );
}

#[test]
fn auth_url_looks_complete_rejects_non_loopback_redirect_callback() {
    let url = "https://claude.ai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
    assert!(!auth_url_looks_complete(url));
}

#[test]
fn auth_url_looks_complete_requires_port_for_loopback_callback() {
    let url = "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%2Fcallback";
    assert!(!auth_url_looks_complete(url));
    let with_port =
        "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A5999%2Fcallback";
    assert!(auth_url_looks_complete(with_port));
}

#[tokio::test]
async fn read_trailing_claude_login_lines_waits_for_late_arrival() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        let _ = tx.send("sk-ant-oat01-late-token".to_string());
    });
    let lines = read_trailing_claude_login_lines(&mut rx, Duration::from_secs(2)).await;
    assert_eq!(lines, vec!["sk-ant-oat01-late-token".to_string()]);
}
