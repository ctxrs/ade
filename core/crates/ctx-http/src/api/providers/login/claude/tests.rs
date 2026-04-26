use super::*;
use std::process::Command;

#[test]
fn preferred_claude_auth_url_uses_browser_open_marker_over_scraped_url() {
    let expected = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";
    let corrupted = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=https:/platform.claude.com/oauth/code/callback&scope=user:inference&code_challenge=abc&code_challenge_method=S256&state=bad-statePastecodehereifprompted%3E";
    let transcript = format!(
        "{CLAUDE_BROWSER_OPEN_MARKER}{expected}\nBrowser didn't open? Use the URL below to sign in\n{corrupted}\nPaste code here if prompted >"
    );

    assert_eq!(
        extract_preferred_claude_auth_url(&transcript)
            .map(|(value, _)| value)
            .as_deref(),
        Some(expected)
    );
}

#[test]
fn browser_open_marker_replaces_longer_incomplete_transcript_url() {
    let current = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=https:/platform.claude.com/oauth/code/callback&scope=user:inference&code_challenge=abc&code_challenge_method=S256&state=bad-state";
    let candidate = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";

    assert!(should_replace_observed_claude_auth_url(
        Some(current),
        candidate,
        ClaudeAuthUrlSource::BrowserOpenMarker,
    ));
}

#[test]
fn provider_browser_auth_tier_skips_os_browser_launch() {
    assert!(claude_login_should_skip_browser_open(Some(
        CLAUDE_BROWSER_AUTH_TIER
    )));
    assert!(claude_login_should_skip_browser_open(Some(
        " Provider-Browser-Auth "
    )));
    assert!(!claude_login_should_skip_browser_open(Some(
        "provider-api-auth"
    )));
    assert!(!claude_login_should_skip_browser_open(None));
}

#[test]
fn reads_captured_browser_open_url_from_side_channel_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let capture_path = temp_dir.path().join("auth-url");
    let expected = "https://claude.ai/oauth/authorize?code=true&client_id=cid&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A58215%2Fcallback&scope=user%3Ainference&code_challenge=abc&code_challenge_method=S256&state=good-state";
    std::fs::write(&capture_path, format!("{expected}\n")).expect("write capture file");

    assert_eq!(
        read_claude_browser_open_capture_url(&capture_path).as_deref(),
        Some(expected)
    );
}

fn write_executable_script(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("chmod script");
    }
}

#[test]
fn browser_open_shim_capture_only_writes_auth_url() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let script_path = temp_dir.path().join("open-browser");
    write_executable_script(&script_path, claude_browser_open_shim_script(true));
    let capture_path = temp_dir.path().join("auth-url");
    let auth_url =
        "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A5999%2Fcallback&state=test";

    let output = Command::new("/bin/sh")
        .arg(&script_path)
        .arg(auth_url)
        .env("CTX_CLAUDE_AUTH_URL_CAPTURE_PATH", &capture_path)
        .output()
        .expect("run capture-only shim");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&capture_path).expect("read capture path"),
        format!("{auth_url}\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{CLAUDE_BROWSER_OPEN_MARKER}{auth_url}\n")
    );
}

#[test]
fn browser_open_shim_invokes_open_and_captures_auth_url() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let script_path = temp_dir.path().join("open-browser");
    write_executable_script(&script_path, claude_browser_open_shim_script(false));
    let capture_path = temp_dir.path().join("auth-url");
    let open_log_path = temp_dir.path().join("open.log");
    let open_path = temp_dir.path().join("open");
    write_executable_script(
        &open_path,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"{}\"\nexit 0\n",
            open_log_path.display()
        ),
    );
    let auth_url =
        "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6001%2Fcallback&state=test";
    let path_env = format!("{}:/usr/bin:/bin", temp_dir.path().display());

    let output = Command::new("/bin/sh")
        .arg(&script_path)
        .arg(auth_url)
        .env("CTX_CLAUDE_AUTH_URL_CAPTURE_PATH", &capture_path)
        .env("PATH", path_env)
        .output()
        .expect("run browser-open shim");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&capture_path).expect("read capture path"),
        format!("{auth_url}\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{CLAUDE_BROWSER_OPEN_MARKER}{auth_url}\n")
    );
    assert_eq!(
        std::fs::read_to_string(&open_log_path).expect("read open log"),
        format!("{auth_url}\n")
    );
}
