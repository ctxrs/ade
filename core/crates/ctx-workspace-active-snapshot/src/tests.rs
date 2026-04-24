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
            reasoning_effort: None,
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
            projection_rev: 123,
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
                provider_tool_name: Some("Bash".to_string()),
                title: Some("x".to_string()),
                subtitle: Some("pwd".to_string()),
                status: Some("completed".to_string()),
                input_preview: None,
                output_preview: None,
                order_seq: i,
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

mod delta_tests {
    use super::super::delta::{apply_head_delta, apply_session_summary_delta};
    use super::super::trim::{new_head_snapshot, session_metadata_from_session};
    use super::super::*;
    use chrono::{TimeZone, Utc};
    use ctx_core::ids::TurnId;
    use ctx_core::models::SessionTurnStatus;

    fn test_session(parent_session_id: Option<SessionId>) -> Session {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        Session {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ctx_core::models::ExecutionEnvironment::Host,
            parent_session_id,
            relationship: parent_session_id.map(|_| "sub_agent".to_string()),
            provider_id: "fake".to_string(),
            model_id: "model-a".to_string(),
            reasoning_effort: Some("medium".to_string()),
            title: "session".to_string(),
            agent_role: "assistant".to_string(),
            status: ctx_core::models::SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn apply_head_delta_updates_session_activity_and_turn_shape() {
        let session = test_session(None);
        let mut head = new_head_snapshot(&session);
        let mut updated_session = session_metadata_from_session(&session);
        updated_session.model_id = "model-b".to_string();
        updated_session.reasoning_effort = Some("high".to_string());

        let delta = SessionHeadDelta {
            session_id: session.id,
            last_event_seq: 11,
            projection_rev: 11,
            state_rev: 11,
            emitted_at_ms: None,
            session: Some(updated_session.clone()),
            activity: Some(SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            }),
            event: None,
            turn: Some(SessionTurn {
                turn_id: TurnId::new(),
                session_id: session.id,
                run_id: None,
                user_message_id: None,
                status: SessionTurnStatus::Running,
                start_seq: Some(11),
                end_seq: None,
                started_at: Utc.timestamp_opt(0, 0).unwrap(),
                updated_at: Utc.timestamp_opt(11, 0).unwrap(),
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 3,
                tool_pending: 1,
                tool_running: 1,
                tool_completed: 1,
                tool_failed: 0,
            }),
            message: None,
            tool_summaries: Vec::new(),
        };

        apply_head_delta(&mut head, &delta);

        assert_eq!(head.session.model_id, updated_session.model_id);
        assert_eq!(
            head.session.reasoning_effort,
            updated_session.reasoning_effort
        );
        assert_eq!(
            head.activity.last_turn_status,
            Some(SessionTurnStatus::Running)
        );
        assert!(head.activity.is_working);
        assert_eq!(head.turns.len(), 1);
        assert_eq!(head.turns[0].tool_total, 3);
        assert_eq!(head.turns[0].tool_pending, 1);
        assert_eq!(head.turns[0].tool_running, 1);
        assert_eq!(head.turns[0].tool_completed, 1);
    }

    #[test]
    fn apply_session_summary_delta_updates_activity_from_activity_only_event() {
        let session = test_session(None);
        let mut summary = SessionSnapshotSummary {
            session: session_metadata_from_session(&session),
            last_message_at: None,
            last_message_preview: None,
            last_event_seq: Some(4),
            projection_rev: 4,
            state_rev: 4,
            activity: SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            },
            unread: None,
        };
        let changed = apply_session_summary_delta(
            &mut summary,
            &SessionSummaryDelta {
                session_id: session.id,
                task_id: session.task_id,
                activity: Some(SessionActivityState {
                    is_working: false,
                    last_turn_status: Some(SessionTurnStatus::Completed),
                }),
                last_message_at: None,
                last_message_preview: None,
                last_event_seq: Some(5),
                projection_rev: Some(4),
                state_rev: Some(5),
                emitted_at_ms: None,
            },
        );

        assert!(changed);
        assert!(!summary.activity.is_working);
        assert_eq!(
            summary.activity.last_turn_status,
            Some(SessionTurnStatus::Completed)
        );
        assert_eq!(summary.last_event_seq, Some(5));
    }

