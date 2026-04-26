use super::commands::handle_parsed_command;
use super::prompt_items::translate_prompt_items_for_app_server;
use super::session::open_session;
use super::status::{build_session_status_details, ThreadStatusSnapshot};
use super::translate::translate_notification;
use super::*;
use crate::app_server::AppServerClient;
use crate::protocol::{CrpChannel, CrpCommand, CrpEvent};
use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FixtureStep {
    BindTurn {
        app_turn_id: String,
        crp_turn_id: String,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
struct SnapshotEvent {
    channel: String,
    event: Value,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq)]
struct SnapshotOutput {
    events: Vec<SnapshotEvent>,
    aliases: HashMap<String, String>,
    latest_token_usage: Option<crate::app_server::ThreadTokenUsage>,
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn testdata_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(file)
}

fn replay_fixture(file: &str) -> SnapshotOutput {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let runtime_guard = runtime.enter();
    let mut state = AppServerSessionState {
        tracker: TurnTracker::new("fixture-session".to_string()),
        client: AppServerClient::test_stub(),
        thread_id: "thr_fixture".to_string(),
        default_cwd: PathBuf::from("/tmp"),
        default_model: "gpt-5.4".to_string(),
        default_effort: Some("medium".to_string()),
        turn_config_overrides: None,
        opened_commands: Vec::new(),
        opened_slash_commands: Vec::new(),
        turn_aliases: TurnAliasState::new(),
        resumed_from_provider_session: false,
        command_execution_seen: false,
    };
    let input = fs::read_to_string(testdata_path(file)).expect("fixture should exist");
    let mut events = Vec::new();
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let step: FixtureStep = serde_json::from_str(line).expect("fixture line should parse");
        let translated = match step {
            FixtureStep::BindTurn {
                app_turn_id,
                crp_turn_id,
            } => {
                state
                    .turn_aliases
                    .bind_turn_alias(app_turn_id, Some(crp_turn_id));
                Vec::new()
            }
            FixtureStep::Notification { method, params } => {
                translate_notification(&mut state, &method, params)
                    .expect("notification should translate")
            }
        };
        for (channel, event) in translated {
            events.push(SnapshotEvent {
                channel: match channel {
                    CrpChannel::Control => "control".to_string(),
                    CrpChannel::Data => "data".to_string(),
                },
                event: serde_json::to_value(event).expect("event should serialize"),
            });
        }
    }

    let output = SnapshotOutput {
        events,
        aliases: state.turn_aliases.app_to_crp.clone(),
        latest_token_usage: state.turn_aliases.latest_token_usage.clone(),
    };
    drop(state);
    drop(runtime_guard);
    drop(runtime);
    output
}

fn assert_snapshot(input: &str, expected: &str) {
    let actual = replay_fixture(input);
    let expected: SnapshotOutput = serde_json::from_str(
        &fs::read_to_string(testdata_path(expected)).expect("expected snapshot should exist"),
    )
    .expect("expected snapshot should parse");
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn open_session_fails_closed_on_resume_error_and_scrubs_ambient_session_env() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("fake-codex.sh");
    let log_path = workdir.join("app-server.log");
    fs::write(
        &script_path,
        format!(
            r#"#!/bin/sh
printf 'CODEX_THREAD_ID=%s\n' "${{CODEX_THREAD_ID-}}" >> "{}"
printf 'CTX_PROVIDER_SESSION_REF=%s\n' "${{CTX_PROVIDER_SESSION_REF-}}" >> "{}"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  if printf '%s' "$line" | grep -q '"method":"initialize"'; then
    printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"0.1","capabilities":{{}},"serverInfo":{{"name":"fake-codex","version":"test"}}}}}}\n' "$id"
  elif printf '%s' "$line" | grep -q '"method":"thread/resume"'; then
    printf 'resume\n' >> "{}"
    printf '{{"jsonrpc":"2.0","id":%s,"error":{{"message":"resume failed"}}}}\n' "$id"
  elif printf '%s' "$line" | grep -q '"method":"thread/start"'; then
    printf 'start\n' >> "{}"
    printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"new-thread"}},"model":"gpt-5.4","cwd":"{}","approvalPolicy":{{}},"sandbox":{{}},"reasoningEffort":"medium"}}}}\n' "$id"
  fi
