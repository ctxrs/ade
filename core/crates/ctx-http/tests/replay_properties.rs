#![cfg(feature = "property_tests")]

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage, WebSocketStream};

use ctx_core::ids::{MessageId, SessionId, TurnId, WorkspaceId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEventType, SessionHeadSnapshot,
    SessionTurn, SessionTurnStatus, SessionTurnTool, Task, Workspace, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveSnapshotStreamMessage,
};
use ctx_daemon::daemon::DaemonState;

mod common;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayExpectation {
    after_seq: i64,
    expected_seqs: Vec<i64>,
    expect_gap: bool,
    #[serde(default)]
    expected_seed_last_event_seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveProjectionExpected {
    head_last_event_seq: i64,
    persisted_event_types: Vec<String>,
    stable_event_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveProjectionEquivalenceFixture {
    user_content: String,
    assistant_content: String,
    tool_call_id: String,
    tool_title: String,
    tool_kind: String,
    tool_input: Value,
    tool_output: String,
    stream_assistant_chunk: String,
    replay_expectations: Vec<ReplayExpectation>,
    expected: ActiveProjectionExpected,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionGapExpected {
    head_last_event_seq: i64,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionGapSeedRehydrateFixture {
    user_content: String,
    assistant_content: String,
    after_seq: i64,
    expected: SessionGapExpected,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionFixtureFile {
    active_projection_equivalence: ActiveProjectionEquivalenceFixture,
    session_gap_seed_rehydrate: SessionGapSeedRehydrateFixture,
}

fn projection_fixtures() -> &'static ProjectionFixtureFile {
    static FIXTURES: OnceLock<ProjectionFixtureFile> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/web/src/testdata/projectionEquivalence.fixtures.json"
        )))
        .expect("projection equivalence fixture JSON should parse")
    })
}

struct ProjectionHarness {
    _repo: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
    state: Arc<DaemonState>,
    server: common::TestServer,
    workspace: Workspace,
    task: Task,
    session: Session,
    turn_id: TurnId,
    user_message_id: MessageId,
    assistant_message_id: MessageId,
}

struct ReplayFixture {
    harness: ProjectionHarness,
    seqs: Vec<i64>,
}

type Ws = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Default)]
struct ReplayObservation {
    seqs: Vec<i64>,
    gap_reason: Option<String>,
    seed_last_event_seq: Option<i64>,
}

