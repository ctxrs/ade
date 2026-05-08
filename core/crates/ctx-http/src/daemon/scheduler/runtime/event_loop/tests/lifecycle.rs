use super::*;

#[tokio::test]
async fn turn_started_event_promotes_starting_turn_to_running() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&fixture.state),
        store: fixture.store.clone(),
        session_id: fixture.session_id,
        task_id: fixture.task_id,
        workspace_id: fixture.workspace_id,
        worktree_id: fixture.worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: fixture.workspace_root.clone(),
        workdir_canonical: Some(fixture.workspace_root.clone()),
        workdir_str: fixture.workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id: fixture.run_id,
        turn_id: fixture.turn_id,
        message_id: fixture.message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnStarted,
            payload_json: json!({
                "session_id": fixture.session_id.0,
                "turn_id": fixture.turn_id.0,
            }),
        })
        .await
        .expect("send turn started event");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    assert_eq!(*start_progress_rx.borrow(), TurnStartProgress::Started);
    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Running);
}

#[tokio::test]
async fn event_loop_exits_without_persisting_when_app_state_owner_is_gone() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;
    let LoopFixture {
        state,
        store,
        workspace_id,
        worktree_id,
        task_id,
        session_id,
        turn_id,
        run_id,
        message_id,
        workspace_root,
    } = fixture;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id,
        task_id,
        workspace_id,
        worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: workspace_root.clone(),
        workdir_canonical: Some(workspace_root.clone()),
        workdir_str: workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id,
        turn_id,
        message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    drop(state);

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnStarted,
            payload_json: json!({}),
        })
        .await
        .expect("send event after dropping app state owner");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await
        .expect("load persisted events");
    assert!(events.is_empty());
}

#[tokio::test]
async fn event_loop_drops_provider_events_after_turn_terminalized_by_store() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;

    crate::daemon::scheduler::terminal::finalize_failed_turn(
        &fixture.state,
        fixture.session_id,
        Some(fixture.run_id),
        fixture.turn_id,
        fixture.message_id,
        crate::daemon::scheduler::terminal::FailedTurnTerminalization {
            message: "provider usage limit exceeded",
            reason: Some("usage_limit"),
            details: None,
            kind: Some(json!("usageLimitExceeded")),
            emit_error_event: true,
        },
    )
    .await
    .expect("terminalize turn");

    let terminal_events = fixture
        .store
        .list_session_events_for_turn(fixture.session_id, fixture.turn_id, false)
        .await
        .expect("load terminal events");
    assert_eq!(terminal_events.len(), 2);

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&fixture.state),
        store: fixture.store.clone(),
        session_id: fixture.session_id,
        task_id: fixture.task_id,
        workspace_id: fixture.workspace_id,
        worktree_id: fixture.worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: fixture.workspace_root.clone(),
        workdir_canonical: Some(fixture.workspace_root.clone()),
        workdir_str: fixture.workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id: fixture.run_id,
        turn_id: fixture.turn_id,
        message_id: fixture.message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::ToolCall,
            payload_json: json!({
                "tool_call_id": "post-terminal-tool",
                "status": "running",
                "toolCall": {
                    "name": "Bash",
                    "kind": "execute"
                },
                "rawInput": {
                    "command": "pwd"
                }
            }),
        })
        .await
        .expect("send post-terminal tool event");
    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::AssistantComplete,
            payload_json: json!({"full_content": "late answer"}),
        })
        .await
        .expect("send post-terminal assistant event");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let events = fixture
        .store
        .list_session_events_for_turn(fixture.session_id, fixture.turn_id, false)
        .await
        .expect("load persisted events");
    assert_eq!(events.len(), terminal_events.len());
    assert!(events.iter().all(|event| {
        !matches!(
            event.event_type,
            SessionEventType::ToolCall | SessionEventType::AssistantComplete
        )
    }));

    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Failed);
    assert_eq!(turn.tool_total, 0);
    assert_eq!(turn.tool_running, 0);
}

