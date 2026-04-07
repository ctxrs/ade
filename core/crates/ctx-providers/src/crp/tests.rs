use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
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

#[test]
fn unknown_crp_event_maps_to_timeline_notice() {
    let mut tool_output_cache: HashMap<String, String> = HashMap::new();
    let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

    let mapped = map_crp_event(
        CrpEvent::Unknown {
            event_type: "tool.progress".to_string(),
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            parse_error: "unknown variant `tool.progress`".to_string(),
            raw: json!({
                "type": "tool.progress",
                "session_id": "session-1",
                "turn_id": "turn-1",
                "message": "Scanning files",
                "percent": 50
            }),
        },
        protocol::CrpChannel::Data,
        11,
        &mut tool_output_cache,
        &mut tool_input_cache,
    );

    assert_eq!(mapped.events.len(), 1);
    assert!(matches!(
        mapped.events[0].event_type,
        SessionEventType::Notice
    ));
    let payload = &mapped.events[0].payload_json;
    assert_eq!(payload.get("kind"), Some(&json!("crp_unknown_event")));
    assert_eq!(payload.get("original_type"), Some(&json!("tool.progress")));
    assert_eq!(payload.get("display_in_timeline"), Some(&json!(true)));
    assert_eq!(payload.get("crp_seq"), Some(&json!(11)));
    assert_eq!(payload.get("crp_channel"), Some(&json!("data")));
    assert_eq!(payload.pointer("/raw/percent"), Some(&json!(50)));
}

#[test]
fn unknown_crp_tool_event_preserves_tool_name_and_preview() {
    let mut tool_output_cache: HashMap<String, String> = HashMap::new();
    let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

    let mapped = map_crp_event(
        CrpEvent::Unknown {
            event_type: "tool.progress".to_string(),
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            parse_error: "unknown variant `tool.progress`".to_string(),
            raw: json!({
                "type": "tool.progress",
                "session_id": "session-1",
                "turn_id": "turn-1",
                "tool_name": "Bash",
                "command": ["find", ".ctx/ctx-pack/agent-basics", "-type", "f"]
            }),
        },
        protocol::CrpChannel::Data,
        12,
        &mut tool_output_cache,
        &mut tool_input_cache,
    );

    let payload = &mapped.events[0].payload_json;
    assert_eq!(payload.get("tool_name"), Some(&json!("Bash")));
    assert_eq!(
        payload.get("tool_preview"),
        Some(&json!("find .ctx/ctx-pack/agent-basics -type f"))
    );
    assert_eq!(
        payload.get("message"),
        Some(&json!(
            "Unknown tool event: Bash · find .ctx/ctx-pack/agent-basics -type f"
        ))
    );
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

#[tokio::test]
async fn prompt_drains_terminal_interrupted_event_after_cancel() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("fake-crp.sh");
    let log_path = workdir.join("stdin.log");

    fs::write(
        &script_path,
        r#"#!/bin/sh
turn_id=""
session_id=""
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
  case "$line" in
    *'"type":"session.prompt"'*)
      turn_id=$(printf '%s' "$line" | sed -n 's/.*"turn_id":"\([^"]*\)".*/\1/p')
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      ;;
    *'"type":"session.cancel"'*)
      sleep 0.1
      printf '{"v":1,"seq":1,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"interrupted"}\n' "$session_id" "$turn_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "cancel-drain";
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

    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "investigate".to_string(),
            attachments: Vec::new(),
            context_blocks: Vec::new(),
            model_id: None,
        },
        workdir: workdir.clone(),
        env: env.clone(),
        event_sink: event_tx,
        cancel_rx,
    };

    let pool = Arc::clone(&adapter.pool);
    let prompt_task = tokio::spawn(async move { pool.prompt(request).await });

    let started = Instant::now();
    loop {
        if let Ok(contents) = fs::read_to_string(&log_path) {
            if contents.contains(r#""type":"session.prompt""#) {
                break;
            }
        }
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for session.prompt");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel_tx.send(()).expect("cancel signal should send");

    let mut saw_turn_interrupted = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), event_rx.recv()).await {
            Ok(Some(event)) => {
                if matches!(event.event_type, SessionEventType::TurnInterrupted) {
                    saw_turn_interrupted = true;
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }

    prompt_task.await??;
    assert!(
        saw_turn_interrupted,
        "expected interrupted terminal event after cancel"
    );

    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn prompt_fails_fast_on_fatal_startup_stderr_and_shuts_down_runtime() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("fatal_startup.sh");

    fs::write(
        &script_path,
        "#!/bin/sh\nprintf 'time=\"2026-04-02T22:10:57Z\" level=fatal msg=\"failed to create temp dir\"\\n' >&2\nsleep 30\n",
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "session-fatal-startup";
    let mut env = HashMap::new();
    env.insert("CTX_SESSION_ID".to_string(), session_key.to_string());

    let (event_sink, mut event_rx) = tokio::sync::mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "user".to_string(),
                attachments: vec![],
                context_blocks: vec![],
                model_id: None,
            },
            workdir,
            env,
            event_sink,
        )
        .await?;

    tokio::time::timeout(Duration::from_secs(5), handle.done)
        .await
        .expect("fatal startup run should finish promptly")?;

    let error_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match event_rx.recv().await {
                Some(event) if matches!(event.event_type, SessionEventType::Error) => {
                    return Some(event)
                }
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await
    .expect("error event wait should complete")
    .expect("expected error event");

    assert!(
        error_event
            .payload_json
            .get("message")
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("level=fatal")),
        "expected fatal stderr to surface through the error event"
    );
    assert!(
        !adapter.has_live_session(session_key).await,
        "fatal startup stderr should shut down the unusable runtime session"
    );

    Ok(())
}