async fn setup_projection_harness() -> ProjectionHarness {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;

    let workspace: Workspace = server
        .client
        .post(format!("{}/api/workspaces", server.base_url))
        .json(&json!({
            "root_path": repo.path(),
            "name": "projection-fixture",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let task: Task = server
        .client
        .post(format!(
            "{}/api/workspaces/{}/tasks",
            server.base_url, workspace.id.0
        ))
        .json(&json!({ "title": "projection-fixture-task" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let session = common::load_primary_session_http(&server.client, &server.base_url, &task).await;

    state.remember_session_meta(&session).await;

    ProjectionHarness {
        _repo: repo,
        _data_dir: data_dir,
        state,
        server,
        workspace,
        task,
        session,
        turn_id: TurnId::new(),
        user_message_id: MessageId::new(),
        assistant_message_id: MessageId::new(),
    }
}

async fn seed_active_projection_case(
    harness: &ProjectionHarness,
    fixture: &ActiveProjectionEquivalenceFixture,
) -> Vec<i64> {
    let store = harness
        .state
        .store_for_session(harness.session.id)
        .await
        .unwrap();
    let started_at = Utc::now();
    let tool_at = started_at + ChronoDuration::seconds(1);
    let assistant_at = started_at + ChronoDuration::seconds(2);
    let updated_at = started_at + ChronoDuration::seconds(3);

    let turn = SessionTurn {
        turn_id: harness.turn_id,
        session_id: harness.session.id,
        run_id: None,
        user_message_id: Some(harness.user_message_id),
        status: SessionTurnStatus::Running,
        start_seq: Some(1),
        end_seq: None,
        started_at,
        updated_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        failure: None,
        tool_total: 1,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 1,
        tool_failed: 0,
    };
    store.insert_session_turn(turn).await.unwrap();

    store
        .insert_message(Message {
            id: harness.user_message_id,
            session_id: harness.session.id,
            task_id: harness.task.id,
            run_id: None,
            turn_id: Some(harness.turn_id),
            turn_sequence: Some(1),
            order_seq: Some(1),
            role: MessageRole::User,
            content: fixture.user_content.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(started_at),
            created_at: started_at,
        })
        .await
        .unwrap();
    store
        .insert_message(Message {
            id: harness.assistant_message_id,
            session_id: harness.session.id,
            task_id: harness.task.id,
            run_id: None,
            turn_id: Some(harness.turn_id),
            turn_sequence: Some(3),
            order_seq: Some(3),
            role: MessageRole::Assistant,
            content: fixture.assistant_content.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(assistant_at),
            created_at: assistant_at,
        })
        .await
        .unwrap();

    let mut seqs = Vec::new();
    let durable_specs = [
        (
            SessionEventType::UserMessage,
            json!({
                "message_id": harness.user_message_id.0,
                "content": fixture.user_content.clone(),
                "attachments": [],
                "order_seq": 1,
            }),
        ),
        (
            SessionEventType::ToolCall,
            json!({
                "tool_call_id": fixture.tool_call_id.clone(),
                "title": fixture.tool_title.clone(),
                "kind": fixture.tool_kind.clone(),
                "input": fixture.tool_input.clone(),
                "order_seq": 2,
            }),
        ),
        (
            SessionEventType::ToolResult,
            json!({
                "tool_call_id": fixture.tool_call_id.clone(),
                "title": fixture.tool_title.clone(),
                "kind": fixture.tool_kind.clone(),
                "outputText": fixture.tool_output.clone(),
                "order_seq": 2,
            }),
        ),
        (
            SessionEventType::AssistantComplete,
            json!({
                "message_id": harness.assistant_message_id.0,
                "content": fixture.assistant_content.clone(),
                "full_content": fixture.assistant_content.clone(),
                "order_seq": 3,
            }),
        ),
    ];

    for (event_type, payload) in durable_specs {
        let event = store
            .append_session_event(
                harness.session.id,
                None,
                Some(harness.turn_id),
                event_type,
                payload,
            )
            .await
            .unwrap();
        seqs.push(event.seq);
        harness.state.publish_event(event).await;
    }
    store
        .update_session_turn_status(
            harness.session.id,
            harness.turn_id,
            SessionTurnStatus::Completed,
            seqs.last().copied(),
            None,
            updated_at,
        )
        .await
        .unwrap();

    let partial = store
        .append_session_event(
            harness.session.id,
            None,
            Some(harness.turn_id),
            SessionEventType::AssistantChunk,
            json!({
                "content_fragment": fixture.stream_assistant_chunk.clone(),
                "order_seq": 3,
            }),
        )
        .await
        .unwrap();
    harness.state.publish_event(partial).await;

    store
        .upsert_session_turn_tool(SessionTurnTool {
            session_id: harness.session.id,
            tool_call_id: fixture.tool_call_id.clone(),
            turn_id: harness.turn_id,
            tool_kind: Some(fixture.tool_kind.clone()),
            provider_tool_name: Some(fixture.tool_kind.clone()),
            title: Some(fixture.tool_title.clone()),
            subtitle: Some("fixture subtitle".to_string()),
            status: Some("completed".to_string()),
            input_json: Some(fixture.tool_input.clone()),
            output_text: Some(fixture.tool_output.clone()),
            order_seq: 3,
            first_event_seq: Some(2),
            input_truncated: None,
            input_original_bytes: None,
            output_truncated: None,
            output_original_bytes: None,
            created_at: tool_at,
            updated_at,
        })
        .await
        .unwrap();

    harness
        .state
        .ensure_workspace_active_snapshot_hydrated(harness.workspace.id)
        .await
        .unwrap();
    harness
        .state
        .refresh_session_head_cache(harness.session.id)
        .await;
    seqs
}

async fn seed_gap_case(harness: &ProjectionHarness, fixture: &SessionGapSeedRehydrateFixture) {
    let store = harness
        .state
        .store_for_session(harness.session.id)
        .await
        .unwrap();
    let started_at = Utc::now();
    let assistant_at = started_at + ChronoDuration::seconds(2);
    let updated_at = started_at + ChronoDuration::seconds(3);

    store
        .insert_session_turn(SessionTurn {
            turn_id: harness.turn_id,
            session_id: harness.session.id,
            run_id: None,
            user_message_id: Some(harness.user_message_id),
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at,
            updated_at,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();

    store
        .insert_message(Message {
            id: harness.user_message_id,
            session_id: harness.session.id,
            task_id: harness.task.id,
            run_id: None,
            turn_id: Some(harness.turn_id),
            turn_sequence: Some(1),
            order_seq: Some(1),
            role: MessageRole::User,
            content: fixture.user_content.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(started_at),
            created_at: started_at,
        })
        .await
        .unwrap();
    store
        .insert_message(Message {
            id: harness.assistant_message_id,
            session_id: harness.session.id,
            task_id: harness.task.id,
            run_id: None,
            turn_id: Some(harness.turn_id),
            turn_sequence: Some(3),
            order_seq: Some(3),
            role: MessageRole::Assistant,
            content: fixture.assistant_content.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(assistant_at),
            created_at: assistant_at,
        })
        .await
        .unwrap();

    let user_message = store
        .append_session_event(
            harness.session.id,
            None,
            Some(harness.turn_id),
            SessionEventType::UserMessage,
            json!({
                "message_id": harness.user_message_id.0,
                "content": fixture.user_content.clone(),
                "attachments": [],
                "order_seq": 1,
            }),
        )
        .await
        .unwrap();
    harness.state.publish_event(user_message).await;

    for idx in 0..2003 {
        let event = store
            .append_session_event(
                harness.session.id,
                None,
                Some(harness.turn_id),
                SessionEventType::Notice,
                json!({
                    "message_id": format!("gap-note-{idx}"),
                    "content": format!("note-{idx}"),
                    "order_seq": 2,
                }),
            )
            .await
            .unwrap();
        harness.state.publish_event(event).await;
    }

    let assistant_complete = store
        .append_session_event(
            harness.session.id,
            None,
            Some(harness.turn_id),
            SessionEventType::AssistantComplete,
            json!({
                "message_id": harness.assistant_message_id.0,
                "content": fixture.assistant_content.clone(),
                "full_content": fixture.assistant_content.clone(),
                "order_seq": 3,
            }),
        )
        .await
        .unwrap();
    harness.state.publish_event(assistant_complete).await;
    store
        .update_session_turn_status(
            harness.session.id,
            harness.turn_id,
            SessionTurnStatus::Completed,
            Some(assistant_complete.seq),
            None,
            updated_at,
        )
        .await
        .unwrap();

    harness
        .state
        .refresh_session_head_cache(harness.session.id)
        .await;
    harness
        .state
        .ensure_workspace_active_snapshot_hydrated(harness.workspace.id)
        .await
        .unwrap();
}

async fn setup_replay_fixture(event_count: usize) -> ReplayFixture {
    let harness = setup_projection_harness().await;
    let store = harness
        .state
        .store_for_session(harness.session.id)
        .await
        .unwrap();

    let mut seqs = Vec::with_capacity(event_count);
    for i in 0..event_count {
        let event = store
            .append_session_event(
                harness.session.id,
                None,
                None,
                SessionEventType::Notice,
                json!({ "i": i }),
            )
            .await
            .unwrap();
        harness.state.publish_event(event.clone()).await;
        seqs.push(event.seq);
    }

    harness
        .state
        .refresh_session_head_cache(harness.session.id)
        .await;
    harness
        .state
        .ensure_workspace_active_snapshot_hydrated(harness.workspace.id)
        .await
        .unwrap();

    ReplayFixture { harness, seqs }
}

async fn connect_workspace_stream(addr: &str, workspace_id: WorkspaceId) -> Ws {
    let ws_url = format!(
        "{}/api/workspaces/{}/stream",
        addr.replace("http://", "ws://"),
        workspace_id.0
    );
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    socket
}

async fn subscribe_and_observe(
    socket: &mut Ws,
    session_id: SessionId,
    after_seq: i64,
    max_wait: Duration,
) -> ReplayObservation {
    let subscribe = json!({
        "type": "subscribe",
        "scope": "active",
        "sessions": [{
            "session_id": session_id.0,
            "replay": {
                "mode": "resume",
                "after_seq": after_seq,
            },
        }],
    })
    .to_string();
    socket
        .send(WsMessage::Text(subscribe.into()))
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + max_wait;
    let mut observed = ReplayObservation::default();
    while tokio::time::Instant::now() < deadline {
        let wait = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_millis(250));
        let next = tokio::time::timeout(wait, socket.next()).await;
        let Ok(Some(Ok(WsMessage::Text(text)))) = next else {
            continue;
        };
        let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotStreamMessage>(&text)
        else {
            continue;
        };
        match message {
            WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match event.as_ref() {
                WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                    if delta.session_id != session_id {
                        continue;
                    }
                    let Some(event) = delta.event.as_ref() else {
                        continue;
                    };
                    if event.seq > 0 {
                        observed.seqs.push(event.seq);
                    }
                }
                WorkspaceActiveSnapshotEvent::SessionGap {
                    session_id: gap_session_id,
                    reason,
                    ..
                } if *gap_session_id == session_id => {
                    observed.gap_reason = reason.clone();
                }
                WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. }
                    if head.session.id == session_id =>
                {
                    observed.seed_last_event_seq = Some(head.last_event_seq);
                }
                _ => {}
            },
            WorkspaceActiveSnapshotStreamMessage::HeadsBatch { deltas, .. } => {
                for delta in deltas {
                    if delta.session_id != session_id {
                        continue;
                    }
                    let Some(event) = delta.event else {
                        continue;
                    };
                    if event.seq > 0 {
                        observed.seqs.push(event.seq);
                    }
                }
            }
            _ => {}
        }
    }
    observed
}

fn event_type_strings(events: &[ctx_core::models::SessionEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| serde_json::to_value(&event.event_type).unwrap())
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn fixture_projection_equivalence_aligns_snapshot_heads_and_replay() {
    let fixture = &projection_fixtures().active_projection_equivalence;
    let harness = setup_projection_harness().await;
    let durable_seqs = seed_active_projection_case(&harness, fixture).await;
    assert_eq!(durable_seqs, vec![1, 2, 3, 4]);

    let active_snapshot: ctx_core::models::WorkspaceActiveSnapshot = harness
        .server
        .client
        .get(format!(
            "{}/api/workspaces/{}/active_snapshot",
            harness.server.base_url, harness.workspace.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let active_heads: ctx_core::models::WorkspaceActiveHeadBatch = harness
        .server
        .client
        .get(format!(
            "{}/api/workspaces/{}/active_heads",
            harness.server.base_url, harness.workspace.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    harness
        .state
        .workspaces
        .workspace_active_snapshot
        .remove_session_head(harness.session.id)
        .await;
    let session_head: SessionHeadSnapshot = harness
        .server
        .client
        .get(format!(
            "{}/api/sessions/{}/head?include_events=true",
            harness.server.base_url, harness.session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_events: ctx_core::models::SessionEventsPage = harness
        .server
        .client
        .get(format!(
            "{}/api/sessions/{}/events?tail=10",
            harness.server.base_url, harness.session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_events_with_transient: ctx_core::models::SessionEventsPage = harness
        .server
        .client
        .get(format!(
            "{}/api/sessions/{}/events?tail=10&include_transient=true",
            harness.server.base_url, harness.session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let snapshot_task = active_snapshot
        .active
        .tasks
        .iter()
        .find(|task| task.task.id == harness.task.id)
        .expect("active snapshot should contain seeded task");
    let heads_head = active_heads
        .heads
        .iter()
        .find(|head| head.session.id == harness.session.id)
        .expect("active heads should contain seeded head");
    let persisted_durable_seqs = durable_seqs
        .iter()
        .copied()
        .take(fixture.expected.persisted_event_types.len())
        .collect::<Vec<_>>();

    assert_eq!(
        session_head.last_event_seq,
        fixture.expected.head_last_event_seq
    );
    assert_eq!(
        session_events
            .events
            .iter()
            .map(|event| event.seq)
            .collect::<Vec<_>>(),
        persisted_durable_seqs
    );
    assert_eq!(
        event_type_strings(&session_events.events),
        fixture.expected.persisted_event_types
    );
    assert_eq!(
        session_events_with_transient
            .events
            .iter()
            .filter(|event| !event.transient)
            .map(|event| event.seq)
            .collect::<Vec<_>>(),
        persisted_durable_seqs
    );
    assert_eq!(
        event_type_strings(&session_events_with_transient.events),
        fixture.expected.stable_event_types
    );
    assert_eq!(
        session_events_with_transient.events.last().map(|event| {
            (
                serde_json::to_value(&event.event_type)
                    .unwrap()
                    .as_str()
                    .map(str::to_string),
                event.transient,
            )
        }),
        Some((Some("assistant_complete".to_string()), true))
    );
    assert_eq!(
        event_type_strings(&session_head.events),
        fixture.expected.persisted_event_types
    );
    assert!(heads_head.events.is_empty());
    assert!(snapshot_task.primary_session_head.is_none());
    assert_eq!(
        session_head
            .events
            .iter()
            .map(|event| event.seq)
            .collect::<Vec<_>>(),
        session_events
            .events
            .iter()
            .map(|event| event.seq)
            .collect::<Vec<_>>()
    );
    assert!(snapshot_task.primary_session.last_event_seq.is_none());
    assert_eq!(
        heads_head
            .tool_summaries
            .iter()
            .map(|tool| tool.tool_call_id.clone())
            .collect::<Vec<_>>(),
        vec![fixture.tool_call_id.clone()]
    );
    assert_eq!(
        session_head
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        vec![
            fixture.user_content.clone(),
            fixture.assistant_content.clone()
        ]
    );

    for replay in &fixture.replay_expectations {
        let mut socket =
            connect_workspace_stream(&harness.server.base_url, harness.workspace.id).await;
        let observed = subscribe_and_observe(
            &mut socket,
            harness.session.id,
            replay.after_seq,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(
            observed.seqs, replay.expected_seqs,
            "after_seq={} should replay the expected projection-cursor suffix",
            replay.after_seq
        );
        assert_eq!(
            observed.gap_reason.is_some(),
            replay.expect_gap,
            "after_seq={} gap expectation mismatch",
            replay.after_seq
        );
        assert_eq!(
            observed.seed_last_event_seq, replay.expected_seed_last_event_seq,
            "after_seq={} seed expectation mismatch",
            replay.after_seq
        );
    }
}

#[tokio::test]
async fn fixture_gap_rehydrates_with_seed_after_replay_limit() {
    let fixture = &projection_fixtures().session_gap_seed_rehydrate;
    let harness = setup_projection_harness().await;
    seed_gap_case(&harness, fixture).await;

    let session_head: SessionHeadSnapshot = harness
        .server
        .client
        .get(format!(
            "{}/api/sessions/{}/head?include_events=true",
            harness.server.base_url, harness.session.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        session_head.last_event_seq,
        fixture.expected.head_last_event_seq
    );

    let mut socket = connect_workspace_stream(&harness.server.base_url, harness.workspace.id).await;
    let observed = subscribe_and_observe(
        &mut socket,
        harness.session.id,
        fixture.after_seq,
        Duration::from_secs(2),
    )
    .await;

    assert_eq!(
        observed.gap_reason.as_deref(),
        Some(fixture.expected.reason.as_str())
    );
    assert_eq!(
        observed.seed_last_event_seq,
        Some(fixture.expected.head_last_event_seq)
    );
    assert!(observed.seqs.iter().all(|seq| *seq > fixture.after_seq));
    assert!(observed.seqs.windows(2).all(|window| window[0] < window[1]));
}

#[tokio::test]
async fn property_replay_respects_after_seq_and_monotonicity() {
    let fixture = setup_replay_fixture(15).await;
    assert!(fixture.seqs.windows(2).all(|window| window[0] < window[1]));

    let mut socket = connect_workspace_stream(
        &fixture.harness.server.base_url,
        fixture.harness.workspace.id,
    )
    .await;
    let after_seqs = [
        0,
        fixture.seqs[0],
        fixture.seqs[3],
        fixture.seqs[7],
        fixture.seqs[14],
        fixture.seqs[14] + 10,
    ];

    for after_seq in after_seqs {
        let expected: Vec<i64> = fixture
            .seqs
            .iter()
            .copied()
            .filter(|seq| *seq > after_seq)
            .collect();
        let observed = subscribe_and_observe(
            &mut socket,
            fixture.harness.session.id,
            after_seq,
            Duration::from_secs(3),
        )
        .await;
        for seq in &observed.seqs {
            assert!(
                expected.contains(seq),
                "after_seq={after_seq}: replayed seq {seq} not in expected set {expected:?}"
            );
            assert!(
                *seq > after_seq,
                "after_seq={after_seq}: replayed seq {seq} must be greater than cursor"
            );
        }
        if expected.is_empty() {
            assert!(
                observed.seqs.is_empty(),
                "after_seq={after_seq}: expected no deltas past replay tail"
            );
        }
        assert!(
            observed.seqs.windows(2).all(|window| window[0] < window[1]),
            "after_seq={after_seq}"
        );
    }
}

#[tokio::test]
async fn property_replay_is_idempotent_for_same_after_seq() {
    let fixture = setup_replay_fixture(12).await;
    let mut socket = connect_workspace_stream(
        &fixture.harness.server.base_url,
        fixture.harness.workspace.id,
    )
    .await;
    let after_seq = fixture.seqs[4];
    let expected: Vec<i64> = fixture
        .seqs
        .iter()
        .copied()
        .filter(|seq| *seq > after_seq)
        .collect();

    let first = subscribe_and_observe(
        &mut socket,
        fixture.harness.session.id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;
    let second = subscribe_and_observe(
        &mut socket,
        fixture.harness.session.id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;

    assert_eq!(first.seqs, expected);
    assert_eq!(second.seqs, expected);
    assert_eq!(first.seqs, second.seqs);
    assert!(
        first.gap_reason.is_none(),
        "unexpected session_gap on first replay"
    );
    assert!(
        second.gap_reason.is_none(),
        "unexpected session_gap on second replay"
    );
    assert!(first.seed_last_event_seq.is_none());
    assert!(second.seed_last_event_seq.is_none());
}

#[tokio::test]
async fn property_replay_after_seq_zero_avoids_gap_without_compact_seed_replay() {
    let fixture = setup_replay_fixture(8).await;
    let mut socket = connect_workspace_stream(
        &fixture.harness.server.base_url,
        fixture.harness.workspace.id,
    )
    .await;

    let observed = subscribe_and_observe(
        &mut socket,
        fixture.harness.session.id,
        0,
        Duration::from_secs(2),
    )
    .await;

    for seq in &observed.seqs {
        assert!(
            fixture.seqs.contains(seq),
            "replayed seq {seq} not found in fixture sequence {:?}",
            fixture.seqs
        );
    }
    assert!(observed.seqs.windows(2).all(|window| window[0] < window[1]));
    assert!(
        observed.gap_reason.is_none(),
        "after_seq=0 should not emit session_gap"
    );
    assert!(
        observed.seed_last_event_seq.is_none(),
        "compact-only hydration should not be treated as a replay-capable seed"
    );
}

#[tokio::test]
async fn property_replay_past_tail_emits_gap_without_deltas() {
    let fixture = setup_replay_fixture(10).await;
    let mut socket = connect_workspace_stream(
        &fixture.harness.server.base_url,
        fixture.harness.workspace.id,
    )
    .await;
    let after_seq = fixture.seqs.last().copied().unwrap_or(0) + 100;

    let observed = subscribe_and_observe(
        &mut socket,
        fixture.harness.session.id,
        after_seq,
        Duration::from_secs(2),
    )
    .await;

    assert!(
        observed.seqs.is_empty(),
        "expected no delta replay beyond session tail"
    );
    assert!(observed.seed_last_event_seq.is_none());
}
