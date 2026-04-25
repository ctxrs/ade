use std::collections::HashMap;

use ctx_worker_protocol::RepoSpec;

use super::*;

fn sample_bootstrap_spec<'a>(
    repo: &'a RepoSpec,
    env: &'a HashMap<String, String>,
) -> BootstrapSpec<'a> {
    BootstrapSpec {
        worker_id: "worker-123",
        gateway_url: "https://gateway.example.test",
        gateway_token: Some("gateway-secret"),
        base_commit: "abc123",
        diff_debounce_ms: 1500,
        repo,
        provider_id: Some("provider-1"),
        env,
        shim_url: "https://gateway.example.test/bin/ctx-worker-shim",
        workdir: "/workspace",
        mount_path: "/mnt/ctx",
        mount_device_candidates: vec!["/dev/nvme1n1".to_string()],
    }
}

#[test]
fn fetch_bootstrap_script_only_renders_auth_and_ca_when_configured() {
    let script = render_fetch_bootstrap_script(
        "worker-123",
        "https://gateway.example.test",
        Some("gateway-secret"),
        Some("pem-base64"),
    );
    assert!(script.contains("CTX_WORKER_ID='worker-123'"));
    assert!(script.contains("CTX_GATEWAY_URL='https://gateway.example.test'"));
    assert!(script.contains("CTX_WORKER_GATEWAY_TOKEN='gateway-secret'"));
    assert!(script.contains("CTX_GATEWAY_CA_B64='pem-base64'"));
    assert!(script.contains("url=\"${url}?token=${CTX_WORKER_GATEWAY_TOKEN}\""));
    assert!(script.contains("args+=(--cacert \"$CTX_GATEWAY_CA_PATH\")"));

    let script_without_auth =
        render_fetch_bootstrap_script("worker-123", "https://gateway.example.test", None, None);
    assert!(!script_without_auth.contains("CTX_WORKER_GATEWAY_TOKEN="));
    assert!(!script_without_auth.contains("CTX_GATEWAY_CA_B64="));
}

#[test]
fn bootstrap_script_filters_unsafe_env_keys_and_sorts_safe_keys() {
    let repo = RepoSpec::Git {
        url: "https://example.test/repo.git".to_string(),
        reference: "main".to_string(),
    };
    let env = HashMap::from([
        ("ZETA".to_string(), "last".to_string()),
        ("ALPHA".to_string(), "first".to_string()),
        ("BAD-KEY".to_string(), "drop".to_string()),
        ("".to_string(), "drop-empty".to_string()),
    ]);
    let script = render_bootstrap_script(&sample_bootstrap_spec(&repo, &env));

    let alpha = script.find("ALPHA='first'").expect("ALPHA env missing");
    let zeta = script.find("ZETA='last'").expect("ZETA env missing");
    assert!(alpha < zeta, "safe env keys should render in sorted order");
    assert!(!script.contains("BAD-KEY='drop'"));
    assert!(!script.contains("drop-empty"));
}

#[test]
fn bootstrap_script_preserves_worker_shim_startup_contract() {
    let repo = RepoSpec::Archive {
        url: "https://example.test/repo.tar.gz".to_string(),
    };
    let env = HashMap::from([("CTX_CODEX_AUTH_B64".to_string(), "encoded".to_string())]);
    let script = render_bootstrap_script(&sample_bootstrap_spec(&repo, &env));

    assert!(script.contains("CTX_BASE_COMMIT='abc123'"));
    assert!(script.contains("CTX_DIFF_DEBOUNCE_MS='1500'"));
    assert!(script.contains("CTX_SHIM_URL='https://gateway.example.test/bin/ctx-worker-shim'"));
    assert!(script.contains("shim_url=$(with_gateway_token \"$CTX_SHIM_URL\")"));
    assert!(script.contains(
        "if [ -n \"${CTX_WORKER_GATEWAY_TOKEN:-}\" ]; then export CTX_WORKER_GATEWAY_TOKEN; fi"
    ));
    assert!(script.contains("nohup /usr/local/bin/ctx-worker-shim \\\n"));
    assert!(script.contains("    --base-commit \"$CTX_BASE_COMMIT\" \\\n"));
    assert!(script.contains("    --diff-debounce-ms \"$CTX_DIFF_DEBOUNCE_MS\" \\\n"));
    assert!(script.contains("CTX_REPO_TYPE='archive'"));
    assert!(script.contains("CTX_REPO_ARCHIVE_URL='https://example.test/repo.tar.gz'"));
}

#[test]
fn safe_env_key_check_rejects_empty_and_non_identifier_names() {
    assert!(is_safe_env_key("VALID_123"));
    assert!(!is_safe_env_key(""));
    assert!(!is_safe_env_key("BAD-KEY"));
    assert!(!is_safe_env_key("HAS SPACE"));
}
