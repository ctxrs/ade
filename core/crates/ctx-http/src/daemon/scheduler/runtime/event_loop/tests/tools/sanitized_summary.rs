use super::*;

#[tokio::test]
async fn tool_result_uses_sanitized_payload_for_persisted_summary() {
    let data_dir = tempdir().expect("temp dir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let mut app_state = AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    );
    app_state.core.tool_output_spool_enabled = true;
    std::fs::create_dir_all(&app_state.core.tool_output_spool_dir).expect("tool output spool dir");
    let state = Arc::new(app_state);

    let workspace_root = data_dir.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root)
        .await
        .expect("workspace root");
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .expect("worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("task");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("session");
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .expect("primary session");
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .expect("workspace session index");
    state.sessions.remember_session_meta(&session).await;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
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
        .expect("insert turn");

    let seeded = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .expect("load baseline head")
        .expect("baseline head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(seeded)
        .await;

    let (ev_tx, ev_rx) = mpsc::channel(8);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, _start_progress_rx) =
        tokio::sync::watch::channel(TurnStartProgress::Pending);
    let loop_task = tokio::spawn(run_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(&state),
        store: store.clone(),
        session_id: session.id,
        task_id: task.id,
        workspace_id: workspace.id,
        worktree_id: worktree.id,
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
        message_id: MessageId::new(),
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
            event_type: SessionEventType::ToolResult,
            payload_json: json!({
                "tool_call_id": "toolu_exec_result_1",
                "kind": "unknown",
                "tool_name": "unknown",
                "title": "unknown",
                "status": "completed",
                "output_preview": "1"
            }),
        })
        .await
        .expect("send tool result");
    drop(ev_tx);

    events_done_rx.await.expect("event loop completion");
    loop_task.await.expect("event loop join");

    let tool = store
        .get_session_turn_tool(session.id, "toolu_exec_result_1")
        .await
        .expect("load tool")
        .expect("persisted tool summary");
    assert_eq!(tool.order_seq, 1);
    assert_eq!(tool.tool_kind.as_deref(), Some("execute"));
    assert_eq!(tool.provider_tool_name.as_deref(), Some("Bash"));
    assert_eq!(tool.title.as_deref(), Some("Bash"));
    assert_eq!(tool.output_text.as_deref(), Some("1"));

    let events = store
        .list_session_events_for_turn(session.id, turn_id, false)
        .await
        .expect("load persisted events");
    let result_event = events
        .into_iter()
        .find(|event| matches!(event.event_type, SessionEventType::ToolResult))
        .expect("tool result event");
    assert!(result_event.payload_json.get("output_artifact").is_none());
}