#[tokio::test]
async fn event_loop_persists_assistant_complete_after_completed_terminal_event() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&fixture.state),
        store: fixture.store.clone(),
        session_id: fixture.session_id,
        task_id: fixture.task_id,
        workspace_id: fixture.workspace_id,
        worktree_id: fixture.worktree_id,
        provider_id: "fake".to_string(),
        model_id: "model".to_string(),
        session_root_kind: "primary".to_string(),
        execution_environment_label: "host".to_string(),
        perf_run_id: None,
        workdir_root: fixture.workspace_root.clone(),
        workdir_canonical: Some(fixture.workspace_root.clone()),
        workdir_str: fixture.workspace_root.to_string_lossy().to_string(),
        run_started_at: Instant::now(),
        run_id: fixture.run_id,
        turn_id: fixture.turn_id,
        message_id: fixture.message_id,
        provider_session_ref: None,
        codex_home: None,
        context_window_metrics: None,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::new(Mutex::new(OrderSeqState::new(1))),
    }));

    fixture
        .store
        .persist_turn_terminal_events(
            fixture.session_id,
            Some(fixture.run_id),
            fixture.turn_id,
            vec![(
                SessionEventType::TurnFinished,
                json!({"status": "completed"}),
            )],
        )
        .await
        .expect("persist completed terminal event before assistant complete");

    ev_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::AssistantComplete,
            payload_json: json!({
                "full_content": "late assistant answer",
                "message_id": "provider-message-1",
                "order_seq": 2,
            }),
        })
        .await
        .expect("send late assistant complete");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let messages = fixture
        .store
        .list_messages_for_session(fixture.session_id)
        .await
        .expect("load messages");
    assert!(messages.iter().any(|message| {
        matches!(message.role, ctx_core::models::MessageRole::Assistant)
            && message.content == "late assistant answer"
    }));

    let events = fixture
        .store
        .list_session_events_for_turn(fixture.session_id, fixture.turn_id, false)
        .await
        .expect("load turn events");
    assert!(events.iter().any(|event| {
        matches!(event.event_type, SessionEventType::AssistantMessageInserted)
            && event
                .payload_json
                .get("content")
                .and_then(|value| value.as_str())
                == Some("late assistant answer")
    }));
}

#[tokio::test]
async fn start_deadline_failure_finalizes_starting_turn_as_failed() {
    let data_dir = tempdir().expect("temp dir");
    let fixture = build_loop_fixture(data_dir.path(), "fake", "model").await;
    let adapter: Arc<dyn ProviderAdapter> = Arc::new(FakeProviderAdapter::new());
    let (event_tx, _event_rx) = mpsc::channel(8);
    let handle = adapter
        .run(
            TurnInput {
                content: "slow-diff-test".to_string(),
                attachments: Vec::new(),
                context_blocks: Vec::new(),
                model_id: None,
            },
            fixture.workspace_root.clone(),
            HashMap::new(),
            event_tx.clone(),
            ProviderRunHooks::default(),
        )
        .await
        .expect("run handle");
    let (_start_progress_tx, start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);

    fail_starting_turn(
        &fixture.state,
        fixture.session_id,
        RunningTurn {
            adapter,
            handle,
            run_id: fixture.run_id,
            turn_id: fixture.turn_id,
            message_id: fixture.message_id,
            provider_id: "fake".to_string(),
            model_id: "model".to_string(),
            execution_environment_label: "host".to_string(),
            session_root_kind: "primary".to_string(),
            event_tx,
            events_done: None,
            start_progress: start_progress_rx,
            start_deadline: tokio::time::Instant::now(),
            mcp_token: None,
        },
        "provider did not report turn start before deadline",
    )
    .await;

    let turn = fixture
        .store
        .get_session_turn(fixture.session_id, fixture.turn_id)
        .await
        .expect("load turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Failed);

    let events = fixture
        .store
        .list_session_events_for_turn(fixture.session_id, fixture.turn_id, false)
        .await
        .expect("load turn events");
    assert!(events.iter().any(|event| {
        matches!(&event.event_type, SessionEventType::Error)
            && event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                == Some("start_not_acknowledged")
    }));
    assert!(events.iter().any(|event| {
        matches!(&event.event_type, SessionEventType::TurnFinished)
            && event
                .payload_json
                .get("status")
                .and_then(|value| value.as_str())
                == Some("failed")
            && event.payload_json.get("kind") == Some(&json!("start_not_acknowledged"))
    }));
}
