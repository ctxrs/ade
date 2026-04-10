use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;

use super::*;

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<String>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn immediate_sweep_config() -> ProviderSessionSweepConfig {
    ProviderSessionSweepConfig {
        idle_ttl: Duration::ZERO,
        max_idle_sessions: 0,
        interval: Duration::from_secs(60),
    }
}

fn write_session_status_runtime(
    workdir: &std::path::Path,
    script_name: &str,
    quiescent: bool,
) -> Result<std::path::PathBuf> {
    let script_path = workdir.join(script_name);
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nwhile IFS= read -r line; do\n  if printf '%s' \"$line\" | grep -q '\"type\":\"session.status\"'; then\n    session_id=$(printf '%s' \"$line\" | sed -n 's/.*\"session_id\":\"\\([^\"]*\\)\".*/\\1/p')\n    printf '{{\"v\":1,\"seq\":1,\"channel\":\"control\",\"type\":\"session.notice\",\"session_id\":\"%s\",\"code\":\"session_status\",\"severity\":\"info\",\"message\":\"status\",\"details\":{{\"quiescent\":{quiescent}}}}}\\n' \"$session_id\"\n  fi\ndone\n",
            quiescent = if quiescent { "true" } else { "false" },
        ),
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;
    Ok(script_path)
}

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

