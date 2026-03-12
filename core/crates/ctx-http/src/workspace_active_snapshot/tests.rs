mod compact_head_tests {
    use super::super::trim::ACTIVE_HEAD_TURN_LIMIT;
    use super::super::*;
    use chrono::{TimeZone, Utc};
    use ctx_core::ids::{MessageId, TurnId};
    use ctx_core::models::{MessageDelivery, MessageRole, SessionTurnToolSummary};

    #[test]
    fn compact_active_head_keeps_last_turns_and_filters_messages_and_tools() {
        let session = SessionMetadata {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ctx_core::models::ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "p".to_string(),
            model_id: "m".to_string(),
            title: "t".to_string(),
            agent_role: "assistant".to_string(),
            status: ctx_core::models::SessionStatus::Active,
            provider_session_ref: None,
            created_at: Utc.timestamp_opt(0, 0).unwrap(),
            updated_at: Utc.timestamp_opt(0, 0).unwrap(),
        };

        let mut head = SessionHeadSnapshot {
            session,
            turns: Vec::new(),
            tool_summaries: Vec::new(),
            events: Vec::new(),
            messages: Vec::new(),
            last_event_seq: 123,
            state_rev: 0,
            activity: SessionActivityState::default(),
            has_more_turns: false,
            history_cursor: None,
            has_more_history: false,
            summary_checkpoint: None,
            head_window: ctx_core::models::SessionHeadWindow::default(),
        };

        for i in 0_i64..10 {
            let turn_id = TurnId::new();
            head.turns.push(SessionTurn {
                turn_id,
                session_id: head.session.id,
                run_id: None,
                user_message_id: None,
                status: ctx_core::models::SessionTurnStatus::Completed,
                start_seq: Some(i),
                end_seq: Some(i),
                started_at: Utc.timestamp_opt(i, 0).unwrap(),
                updated_at: Utc.timestamp_opt(i, 0).unwrap(),
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 1,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 1,
                tool_failed: 0,
            });
            head.messages.push(Message {
                id: MessageId::new(),
                session_id: head.session.id,
                task_id: head.session.task_id,
                run_id: None,
                turn_id: Some(turn_id),
                turn_sequence: Some(i),
                order_seq: None,
                role: MessageRole::User,
                content: format!("m{i}"),
                attachments: Vec::new(),
                delivery: MessageDelivery::Immediate,
                delivered_at: None,
                created_at: Utc.timestamp_opt(i, 0).unwrap(),
            });
            head.tool_summaries.push(SessionTurnToolSummary {
                session_id: head.session.id,
                tool_call_id: format!("tool{i}"),
                turn_id,
                tool_kind: Some("shell".to_string()),
                title: Some("x".to_string()),
                status: Some("completed".to_string()),
                input_preview: None,
                output_preview: None,
                first_event_seq: Some(i),
                input_truncated: None,
                input_original_bytes: None,
                output_truncated: None,
                output_original_bytes: None,
                created_at: Utc.timestamp_opt(i, 0).unwrap(),
                updated_at: Utc.timestamp_opt(i, 0).unwrap(),
            });
        }

        let compact = compact_active_head_snapshot(&head);
        assert_eq!(compact.turns.len(), ACTIVE_HEAD_TURN_LIMIT);
        assert!(compact.events.is_empty());
        let kept: std::collections::HashSet<_> = compact.turns.iter().map(|t| t.turn_id).collect();
        assert!(compact
            .messages
            .iter()
            .all(|m| m.turn_id.map(|id| kept.contains(&id)).unwrap_or(true)));
        assert!(compact
            .tool_summaries
            .iter()
            .all(|t| kept.contains(&t.turn_id)));
    }
}

mod replay_tests {
    use super::super::trim::session_metadata_from_session;
    use super::super::*;
    use chrono::{TimeZone, Utc};
    use ctx_core::models::{SessionStatus, TaskStatus};

    fn replay_task(session: &Session) -> WorkspaceActiveTaskSummary {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        WorkspaceActiveTaskSummary {
            task: Task {
                id: session.task_id,
                workspace_id: session.workspace_id,
                title: "task".to_string(),
                description: None,
                status: TaskStatus::Running,
                created_at: now,
                updated_at: now,
                exec_plan_id: None,
                primary_session_id: Some(session.id),
                primary_worktree_id: Some(session.worktree_id),
                archived_at: None,
                assistant_seen_at: None,
                last_activity_at: None,
                last_assistant_message_at: None,
                has_active_session: true,
            },
            primary_session: SessionSnapshotSummary {
                session: session_metadata_from_session(session),
                last_message_at: None,
                last_message_preview: None,
                last_event_seq: Some(0),
                state_rev: 0,
                activity: SessionActivityState::default(),
                unread: None,
            },
            primary_session_head: None,
            sessions: vec![SessionSnapshotSummary {
                session: session_metadata_from_session(session),
                last_message_at: None,
                last_message_preview: None,
                last_event_seq: Some(0),
                state_rev: 0,
                activity: SessionActivityState::default(),
                unread: None,
            }],
            sort_at: now,
        }
    }

    fn replay_session(session_id: SessionId) -> Session {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        Session {
            id: session_id,
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ctx_core::models::ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            title: "session".to_string(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn replay_records_delta_without_event() {
        let session_id = SessionId(uuid::Uuid::nil());
        let delta = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            state_rev: 0,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let delta_next = SessionHeadDelta {
            session_id,
            last_event_seq: 6,
            state_rev: 0,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let mut state = SessionReplayState::default();
        state.record(&delta);
        state.record(&delta_next);

        match state.replay(5, 10) {
            SessionReplayResult::Replay { deltas, last_sent } => {
                assert_eq!(last_sent, 6);
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].last_event_seq, 6);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_session_stream_emits_gap_then_seed_for_resuming_cursor() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let session = replay_session(SessionId::new());
        let mut head = new_head_snapshot(&session);
        head.last_event_seq = 7;
        hub.hydrate_snapshot(
            session.workspace_id,
            1,
            0,
            vec![replay_task(&session)],
            vec![head.clone()],
        )
        .await;

        match hub
            .replay_session_stream(session.workspace_id, session.id, 3, 50)
            .await
        {
            WorkspaceSessionReplay::Replay { items, last_sent } => {
                assert_eq!(last_sent, 7);
                assert_eq!(items.len(), 2);
                match &items[0] {
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => {
                        assert_eq!(*session_id, session.id);
                        assert_eq!(*after_seq, 3);
                        assert_eq!(reason.as_deref(), Some("missing_replay_events"));
                    }
                    other => panic!("expected gap, got {other:?}"),
                }
                match &items[1] {
                    WorkspaceSessionReplayItem::Seed(seed) => {
                        assert_eq!(seed.session.id, session.id);
                        assert_eq!(seed.last_event_seq, 7);
                    }
                    other => panic!("expected seed, got {other:?}"),
                }
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_session_stream_suppresses_gap_for_non_resuming_cursor() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let session = replay_session(SessionId::new());
        let mut head = new_head_snapshot(&session);
        head.last_event_seq = 11;
        hub.hydrate_snapshot(
            session.workspace_id,
            1,
            0,
            vec![replay_task(&session)],
            vec![head],
        )
        .await;

        match hub
            .replay_session_stream(session.workspace_id, session.id, 0, 50)
            .await
        {
            WorkspaceSessionReplay::Replay { items, last_sent } => {
                assert!(items.is_empty());
                assert_eq!(last_sent, 11);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }
}
