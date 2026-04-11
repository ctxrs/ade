use super::status::{build_session_status_details, ThreadStatusSnapshot};
use super::translate::{canonical_context_window_from_thread_usage, translate_notification};
use super::*;
use pretty_assertions::assert_eq;
use std::fs;
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
fn canonical_context_window_matches_expected_shape() {
    let usage = crate::app_server::ThreadTokenUsage {
        total: crate::app_server::TokenUsageBreakdown {
            total_tokens: 4200,
            input_tokens: 3000,
            cached_input_tokens: 0,
            output_tokens: 900,
            reasoning_output_tokens: 300,
        },
        last: crate::app_server::TokenUsageBreakdown {
            total_tokens: 4200,
            input_tokens: 3000,
            cached_input_tokens: 0,
            output_tokens: 900,
            reasoning_output_tokens: 300,
        },
        model_context_window: Some(128000),
    };
    let metrics = canonical_context_window_from_thread_usage(&usage).expect("metrics");
    assert_eq!(metrics["context_tokens_estimate"], json!(4200));
    assert_eq!(metrics["context_window_tokens"], json!(128000));
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