#[tokio::test]
async fn reap_idle_sessions_reaps_quiescent_live_session() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = write_session_status_runtime(&workdir, "quiescent-status.sh", true)?;
    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "quiescent-reap";
    let env = HashMap::new();

    let _session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(!adapter.has_live_session(session_key).await);
    assert!(adapter.pool.list_processes().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn provider_runtime_sessions_keep_status_probe_when_opened_metadata_omits_capability(
) -> Result<()> {
    let _env_lock = ENV_LOCK.lock().await;
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("provider-runtime-status-default.sh");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"type":"session.open"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":1,"channel":"control","type":"session.opened","session_id":"%s"}\n' "$session_id"
      ;;
    *'"type":"session.prompt"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      turn_id=$(printf '%s' "$line" | sed -n 's/.*"turn_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":2,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"success"}\n' "$session_id" "$turn_id"
      ;;
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":3,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","details":{"quiescent":true}}\n' "$session_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_provider_runtime(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let mut env = HashMap::new();
    env.insert(
        "CTX_SESSION_ID".to_string(),
        "runtime-status-default".to_string(),
    );

    let (event_sink, _event_rx) = mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "ping".to_string(),
                attachments: Vec::new(),
                context_blocks: Vec::new(),
                model_id: None,
            },
            workdir.clone(),
            env,
            event_sink,
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(5), handle.done)
        .await
        .context("prompt run should finish")??;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;
    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(!adapter.has_live_session("runtime-status-default").await);
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_reaps_unopened_session_without_status_probe() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture-unopened.sh");
    let log_path = workdir.join("capture-unopened.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\ndone\n",
    )?;
    fs::write(&log_path, "")?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "unopened-reap";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );

    let _session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(!adapter.has_live_session(session_key).await);
    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        !log_contents.contains(r#""type":"session.status""#),
        "unopened sessions must be reaped without a session.status probe"
    );
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_keeps_busy_session() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = write_session_status_runtime(&workdir, "busy-status.sh", false)?;
    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "busy-reap";
    let env = HashMap::new();

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.opened.store(true, Ordering::SeqCst);

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            skipped_busy: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(adapter.has_live_session(session_key).await);

    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_removes_dead_sessions_without_status_probe() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let adapter = Tier1CrpAdapter::from_raw(
        "fake-crp",
        "/bin/sh".to_string(),
        vec!["-c".into(), "cat >/dev/null".into()],
    );
    let session_key = "dead-reap";
    let env = HashMap::new();

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.process.shutdown("simulated runtime exit").await;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            dead_removed: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(!adapter.has_live_session(session_key).await);
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_skips_probe_for_runtimes_without_session_status() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture-idle.sh");
    let log_path = workdir.join("capture-idle.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\ndone\n",
    )?;
    fs::write(&log_path, "")?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw_with_session_status(
        "unsupported-crp",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
        false,
    );
    let session_key = "unsupported-probe";
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

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert!(adapter.has_live_session(session_key).await);
    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        !log_contents.contains(r#""type":"session.status""#),
        "unsupported runtimes must not be probed for session.status"
    );

    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn get_or_create_session_over_cap_does_not_probe_status_inline() -> Result<()> {
    let _env_lock = ENV_LOCK.lock().await;
    let _max_idle_guard = ScopedEnvVar::set("CTX_PROVIDER_WORKER_MAX_IDLE_SESSIONS", "1");

    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("capture-over-cap.sh");
    let log_path = workdir.join("capture-over-cap.log");

    fs::write(
        &script_path,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$LOG_FILE\"\ndone\n",
    )?;
    fs::write(&log_path, "")?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_provider_runtime(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );

    let first = adapter
        .pool
        .get_or_create_session("first-session", &workdir, &env)
        .await?;
    first.opened.store(true, Ordering::SeqCst);

    let second = tokio::time::timeout(
        Duration::from_secs(1),
        adapter
            .pool
            .get_or_create_session("second-session", &workdir, &env),
    )
    .await
    .context("timed out creating second session over idle cap")??;
    second.opened.store(true, Ordering::SeqCst);

    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        !log_contents.contains(r#""type":"session.status""#),
        "over-cap session creation must not synchronously probe session.status"
    );

    first.process.shutdown("test complete").await;
    second.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_never_kills_in_flight_model_update() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("set-model-busy.sh");
    let log_path = workdir.join("set-model-busy.log");
    let model_seen_path = workdir.join("model-seen");
    let allow_model_path = workdir.join("allow-model");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
  case "$line" in
    *'"type":"session.set_model"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      : > "$MODEL_SEEN_FILE"
      while [ ! -f "$ALLOW_MODEL_FILE" ]; do
        sleep 0.02
      done
      printf '{"v":1,"seq":1,"channel":"control","type":"session.notice","session_id":"%s","code":"session_model_updated","severity":"info","details":{"model_id":"amp-medium"}}\n' "$session_id"
      ;;
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":2,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","details":{"quiescent":true}}\n' "$session_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "busy-model-update";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert(
        "MODEL_SEEN_FILE".to_string(),
        model_seen_path.to_string_lossy().to_string(),
    );
    env.insert(
        "ALLOW_MODEL_FILE".to_string(),
        allow_model_path.to_string_lossy().to_string(),
    );

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.opened.store(true, Ordering::SeqCst);

    let adapter_for_task = adapter.clone();
    let set_model = tokio::spawn(async move {
        adapter_for_task
            .set_session_model(session_key.to_string(), "amp-medium".to_string())
            .await
    });

    let started = Instant::now();
    while !model_seen_path.exists() {
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for session.set_model");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;
    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert!(adapter.has_live_session(session_key).await);

    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        !log_contents.contains(r#""type":"session.status""#),
        "busy session.set_model must not be status-probed"
    );

    fs::write(&allow_model_path, "")?;
    set_model.await??;

    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn draining_model_update_session_shuts_down_after_completion() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("set-model-drain.sh");
    let log_path = workdir.join("set-model-drain.log");
    let model_seen_path = workdir.join("model-seen");
    let allow_model_path = workdir.join("allow-model");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
  case "$line" in
    *'"type":"session.set_model"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      : > "$MODEL_SEEN_FILE"
      while [ ! -f "$ALLOW_MODEL_FILE" ]; do
        sleep 0.02
      done
      printf '{"v":1,"seq":1,"channel":"control","type":"session.notice","session_id":"%s","code":"session_model_updated","severity":"info","details":{"model_id":"amp-medium"}}\n' "$session_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let session_key = "draining-model-update";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert(
        "MODEL_SEEN_FILE".to_string(),
        model_seen_path.to_string_lossy().to_string(),
    );
    env.insert(
        "ALLOW_MODEL_FILE".to_string(),
        allow_model_path.to_string_lossy().to_string(),
    );

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.opened.store(true, Ordering::SeqCst);

    let adapter_for_task = adapter.clone();
    let set_model = tokio::spawn(async move {
        adapter_for_task
            .set_session_model(session_key.to_string(), "amp-medium".to_string())
            .await
    });

    let started = Instant::now();
    while !model_seen_path.exists() {
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for session.set_model");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    adapter.pool.restart_drain("test drain").await;
    assert_eq!(adapter.pool.list_processes().await.len(), 1);

    fs::write(&allow_model_path, "")?;
    set_model.await??;

    let shutdown_started = Instant::now();
    while adapter.has_live_session(session_key).await {
        if shutdown_started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for draining model-update session shutdown");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(adapter.pool.list_processes().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn canceled_prompt_clears_opening_and_reaps_unopened_session() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("opening-cancel.sh");
    let log_path = workdir.join("opening-cancel.log");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
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
    let session_key = "opening-cancel";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );

    let (event_tx, _event_rx) = mpsc::channel(8);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "work".to_string(),
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
            if contents.contains(r#""type":"session.open""#)
                && contents.contains(r#""type":"session.prompt""#)
            {
                break;
            }
        }
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for startup commands");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel_tx.send(()).expect("cancel signal should send");
    let _ = tokio::time::timeout(Duration::from_secs(5), prompt_task)
        .await
        .context("timed out waiting for canceled prompt")??;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(!adapter.has_live_session(session_key).await);
    Ok(())
}

#[tokio::test]
async fn completed_prompt_reaps_oldest_idle_session_in_background() -> Result<()> {
    let _env_lock = ENV_LOCK.lock().await;
    let _max_idle_guard = ScopedEnvVar::set("CTX_PROVIDER_WORKER_MAX_IDLE_SESSIONS", "1");

    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("background-reap.sh");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"type":"session.open"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":1,"channel":"control","type":"session.opened","session_id":"%s","supports_session_status":true}\n' "$session_id"
      ;;
    *'"type":"session.prompt"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      turn_id=$(printf '%s' "$line" | sed -n 's/.*"turn_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":2,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"success"}\n' "$session_id" "$turn_id"
      ;;
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":3,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","details":{"quiescent":true}}\n' "$session_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_provider_runtime(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );

    for session_key in ["first-idle", "second-idle"] {
        let mut env = HashMap::new();
        env.insert("CTX_SESSION_ID".to_string(), session_key.to_string());
        let (event_sink, _event_rx) = mpsc::channel(8);
        let handle = adapter
            .run(
                TurnInput {
                    content: "ping".to_string(),
                    attachments: Vec::new(),
                    context_blocks: Vec::new(),
                    model_id: None,
                },
                workdir.clone(),
                env,
                event_sink,
            )
            .await?;
        tokio::time::timeout(Duration::from_secs(5), handle.done)
            .await
            .context("prompt run should finish")??;
    }

    let started = Instant::now();
    loop {
        if adapter.pool.list_processes().await.len() == 1 {
            break;
        }
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for background idle reap");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(!adapter.has_live_session("first-idle").await);
    assert!(adapter.has_live_session("second-idle").await);

    adapter
        .restart("test complete", ProviderRestartMode::Immediate)
        .await?;
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_scans_past_busy_oldest_session_to_enforce_cap() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("status-cap-scan.sh");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      if [ "$session_id" = "oldest-busy" ]; then
        quiescent=false
      else
        quiescent=true
      fi
      printf '{"v":1,"seq":1,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","details":{"quiescent":%s}}\n' "$session_id" "$quiescent"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let env = HashMap::new();

    let oldest = adapter
        .pool
        .get_or_create_session("oldest-busy", &workdir, &env)
        .await?;
    oldest.opened.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(20)).await;

    let newest = adapter
        .pool
        .get_or_create_session("newest-quiescent", &workdir, &env)
        .await?;
    newest.opened.store(true, Ordering::SeqCst);

    let stats = adapter
        .pool
        .reap_idle_sessions(ProviderSessionSweepConfig {
            idle_ttl: Duration::from_secs(3600),
            max_idle_sessions: 1,
            interval: Duration::from_secs(60),
        })
        .await;

    assert_eq!(
        stats,
        ProviderSessionSweepStats {
            reaped: 1,
            skipped_busy: 1,
            ..ProviderSessionSweepStats::default()
        }
    );
    assert!(adapter.has_live_session("oldest-busy").await);
    assert!(!adapter.has_live_session("newest-quiescent").await);

    oldest.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn background_reap_runs_followup_sweep_for_sessions_that_finish_mid_sweep() -> Result<()> {
    let _env_lock = ENV_LOCK.lock().await;
    let _max_idle_guard = ScopedEnvVar::set("CTX_PROVIDER_WORKER_MAX_IDLE_SESSIONS", "0");

    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("status-followup.sh");
    let log_path = workdir.join("status-followup.log");
    let first_status_seen_path = workdir.join("first-status-seen");
    let allow_first_status_path = workdir.join("allow-first-status");

    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
  case "$line" in
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      if [ "$session_id" = "first-idle" ]; then
        : > "$FIRST_STATUS_SEEN_FILE"
        while [ ! -f "$ALLOW_FIRST_STATUS_FILE" ]; do
          sleep 0.02
        done
      fi
      printf '{"v":1,"seq":1,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","details":{"quiescent":true}}\n' "$session_id"
      ;;
  esac
done
"#,
    )?;
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)?;

    let adapter = Tier1CrpAdapter::from_raw(
        "codex",
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().to_string()],
    );
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert(
        "FIRST_STATUS_SEEN_FILE".to_string(),
        first_status_seen_path.to_string_lossy().to_string(),
    );
    env.insert(
        "ALLOW_FIRST_STATUS_FILE".to_string(),
        allow_first_status_path.to_string_lossy().to_string(),
    );

    let first = adapter
        .pool
        .get_or_create_session("first-idle", &workdir, &env)
        .await?;
    first.opened.store(true, Ordering::SeqCst);

    adapter.pool.trigger_background_reap();

    let started = Instant::now();
    while !first_status_seen_path.exists() {
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for first session.status probe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let second = adapter
        .pool
        .get_or_create_session("second-idle", &workdir, &env)
        .await?;
    second.opened.store(true, Ordering::SeqCst);

    adapter.pool.trigger_background_reap();
    fs::write(&allow_first_status_path, "")?;

    let drain_started = Instant::now();
    while !adapter.pool.list_processes().await.is_empty() {
        if drain_started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for queued background reaps");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(!adapter.has_live_session("first-idle").await);
    assert!(!adapter.has_live_session("second-idle").await);
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_never_kills_active_prompt() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("active-prompt.sh");
    let log_path = workdir.join("active-prompt.log");

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
    let session_key = "active-reap";
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

    let (event_tx, _event_rx) = mpsc::channel(8);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "work".to_string(),
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
            anyhow::bail!("timed out waiting for active session.prompt");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;
    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert!(adapter.has_live_session(session_key).await);

    let log_contents = fs::read_to_string(&log_path)?;
    assert!(
        !log_contents.contains(r#""type":"session.status""#),
        "active prompt session should not be status-probed"
    );

    cancel_tx.send(()).expect("cancel signal should send");
    prompt_task.await??;
    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_skips_session_that_becomes_active_during_status_probe() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("status-race.sh");
    let log_path = workdir.join("status-race.log");
    let status_seen_path = workdir.join("status-seen");
    let allow_status_path = workdir.join("allow-status");

    fs::write(
        &script_path,
        r#"#!/bin/sh
turn_id=""
session_id=""
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG_FILE"
  case "$line" in
    *'"type":"session.status"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      : > "$STATUS_SEEN_FILE"
      while [ ! -f "$ALLOW_STATUS_FILE" ]; do
        sleep 0.02
      done
      printf '{"v":1,"seq":1,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","message":"status","details":{"quiescent":true}}\n' "$session_id"
      ;;
    *'"type":"session.prompt"'*)
      turn_id=$(printf '%s' "$line" | sed -n 's/.*"turn_id":"\([^"]*\)".*/\1/p')
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      ;;
    *'"type":"session.cancel"'*)
      printf '{"v":1,"seq":2,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"interrupted"}\n' "$session_id" "$turn_id"
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
    let session_key = "status-race-reap";
    let mut env = HashMap::new();
    env.insert(
        "LOG_FILE".to_string(),
        log_path.to_string_lossy().to_string(),
    );
    env.insert(
        "STATUS_SEEN_FILE".to_string(),
        status_seen_path.to_string_lossy().to_string(),
    );
    env.insert(
        "ALLOW_STATUS_FILE".to_string(),
        allow_status_path.to_string_lossy().to_string(),
    );

    let session = adapter
        .pool
        .get_or_create_session(session_key, &workdir, &env)
        .await?;
    session.opened.store(true, Ordering::SeqCst);
    let initial_last_used = session.last_used();

    let pool = Arc::clone(&adapter.pool);
    let reap_task =
        tokio::spawn(async move { pool.reap_idle_sessions(immediate_sweep_config()).await });

    let started = Instant::now();
    while !status_seen_path.exists() {
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for session.status probe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (event_tx, mut event_rx) = mpsc::channel(8);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "work".to_string(),
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
        if session.last_used() != initial_last_used {
            break;
        }
        if started.elapsed() > Duration::from_secs(5) {
            anyhow::bail!("timed out waiting for overlapping prompt reuse");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    fs::write(&allow_status_path, "")?;

    let stats = reap_task.await?;
    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert!(adapter.has_live_session(session_key).await);

    cancel_tx.send(()).expect("cancel signal should send");
    prompt_task.await??;
    while let Ok(event) = event_rx.try_recv() {
        assert_ne!(
            event.payload_json.get("code"),
            Some(&json!("session_status")),
            "sweep-only session_status notices must not leak into prompt streams"
        );
    }

    let session = adapter.pool.require_open_session(session_key).await?;
    session.process.shutdown("test complete").await;
    Ok(())
}

#[tokio::test]
async fn reap_idle_sessions_preserves_draining_sessions_until_prompt_completion() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("draining-prompt.sh");
    let log_path = workdir.join("draining-prompt.log");

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
    let session_key = "draining-reap";
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

    let (event_tx, _event_rx) = mpsc::channel(8);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "work".to_string(),
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
            anyhow::bail!("timed out waiting for draining session.prompt");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    adapter.pool.restart_drain("test drain").await;

    let stats = adapter
        .pool
        .reap_idle_sessions(immediate_sweep_config())
        .await;
    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert_eq!(adapter.pool.list_processes().await.len(), 1);

    cancel_tx.send(()).expect("cancel signal should send");
    prompt_task.await??;

    assert!(adapter.pool.list_processes().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn completed_prompt_refreshes_idle_timestamp_before_reap() -> Result<()> {
    let tempdir = tempfile::tempdir()?;
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("touch-after-prompt.sh");

    fs::write(
        &script_path,
        r#"#!/bin/sh
turn_id=""
session_id=""
while IFS= read -r line; do
  case "$line" in
    *'"type":"session.open"'*)
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      printf '{"v":1,"seq":1,"channel":"control","type":"session.opened","session_id":"%s","provider_session_id":"provider-touch-after-prompt"}\n' "$session_id"
      ;;
    *'"type":"session.prompt"'*)
      turn_id=$(printf '%s' "$line" | sed -n 's/.*"turn_id":"\([^"]*\)".*/\1/p')
      session_id=$(printf '%s' "$line" | sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')
      sleep 0.2
      printf '{"v":1,"seq":2,"channel":"control","type":"turn.completed","session_id":"%s","turn_id":"%s","status":"success"}\n' "$session_id" "$turn_id"
      ;;
    *'"type":"session.status"'*)
      printf '{"v":1,"seq":3,"channel":"control","type":"session.notice","session_id":"%s","code":"session_status","severity":"info","message":"status","details":{"quiescent":true}}\n' "$session_id"
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
    let session_key = "touch-after-prompt";
    let env = HashMap::new();

    let (event_tx, _event_rx) = mpsc::channel(8);
    let (_cancel_tx, cancel_rx) = oneshot::channel();
    let request = CrpPromptRequest {
        session_key: session_key.to_string(),
        input: TurnInput {
            content: "work".to_string(),
            attachments: Vec::new(),
            context_blocks: Vec::new(),
            model_id: None,
        },
        workdir: workdir.clone(),
        env: env.clone(),
        event_sink: event_tx,
        cancel_rx,
    };

    adapter.pool.prompt(request).await?;

    let stats = adapter
        .pool
        .reap_idle_sessions(ProviderSessionSweepConfig {
            idle_ttl: Duration::from_secs(1),
            max_idle_sessions: usize::MAX,
            interval: Duration::from_secs(60),
        })
        .await;
    assert_eq!(stats, ProviderSessionSweepStats::default());
    assert!(adapter.has_live_session(session_key).await);

    let session = adapter.pool.require_open_session(session_key).await?;
    session.process.shutdown("test complete").await;
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