    #[tokio::test]
    async fn publish_session_head_delta_keeps_live_subagent_session_heads_hot() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let parent = test_session(None);
        let subagent = test_session(Some(parent.id));
        let mut seeded = new_head_snapshot(&subagent);
        seeded.last_event_seq = 5;
        seeded.projection_rev = 5;
        hub.update_session_head(seeded).await;
        let delta = SessionHeadDelta {
            session_id: subagent.id,
            last_event_seq: 7,
            projection_rev: 7,
            state_rev: 7,
            emitted_at_ms: None,
            session: Some(session_metadata_from_session(&subagent)),
            activity: Some(SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            }),
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        hub.publish_session_head_delta(subagent.workspace_id, &subagent, delta, true)
            .await;

        let cached = hub
            .get_session_head(subagent.id)
            .await
            .expect("seeded subagent head should stay hot");
        assert_eq!(cached.last_event_seq, 7);
        assert_eq!(cached.projection_rev, 7);
        assert!(cached.activity.is_working);

        let active_heads = hub.active_heads(subagent.workspace_id).await;
        assert!(
            active_heads.heads.is_empty(),
            "subagent heads should not inflate the workspace active-head batch"
        );
    }

    #[tokio::test]
    async fn publish_session_head_delta_does_not_seed_cold_subagent_from_single_delta() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let parent = test_session(None);
        let subagent = test_session(Some(parent.id));
        let delta = SessionHeadDelta {
            session_id: subagent.id,
            last_event_seq: 7,
            projection_rev: 7,
            state_rev: 7,
            emitted_at_ms: None,
            session: Some(session_metadata_from_session(&subagent)),
            activity: Some(SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            }),
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        hub.publish_session_head_delta(subagent.workspace_id, &subagent, delta, true)
            .await;

        assert!(
            hub.get_session_head(subagent.id).await.is_none(),
            "cold subagent misses should not be synthesized from a single live delta"
        );

        let active_heads = hub.active_heads(subagent.workspace_id).await;
        assert!(
            active_heads.heads.is_empty(),
            "subagent heads should not inflate the workspace active-head batch"
        );
    }

    #[tokio::test]
    async fn publish_session_head_delta_marks_cold_primary_head_unservable_until_hydrated() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let primary = test_session(None);
        let delta = SessionHeadDelta {
            session_id: primary.id,
            last_event_seq: 7,
            projection_rev: 7,
            state_rev: 7,
            emitted_at_ms: None,
            session: Some(session_metadata_from_session(&primary)),
            activity: Some(SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            }),
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };

        hub.publish_session_head_delta(primary.workspace_id, &primary, delta, true)
            .await;

        assert!(
            hub.get_session_head(primary.id).await.is_none(),
            "cold primary misses should stay store-backed until a hydrated head is loaded"
        );

        let active_heads = hub.active_heads(primary.workspace_id).await;
        assert_eq!(active_heads.heads.len(), 1);
        assert_eq!(active_heads.heads[0].session.id, primary.id);

        let mut hydrated = new_head_snapshot(&primary);
        hydrated.last_event_seq = 7;
        hydrated.projection_rev = 7;
        hub.update_session_head(hydrated.clone()).await;

        let cached = hub
            .get_session_head(primary.id)
            .await
            .expect("hydrated head should become serveable");
        assert_eq!(cached.last_event_seq, hydrated.last_event_seq);
        assert_eq!(cached.projection_rev, hydrated.projection_rev);
    }

    #[tokio::test]
    async fn compact_session_head_cache_does_not_seed_replay_head_cache() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let primary = test_session(None);
        let mut compact = new_head_snapshot(&primary);
        compact.last_event_seq = 7;
        compact.projection_rev = 7;

        hub.update_compact_session_head(compact.clone()).await;

        assert!(
            hub.get_session_head(primary.id).await.is_none(),
            "compact heads must not be treated as replay-capable cache seeds"
        );
        let cached = hub
            .get_cached_session_head_for_read(primary.id)
            .await
            .expect("compact head should still satisfy ordinary read caching");
        assert_eq!(cached.last_event_seq, compact.last_event_seq);
        assert_eq!(cached.projection_rev, compact.projection_rev);

        match hub
            .replay_session_stream(primary.workspace_id, primary.id, 3, 0, 50)
            .await
        {
            WorkspaceSessionReplay::Replay { items, last_sent } => {
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 7,
                        projection_rev: 7,
                    }
                );
                assert_eq!(
                    items.len(),
                    1,
                    "gap replay should not seed from compact-only cache"
                );
                match &items[0] {
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => {
                        assert_eq!(*session_id, primary.id);
                        assert_eq!(*after_seq, 3);
                        assert_eq!(reason.as_deref(), Some("missing_replay_events"));
                    }
                    other => panic!("expected leading gap, got {other:?}"),
                }
            }
            other => panic!("expected replay response, got {other:?}"),
        }
    }
}

