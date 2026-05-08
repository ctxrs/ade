use super::*;

#[test]
fn expected_callback_extracts_loopback_redirect() {
    let auth_url = "https://chat.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6543%2Fauth%2Fcallback";
    let expected = expected_callback_from_auth_url(auth_url);
    assert_eq!(
        expected.as_deref(),
        Some("http://localhost:6543/auth/callback")
    );
}

#[test]
fn callback_validation_rejects_non_loopback_host() {
    let err = validate_callback_url(
        "http://example.com:1234/auth/callback?code=abc",
        Some("http://localhost:1234/auth/callback"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("loopback"));
}

#[test]
fn callback_validation_accepts_expected_port_path_and_query() {
    validate_callback_url(
        "http://127.0.0.1:4321/auth/callback?code=abc&state=def",
        Some("http://localhost:4321/auth/callback"),
    )
    .expect("callback URL should validate");
}

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

#[tokio::test]
async fn resolve_claude_login_runtime_requires_managed_or_configured_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();

    let err = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect_err("missing managed/configured claude login command should fail");
    assert!(err
        .to_string()
        .contains("runtime_command_missing: provider=claude-cli"));
    assert!(err
        .to_string()
        .contains("host PATH lookup is not supported"));
}

#[tokio::test]
async fn resolve_claude_login_runtime_uses_configured_runtime_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["--shim".to_string()],
            dependencies: vec!["dep-node".to_string()],
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from config");
    assert!(resolved.command_abs_path.contains("claude-cli-mock.sh"));
    assert_eq!(resolved.args, vec!["--shim".to_string()]);
    assert_eq!(resolved.dependencies, vec!["dep-node".to_string()]);
    assert_eq!(
        resolved.source,
        installer::ProviderRuntimeCommandSource::UserOverride
    );
}

#[tokio::test]
async fn resolve_claude_login_runtime_prefers_configured_runtime_command_when_login_command_missing(
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let data_root = temp.path().to_path_buf();
    let runtime_path = data_root.join("claude-cli-runtime-mock.sh");
    std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
    let runtime_path_str = runtime_path.to_string_lossy().to_string();
    let mut cfg = installer::load_agent_server_config(&data_root)
        .await
        .expect("load config for runtime resolution test");
    cfg.providers.insert(
        "claude-cli".to_string(),
        installer::AgentServerCommand {
            command: runtime_path_str,
            args: vec!["cli.js".to_string()],
            dependencies: Vec::new(),
            managed: None,
        },
    );
    installer::save_agent_server_config(&data_root, &cfg)
        .await
        .expect("save config for runtime resolution test");

    let resolved = resolve_claude_login_runtime_from_config(&data_root)
        .await
        .expect("resolve runtime from provider command");
    assert!(resolved
        .command_abs_path
        .contains("claude-cli-runtime-mock.sh"));
    assert_eq!(resolved.args, vec!["cli.js".to_string()]);
}