done
"#,
            log_path.display(),
            log_path.display(),
            log_path.display(),
            log_path.display(),
            workdir.display()
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&script_path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("script perms");

    let _codex_bin = EnvGuard::set("CTX_CODEX_BIN_PATH", &script_path.to_string_lossy());
    let _stale_thread = EnvGuard::set("CODEX_THREAD_ID", "stale-thread");
    let _stale_provider = EnvGuard::set("CTX_PROVIDER_SESSION_REF", "stale-provider");

    let err = match open_session(
        crate::protocol::CrpSessionConfig {
            cwd: Some(workdir.clone()),
            ..Default::default()
        },
        Some("expected-provider-ref".to_string()),
        &RuntimeOptions::default(),
    )
    .await
    {
        Ok(_) => panic!("resume failure must not succeed"),
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("failed to resume Codex provider session `expected-provider-ref`"),
        "{err:#}"
    );

    let log = fs::read_to_string(&log_path).expect("read fake codex log");
    assert!(log.contains("resume"));
    assert!(
        !log.contains("start"),
        "resume failure must not fall back to thread/start: {log}"
    );
    assert!(
        log.contains("CODEX_THREAD_ID=") && !log.contains("CODEX_THREAD_ID=stale-thread"),
        "app-server must not inherit stale CODEX_THREAD_ID: {log}"
    );
    assert!(
        log.contains("CTX_PROVIDER_SESSION_REF=")
            && !log.contains("CTX_PROVIDER_SESSION_REF=stale-provider"),
        "app-server must not inherit stale CTX_PROVIDER_SESSION_REF: {log}"
    );
}

#[test]
fn current_model_id_preserves_reasoning_effort_suffix() {
    assert_eq!(current_model_id("gpt-5.4", Some("xhigh")), "gpt-5.4/xhigh");
    assert_eq!(current_model_id("gpt-5.4", None), "gpt-5.4");
    assert_eq!(current_model_id("gpt-5.4", Some("  ")), "gpt-5.4");
}

#[test]
fn basic_message_snapshot_matches() {
    assert_snapshot(
        "app_server_basic_message.input.jsonl",
        "app_server_basic_message.expected.json",
    );
}

#[test]
fn reasoning_and_tools_snapshot_matches() {
    assert_snapshot(
        "app_server_reasoning_and_tools.input.jsonl",
        "app_server_reasoning_and_tools.expected.json",
    );
}

#[test]
fn error_and_usage_snapshot_matches() {
    assert_snapshot(
        "app_server_error_and_usage.input.jsonl",
        "app_server_error_and_usage.expected.json",
    );
}

#[test]
fn file_change_object_kind_snapshot_matches() {
    assert_snapshot(
        "app_server_file_change_object_kind.input.jsonl",
        "app_server_file_change_object_kind.expected.json",
    );
}

#[test]
fn canonical_context_window_uses_last_usage_for_live_meter() {
    let usage = crate::app_server::ThreadTokenUsage {
        total: crate::app_server::TokenUsageBreakdown {
            total_tokens: 4200,
            input_tokens: 3000,
            cached_input_tokens: 0,
            output_tokens: 900,
            reasoning_output_tokens: 300,
        },
        last: crate::app_server::TokenUsageBreakdown {
            total_tokens: 300,
            input_tokens: 200,
            cached_input_tokens: 0,
            output_tokens: 70,
            reasoning_output_tokens: 30,
        },
        model_context_window: Some(128000),
    };
    let metrics = canonical_context_window_from_thread_usage(&usage).expect("metrics");
    assert_eq!(metrics["context_tokens_estimate"], json!(300));
    assert_eq!(metrics["context_window_tokens"], json!(128000));
    assert_eq!(metrics["remaining_tokens_estimate"], json!(127700));
    assert_eq!(metrics["total_input_tokens"], json!(200));
    assert_eq!(metrics["total_output_tokens"], json!(100));
}

#[test]
fn session_status_details_report_quiescent_when_loaded_threads_are_idle() {
    let details = build_session_status_details(
        "thr_root",
        None,
        false,
        false,
        vec![
            ThreadStatusSnapshot {
                thread_id: "thr_root".to_string(),
                status: crate::app_server::ThreadStatus::Idle,
            },
            ThreadStatusSnapshot {
                thread_id: "thr_child".to_string(),
                status: crate::app_server::ThreadStatus::Idle,
            },
        ],
    );

    assert_eq!(details["quiescent"], json!(true));
    assert_eq!(details["active_thread_ids"], json!([]));
    assert_eq!(details["busy_reasons"], json!([]));
}

#[test]
fn session_status_details_report_busy_for_active_loaded_thread_or_turn() {
    let details = build_session_status_details(
        "thr_root",
        Some("turn-1".to_string()),
        false,
        false,
        vec![
            ThreadStatusSnapshot {
                thread_id: "thr_root".to_string(),
                status: crate::app_server::ThreadStatus::Active {
                    active_flags: Vec::new(),
                },
            },
            ThreadStatusSnapshot {
                thread_id: "thr_child".to_string(),
                status: crate::app_server::ThreadStatus::Idle,
            },
        ],
    );

    assert_eq!(details["quiescent"], json!(false));
    assert_eq!(details["active_turn_id"], json!("turn-1"));
    assert_eq!(details["active_thread_ids"], json!(["thr_root"]));
    assert_eq!(
        details["busy_reasons"],
        json!(["active_turn", "loaded_thread_active"])
    );
}