mod worktree_vcs_tests {
    use super::super::*;
    use chrono::Utc;
    use ctx_core::ids::{SessionId, TaskId};
    use ctx_core::models::{
        SessionActivityState, SessionMetadata, SessionSnapshotSummary, SessionStatus,
        SessionTurnStatus, Task, TaskStatus, WorkspaceActiveTaskSummary, WorktreeVcsBaseResolution,
        WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsGitStatusSummary,
        WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFiles,
        WorktreeVcsTouchedFilesState,
    };

    fn sample_worktree_vcs_snapshot(worktree_id: WorktreeId) -> WorktreeVcsSnapshot {
        WorktreeVcsSnapshot {
            worktree_id,
            rev: 1,
            emitted_at_ms: 1,
            base_commit_sha: "base".to_string(),
            head_commit_sha: "head".to_string(),
            target_branch: Some("origin/main".to_string()),
            target_branch_commit_sha: Some("target".to_string()),
            base_resolution: WorktreeVcsBaseResolution::default(),
            compute_state: WorktreeVcsComputeState::Ready,
            summary: WorktreeVcsSummary::default(),
            git_status: WorktreeVcsGitStatusSummary {
                raw: "## main\n M edited.txt\n".to_string(),
                summary_line: "## main".to_string(),
                ..Default::default()
            },
            touched_files: WorktreeVcsTouchedFiles::default(),
            touched_files_state: WorktreeVcsTouchedFilesState::Ready,
            freshness: WorktreeVcsFreshness::Fresh,
            available: true,
            unavailable_reason: None,
            schema_version: 1,
        }
    }

    fn sample_task_summary(
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
    ) -> WorkspaceActiveTaskSummary {
        let now = Utc::now();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        WorkspaceActiveTaskSummary {
            task: Task {
                id: task_id,
                workspace_id,
                title: "task".to_string(),
                description: None,
                status: TaskStatus::Running,
                exec_plan_id: None,
                primary_session_id: Some(session_id),
                primary_worktree_id: Some(worktree_id),
                created_at: now,
                updated_at: now,
                archived_at: None,
                assistant_seen_at: None,
                last_activity_at: Some(now),
                last_assistant_message_at: None,
                has_active_session: true,
            },
            primary_session: SessionSnapshotSummary {
                session: SessionMetadata {
                    id: session_id,
                    task_id,
                    workspace_id,
                    worktree_id,
                    execution_environment: ctx_core::models::ExecutionEnvironment::Host,
                    parent_session_id: None,
                    relationship: None,
                    provider_id: "fake".to_string(),
                    model_id: "fake-model".to_string(),
                    reasoning_effort: None,
                    title: "session".to_string(),
                    agent_role: "assistant".to_string(),
                    status: SessionStatus::Active,
                    provider_session_ref: None,
                    created_at: now,
                    updated_at: now,
                },
                last_message_at: None,
                last_message_preview: None,
                last_event_seq: None,
                projection_rev: 0,
                state_rev: 0,
                activity: SessionActivityState {
                    is_working: false,
                    last_turn_status: Some(SessionTurnStatus::Completed),
                },
                unread: None,
            },
            primary_session_head: None,
            sessions: Vec::new(),
            sort_at: now,
        }
    }

    #[tokio::test]
    async fn publish_worktree_vcs_snapshot_clears_raw_git_status_text() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let workspace_id = WorkspaceId::new();
        let worktree_id = WorktreeId::new();
        let mut rx = hub.subscribe(workspace_id).await;

        hub.publish_worktree_vcs_snapshot(workspace_id, sample_worktree_vcs_snapshot(worktree_id))
            .await;

