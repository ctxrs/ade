use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use tokio::time::Duration;

use super::*;

#[tokio::test]
async fn set_session_model_writes_crp_command_for_live_session() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture.sh");
    let log_path = workdir.join("stdin.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.set_model\"'; then\n    printf '{\"v\":1,\"seq\":1,\"channel\":\"control\",\"type\":\"session.notice\",\"session_id\":\"session-set-model\",\"code\":\"session_model_updated\",\"severity\":\"info\",\"message\":\"session model updated to amp-medium\",\"details\":{\"model_id\":\"amp-medium\"}}\\n'\n  fi\ndone\n",
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "session-set-model";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.opened.store(true, Ordering::SeqCst);

    adapter
        .set_session_model(session_key.to_string(), "amp-medium".to_string())
        .await?;

    let started = Instant::now();
    let contents = loop {
        if let Ok(contents) = fs::read_to_string(&log_path) {
            if !contents.trim().is_empty() {
                break contents;
            }
        }
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for CRP command capture");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let first_line = contents
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("captured command line");
    let payload: serde_json::Value = serde_json::from_str(first_line)?;
    assert_eq!(payload.get("v"), Some(&json!(1)));
    assert_eq!(payload.get("type"), Some(&json!("session.set_model")));
    assert_eq!(payload.get("session_id"), Some(&json!(session_key)));
    assert_eq!(payload.get("model_id"), Some(&json!("amp-medium")));

    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn set_session_model_rejects_missing_live_session() {
    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec!["-c".into(), "cat >/dev/null".into()],
    );
    let err = adapter
        .set_session_model("missing".to_string(), "amp-medium".to_string())
        .await
        .expect_err("missing session should fail");
    assert!(err
        .to_string()
        .contains("provider session missing is not live"));
}
