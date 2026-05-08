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