        let event = rx.recv().await.expect("worktree vcs event");
        match event {
            WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. } => {
                assert_eq!(snapshot.worktree_id, worktree_id);
                assert_eq!(snapshot.git_status.raw, "");
                assert_eq!(snapshot.git_status.summary_line, "## main");
            }
            other => panic!("expected worktree vcs snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn active_snapshot_orders_worktree_vcs_snapshots_by_worktree_id() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let workspace_id = WorkspaceId::new();
        let worktree_a = WorktreeId::new();
        let worktree_b = WorktreeId::new();
        let tasks = vec![
            sample_task_summary(workspace_id, worktree_a),
            sample_task_summary(workspace_id, worktree_b),
        ];

        hub.hydrate_snapshot(workspace_id, 1, 0, tasks, Vec::new())
            .await;
        hub.hydrate_worktree_vcs_snapshots(
            workspace_id,
            vec![
                sample_worktree_vcs_snapshot(worktree_b),
                sample_worktree_vcs_snapshot(worktree_a),
            ],
        )
        .await;

        let snapshot = hub.active_snapshot(workspace_id, i64::MAX).await;
        let actual = snapshot
            .worktree_vcs_snapshots
            .into_iter()
            .map(|snapshot| snapshot.worktree_id)
            .collect::<Vec<_>>();
        let mut expected = vec![worktree_a, worktree_b];
        expected.sort_by_key(|worktree_id| worktree_id.0);

        assert_eq!(actual, expected);
    }
}