#[test]
fn session_status_details_report_historical_command_execution_without_blocking_quiescence() {
    let details = build_session_status_details(
        "thr_root",
        None,
        false,
        true,
        vec![ThreadStatusSnapshot {
            thread_id: "thr_root".to_string(),
            status: crate::app_server::ThreadStatus::Idle,
        }],
    );

    assert_eq!(details["quiescent"], json!(true));
    assert_eq!(details["command_execution_observed"], json!(true));
    assert_eq!(details["busy_reasons"], json!([]));
}

#[test]
fn session_status_details_report_resumed_provider_session_without_blocking_quiescence() {
    let details = build_session_status_details(
        "thr_root",
        None,
        true,
        false,
        vec![ThreadStatusSnapshot {
            thread_id: "thr_root".to_string(),
            status: crate::app_server::ThreadStatus::Idle,
        }],
    );

    assert_eq!(details["quiescent"], json!(true));
    assert_eq!(details["resumed_from_provider_session"], json!(true));
    assert_eq!(details["busy_reasons"], json!([]));
}

#[tokio::test]
async fn session_authenticate_emits_explicit_unsupported_notice() {
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let (data_tx, _data_rx) = mpsc::channel(1);
    let router = CrpEventRouter::new(control_tx, data_tx);
    let mut session = Some(AppServerSessionState {
        tracker: TurnTracker::new("fixture-session".to_string()),
        client: AppServerClient::test_stub(),
        thread_id: "thr_fixture".to_string(),
        default_cwd: PathBuf::from("/tmp"),
        default_model: "gpt-5.4".to_string(),
        default_effort: Some("medium".to_string()),
        turn_config_overrides: None,
        opened_commands: Vec::new(),
        opened_slash_commands: Vec::new(),
        turn_aliases: TurnAliasState::new(),
        resumed_from_provider_session: false,
        command_execution_seen: false,
    });

    handle_command(
        RuntimeCommand::Parsed(Box::new(CrpCommand::SessionAuthenticate {
            session_id: Some("fixture-session".to_string()),
            method_id: Some("oauth".to_string()),
        })),
        &mut session,
        &router,
        &RuntimeOptions::default(),
    )
    .await
    .expect("session.authenticate handling should not fail");

    match control_rx.try_recv().expect("expected control event") {
        CrpEvent::SessionNotice {
            session_id,
            code,
            severity,
            message,
            details,
            ..
        } => {
            assert_eq!(session_id, "fixture-session");
            assert_eq!(code, "auth_error");
            assert_eq!(severity.as_deref(), Some("error"));
            assert_eq!(
                message.as_deref(),
                Some("session.authenticate is not supported by this runtime")
            );
            assert_eq!(
                details,
                Some(json!({
                    "provider": "codex-crp",
                    "reason": "unsupported_command",
                }))
            );
        }
        other => panic!("expected session notice, got {other:?}"),
    }
}