#[tokio::test]
async fn opencode_flattens_prompt_items_into_single_prompt_field() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture_prompt.sh");
    let log_path = workdir.join("stdin.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.open\"'; then\n    printf '{\"v\":1,\"seq\":1,\"channel\":\"control\",\"type\":\"session.opened\",\"session_id\":\"session-prompt\"}\\n'\n  fi\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.prompt\"'; then\n    turn_id=$(printf '%s' \"$line\" | sed -n 's/.*\"turn_id\":\"\\([^\"]*\\)\".*/\\1/p')\n    printf '{\"v\":1,\"seq\":2,\"channel\":\"control\",\"type\":\"turn.completed\",\"session_id\":\"session-prompt\",\"turn_id\":\"%s\",\"status\":\"success\"}\\n' \"$turn_id\"\n  fi\ndone\n",
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "opencode",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert("CTX_SESSION_ID".to_string(), "session-prompt".to_string());

    let (event_sink, mut event_rx) = tokio::sync::mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "user".to_string(),
                attachments: vec![],
                context_blocks: vec![json!({"type":"text","text":"system"})],
                model_id: Some("openai/gpt-4.1-mini".to_string()),
            },
            workdir.clone(),
            env,
            event_sink,
        )
        .await?;

    tokio::time::timeout(Duration::from_secs(5), handle.done)
        .await
        .expect("prompt run should finish")?;
    while event_rx.recv().await.is_some() {}

    let contents = fs::read_to_string(&log_path)?;
    let prompt_line = contents
        .lines()
        .find(|line| line.contains("\"type\":\"session.prompt\""))
        .expect("captured prompt line");
    let payload: serde_json::Value = serde_json::from_str(prompt_line)?;
    assert_eq!(payload.get("items"), None);
    assert_eq!(payload.get("prompt"), Some(&json!("system\n\nuser")));

    Ok(())
}

#[tokio::test]
async fn prompt_model_override_can_be_disabled_via_env() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture_prompt_model.sh");
    let log_path = workdir.join("stdin.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.open\"'; then\n    printf '{\"v\":1,\"seq\":1,\"channel\":\"control\",\"type\":\"session.opened\",\"session_id\":\"session-no-model\"}\\n'\n  fi\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.prompt\"'; then\n    turn_id=$(printf '%s' \"$line\" | sed -n 's/.*\"turn_id\":\"\\([^\"]*\\)\".*/\\1/p')\n    printf '{\"v\":1,\"seq\":2,\"channel\":\"control\",\"type\":\"turn.completed\",\"session_id\":\"session-no-model\",\"turn_id\":\"%s\",\"status\":\"success\"}\\n' \"$turn_id\"\n  fi\ndone\n",
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "kimi",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert("CTX_SESSION_ID".to_string(), "session-no-model".to_string());
    env.insert(
        "CTX_CRP_DISABLE_MODEL_OVERRIDE".to_string(),
        "1".to_string(),
    );

    let (event_sink, mut event_rx) = tokio::sync::mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "user".to_string(),
                attachments: vec![],
                context_blocks: vec![],
                model_id: Some("openai/gpt-4.1-mini".to_string()),
            },
            workdir.clone(),
            env,
            event_sink,
        )
        .await?;

    tokio::time::timeout(Duration::from_secs(5), handle.done)
        .await
        .expect("prompt run should finish")?;
    while event_rx.recv().await.is_some() {}

    let contents = fs::read_to_string(&log_path)?;
    let prompt_line = contents
        .lines()
        .find(|line| line.contains("\"type\":\"session.prompt\""))
        .expect("captured prompt line");
    let payload: serde_json::Value = serde_json::from_str(prompt_line)?;
    assert_eq!(payload.get("model"), None);

    Ok(())
}

#[tokio::test]
async fn shutdown_cached_session_is_not_live_and_gets_replaced() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec!["-c".into(), "cat >/dev/null".into()],
    );
    let env = HashMap::new();
    let session_key = "session-restart-after-shutdown";

    let first = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    assert!(adapter.has_live_session(session_key).await);

    first.process.shutdown("simulated runtime exit").await;

    assert!(!adapter.has_live_session(session_key).await);

    let second = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    assert!(
        !Arc::ptr_eq(&first, &second),
        "expected a dead cached session to be replaced"
    );
    assert_eq!(session_shutdown_reason(&second), None);

    second.process.shutdown("test complete").await;
    Ok(())
}

#[test]
fn auth_required_stderr_notice_payload_is_redacted() {
    let payload =
        auth_required_notice_payload_from_stderr("https://auth.example.test/start?token=secret");

    assert_eq!(payload.get("kind"), Some(&json!("auth_required")));
    assert_eq!(payload.get("code"), Some(&json!("auth_required")));
    assert_eq!(
        payload.get("message"),
        Some(&json!("Authentication required."))
    );
    assert_eq!(payload.get("source"), Some(&json!("crp_stderr")));
    assert_eq!(payload.get("auth_url"), None);
}