mod replay_tests {
    use super::super::trim::{new_head_snapshot, session_metadata_from_session};
    use super::super::*;
    use chrono::{TimeZone, Utc};
    use ctx_core::ids::{SessionEventId, TurnId};
    use ctx_core::models::{SessionStatus, TaskStatus};
    use serde_json::json;

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
                projection_rev: 0,
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
                projection_rev: 0,
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
            reasoning_effort: None,
            title: "session".to_string(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn prunes_non_active_compact_heads_before_active_primary_heads() {
        let hub = WorkspaceActiveSnapshotHub::new_with_session_head_limit(3);
        let active = replay_session(SessionId::new());
        let active_head = new_head_snapshot(&active);
        hub.hydrate_snapshot(
            active.workspace_id,
            1,
            0,
            vec![replay_task(&active)],
            vec![active_head],
        )
        .await;

        let mut stale_session_ids = Vec::new();
        for _ in 0..3 {
            let stale_id = SessionId::new();
            stale_session_ids.push(stale_id);
            let mut stale = replay_session(stale_id);
            stale.workspace_id = active.workspace_id;
            hub.update_compact_session_head(new_head_snapshot(&stale))
                .await;
        }

        let stats = hub.stats().await;
        assert_eq!(stats.session_heads_count, 3);
        assert!(
            hub.get_cached_session_head_for_read(active.id)
                .await
                .is_some(),
            "active primary head should survive pruning",
        );
        let mut remaining_stale = 0usize;
        for session_id in &stale_session_ids {
            if hub
                .get_cached_session_head_for_read(*session_id)
                .await
                .is_some()
            {
                remaining_stale += 1;
            }
        }
        assert_eq!(remaining_stale, 2);
    }

    #[tokio::test]
    async fn deleted_primary_heads_become_prunable() {
        let hub = WorkspaceActiveSnapshotHub::new_with_session_head_limit(2);
        let active = replay_session(SessionId::new());
        let active_head = new_head_snapshot(&active);
        hub.hydrate_snapshot(
            active.workspace_id,
            1,
            0,
            vec![replay_task(&active)],
            vec![active_head],
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let mut stale_a = replay_session(SessionId::new());
        stale_a.workspace_id = active.workspace_id;
        hub.update_compact_session_head(new_head_snapshot(&stale_a))
            .await;

        hub.publish_active_task_delete(active.workspace_id, active.task_id)
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let mut stale_b = replay_session(SessionId::new());
        stale_b.workspace_id = active.workspace_id;
        hub.update_compact_session_head(new_head_snapshot(&stale_b))
            .await;

        let stats = hub.stats().await;
        assert_eq!(stats.session_heads_count, 2);
        assert!(
            hub.get_cached_session_head_for_read(active.id)
                .await
                .is_none(),
            "former primary head should be prunable after task removal",
        );
    }

    #[test]
    fn replay_records_delta_without_event() {
        let session_id = SessionId(uuid::Uuid::nil());
        let delta = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 5,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let delta_next = SessionHeadDelta {
            session_id,
            last_event_seq: 6,
            projection_rev: 6,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let mut state = SessionReplayState::default();
        state.record(&delta);
        state.record(&delta_next);

        match state.replay(
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 5,
            },
            10,
        ) {
            SessionReplayResult::Replay { deltas, last_sent } => {
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 6,
                        projection_rev: 6,
                    }
                );
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].last_event_seq, 6);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[test]
    fn replay_records_equal_seq_projection_rev_advance() {
        let session_id = SessionId(uuid::Uuid::nil());
        let delta = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 5,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let delta_next = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 6,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let mut state = SessionReplayState::default();
        state.record(&delta);
        state.record(&delta_next);

        match state.replay(
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 5,
            },
            10,
        ) {
            SessionReplayResult::Replay { deltas, last_sent } => {
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].projection_rev, 6);
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 5,
                        projection_rev: 6,
                    }
                );
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[test]
    fn replay_ignores_transient_negative_seq_deltas() {
        let session_id = SessionId(uuid::Uuid::nil());
        let durable = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 5,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let transient = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 5,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: Some(SessionEvent {
                seq: -1,
                id: SessionEventId::new(),
                session_id,
                run_id: None,
                turn_id: Some(TurnId::new()),
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({ "content_fragment": "partial" }),
                transient: true,
                created_at: Utc.timestamp_opt(0, 0).unwrap(),
            }),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let durable_next = SessionHeadDelta {
            session_id,
            last_event_seq: 6,
            projection_rev: 6,
            state_rev: 0,
            emitted_at_ms: None,
            session: None,
            activity: None,
            event: None,
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        };
        let mut state = SessionReplayState::default();
        state.record(&durable);
        state.record(&transient);
        state.record(&durable_next);

        match state.replay(
            SessionReplayCursor {
                last_event_seq: 5,
                projection_rev: 5,
            },
            10,
        ) {
            SessionReplayResult::Replay { deltas, last_sent } => {
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].last_event_seq, 6);
                assert!(deltas[0].event.is_none());
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 6,
                        projection_rev: 6,
                    }
                );
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
        head.projection_rev = 9;
        hub.hydrate_snapshot(
            session.workspace_id,
            1,
            0,
            vec![replay_task(&session)],
            vec![head.clone()],
        )
        .await;

        match hub
            .replay_session_stream(session.workspace_id, session.id, 3, 3, 50)
            .await
        {
            WorkspaceSessionReplay::Replay { items, last_sent } => {
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 7,
                        projection_rev: 9,
                    }
                );
                assert_eq!(items.len(), 1);
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
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn truncated_compact_session_head_cache_misses_request_path() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let primary = replay_session(SessionId::new());
        let mut compact = new_head_snapshot(&primary);
        compact.last_event_seq = 7;
        compact.projection_rev = 7;
        compact.head_window.truncated = true;

        hub.update_compact_session_head(compact).await;

        assert!(
            hub.get_cached_session_head_for_request(primary.id, false, 60)
                .await
                .is_none(),
            "truncated compact heads must not satisfy the stronger request path"
        );
    }

    #[tokio::test]
    async fn hydrate_snapshot_seeds_compact_read_cache_without_replay_capability() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let session = replay_session(SessionId::new());
        let mut head = new_head_snapshot(&session);
        head.last_event_seq = 11;
        head.projection_rev = 17;

        hub.hydrate_snapshot(
            session.workspace_id,
            1,
            0,
            vec![replay_task(&session)],
            vec![head.clone()],
        )
        .await;

        assert!(
            hub.get_session_head(session.id).await.is_none(),
            "hydrated compact heads must not be treated as replay-capable seeds"
        );
        let cached = hub
            .get_cached_session_head_for_read(session.id)
            .await
            .expect("hydrated compact head should be readable from cache");
        assert_eq!(cached.last_event_seq, head.last_event_seq);
        assert_eq!(cached.projection_rev, head.projection_rev);
    }

    #[tokio::test]
    async fn replay_session_stream_seeds_head_for_non_resuming_cursor_gap() {
        let hub = WorkspaceActiveSnapshotHub::new();
        let session = replay_session(SessionId::new());
        let mut head = new_head_snapshot(&session);
        head.last_event_seq = 11;
        head.projection_rev = 17;
        hub.hydrate_snapshot(
            session.workspace_id,
            1,
            0,
            vec![replay_task(&session)],
            vec![head],
        )
        .await;

        match hub
            .replay_session_stream(session.workspace_id, session.id, 0, 0, 50)
            .await
        {
            WorkspaceSessionReplay::Replay { items, last_sent } => {
                assert_eq!(
                    last_sent,
                    SessionReplayCursor {
                        last_event_seq: 11,
                        projection_rev: 17,
                    }
                );
                assert!(
                    items.is_empty(),
                    "compact hydration alone should not seed replay"
                );
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }
}