#[tokio::test]
async fn open_session_defers_mcp_overrides_until_turn_start() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let workdir = tempdir.path().to_path_buf();
    let script_path = workdir.join("fake-codex.py");
    let log_path = workdir.join("app-server.log");
    fs::write(
        &script_path,
        format!(
            r#"#!/usr/bin/env python3
import json
import sys
from pathlib import Path

log_path = Path("{}")
for raw in sys.stdin:
    line = raw.rstrip("\n")
    log_path.write_text(log_path.read_text() + line + "\n" if log_path.exists() else line + "\n")
    msg = json.loads(line)
    method = msg.get("method")
    ident = msg.get("id")
    if method == "initialize":
        resp = {{"jsonrpc":"2.0","id":ident,"result":{{"protocolVersion":"0.1","capabilities":{{}},"serverInfo":{{"name":"fake-codex","version":"test"}}}}}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()
    elif method == "thread/start":
        resp = {{"jsonrpc":"2.0","id":ident,"result":{{"thread":{{"id":"new-thread"}},"model":"gpt-5.4","cwd":"{}","approvalPolicy":{{}},"sandbox":{{}},"reasoningEffort":"medium"}}}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()
    elif method == "turn/start":
        resp = {{"jsonrpc":"2.0","id":ident,"result":{{"turn":{{"id":"turn-app-1"}}}}}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()
"#,
            log_path.display(),
            workdir.display()
        ),
    )
    .expect("write fake codex");
    let mut permissions = fs::metadata(&script_path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("script perms");

    let _codex_bin = EnvGuard::set("CTX_CODEX_BIN_PATH", &script_path.to_string_lossy());
    let (control_tx, _control_rx) = mpsc::unbounded_channel();
    let (data_tx, _data_rx) = mpsc::channel(1);
    let router = CrpEventRouter::new(control_tx, data_tx);
    let mut session = None;

    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "ctx".to_string(),
        crate::protocol::CrpMcpServerConfig {
            command: Some("ctx-mcp".to_string()),
            args: Some(vec!["--stdio".to_string()]),
            ..Default::default()
        },
    );

    handle_parsed_command(
        CrpCommand::SessionOpen {
            session_id: Some("fixture-session".to_string()),
            provider_session_id: None,
            config: Some(crate::protocol::CrpSessionConfig {
                cwd: Some(workdir.clone()),
                mcp_servers: Some(mcp_servers),
                ..Default::default()
            }),
        },
        &mut session,
        &router,
        &RuntimeOptions::default(),
    )
    .await
    .expect("session open should succeed");

    handle_parsed_command(
        CrpCommand::SessionPrompt {
            session_id: Some("fixture-session".to_string()),
            turn_id: Some("turn-crp-1".to_string()),
            prompt: Some("hello".to_string()),
            items: None,
            model: None,
            reasoning_effort: None,
            cwd: None,
        },
        &mut session,
        &router,
        &RuntimeOptions::default(),
    )
    .await
    .expect("session prompt should succeed");

    let log = fs::read_to_string(&log_path).expect("read fake codex log");
    let thread_start = log
        .lines()
        .find(|line| line.contains(r#""method":"thread/start""#))
        .expect("thread/start request logged");
    let turn_start = log
        .lines()
        .find(|line| line.contains(r#""method":"turn/start""#))
        .expect("turn/start request logged");

    assert!(
        !thread_start.contains("mcp_servers.ctx"),
        "thread/start must not bootstrap ctx MCP: {thread_start}"
    );
    assert!(
        turn_start.contains("mcp_servers.ctx"),
        "turn/start must include deferred ctx MCP config: {turn_start}"
    );

    if let Some(state) = session.as_mut() {
        state.client.shutdown().await;
    }
}

#[tokio::test]
async fn session_status_without_active_session_emits_failure_notice() {
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let (data_tx, _data_rx) = mpsc::channel(1);
    let router = CrpEventRouter::new(control_tx, data_tx);
    let mut session = None;

    handle_command(
        RuntimeCommand::Parsed(Box::new(CrpCommand::SessionStatus {
            session_id: Some("fixture-session".to_string()),
        })),
        &mut session,
        &router,
        &RuntimeOptions::default(),
    )
    .await
    .expect("session.status handling should not fail");

    match control_rx.try_recv().expect("expected control event") {
        CrpEvent::SessionNotice {
            session_id,
            code,
            message,
            ..
        } => {
            assert_eq!(session_id, "fixture-session");
            assert_eq!(code, "session_status_failed");
            assert_eq!(
                message.as_deref(),
                Some("session status query failed: no active session")
            );
        }
        other => panic!("expected session notice, got {other:?}"),
    }
}

#[tokio::test]
async fn translate_prompt_items_for_app_server_converts_blob_refs_to_local_images() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let _guard = EnvGuard::set("CTX_DATA_ROOT", &data_root.path().to_string_lossy());
    let expected_path = data_root.path().join("blobs").join("blob-123");

    let translated = translate_prompt_items_for_app_server(vec![json!({
        "type": "image_ref",
        "blob_id": "blob-123",
        "mime_type": "image/png",
        "name": "cat.png",
    })])
    .await
    .expect("image_ref should translate");

    assert_eq!(
        translated,
        vec![json!({
            "type": "localImage",
            "path": expected_path.to_string_lossy().to_string(),
        })]
    );
}

#[tokio::test]
async fn translate_prompt_items_for_app_server_converts_inline_images_to_data_urls() {
    let translated = translate_prompt_items_for_app_server(vec![json!({
        "type": "image",
        "mime_type": "image/png",
        "data": "AQID",
        "name": "cat.png",
    })])
    .await
    .expect("inline image should translate");

    assert_eq!(
        translated,
        vec![json!({
            "type": "image",
            "url": "data:image/png;base64,AQID",
        })]
    );
}

#[tokio::test]
async fn translate_prompt_items_for_app_server_renames_local_image_items() {
    let translated = translate_prompt_items_for_app_server(vec![json!({
        "type": "local_image",
        "path": "/tmp/cat.png",
    })])
    .await
    .expect("local_image should translate");

    assert_eq!(
        translated,
        vec![json!({
            "type": "localImage",
            "path": "/tmp/cat.png",
        })]
    );
}
