use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::SessionId;
use ctx_core::models::SessionEventType;
use ctx_providers::ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome};

use crate::daemon::{AppState, StoreLookup};

#[derive(Debug)]
pub(crate) struct SubmitAskUserAnswer {
    pub(crate) tool_call_id: String,
    pub(crate) outcome: AskUserQuestionOutcome,
    pub(crate) answers: HashMap<String, String>,
}

#[derive(Debug)]
pub(crate) enum SubmitAskUserAnswerError {
    MissingToolCallId,
    SessionNotFound,
    StoreUnavailable(anyhow::Error),
    LoadSession,
    NoPendingQuestion,
}

pub(crate) async fn submit_ask_user_answer(
    state: &Arc<AppState>,
    session_id: SessionId,
    submission: SubmitAskUserAnswer,
) -> Result<(), SubmitAskUserAnswerError> {
    let store = store_for_ask_user_session(state, session_id).await?;
    if !store
        .get_session(session_id)
        .await
        .map_err(|_| SubmitAskUserAnswerError::LoadSession)?
        .is_some()
    {
        return Err(SubmitAskUserAnswerError::SessionNotFound);
    }

    let tool_call_id = submission.tool_call_id.trim().to_string();
    if tool_call_id.is_empty() {
        return Err(SubmitAskUserAnswerError::MissingToolCallId);
    }

    let answers_for_event = submission.answers.clone();
    let ok = state
        .core
        .ask_user_question
        .submit(
            &session_id.0.to_string(),
            &tool_call_id,
            AskUserQuestionAnswer {
                outcome: submission.outcome,
                answers: submission.answers,
            },
        )
        .await;

    if !ok {
        return Err(SubmitAskUserAnswerError::NoPendingQuestion);
    }

    if let Ok(event) = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "ask_user_question_answered",
                "tool_call_id": tool_call_id,
                "outcome": submission.outcome.as_str(),
                "answers": answers_for_event,
            }),
        )
        .await
    {
        state.publish_event(event).await;
    }

    Ok(())
}

async fn store_for_ask_user_session(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, SubmitAskUserAnswerError> {
    let store = match state.lookup_session_store(session_id).await {
        StoreLookup::Found(store) => store,
        StoreLookup::Missing | StoreLookup::Deleting => {
            return Err(SubmitAskUserAnswerError::SessionNotFound);
        }
        StoreLookup::Unavailable(err) => {
            return Err(SubmitAskUserAnswerError::StoreUnavailable(err));
        }
    };
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(SubmitAskUserAnswerError::StoreUnavailable)?
    {
        return Err(SubmitAskUserAnswerError::SessionNotFound);
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::models::ExecutionEnvironment;
    use ctx_store::StoreManager;

    async fn test_state(root: &std::path::Path) -> Arc<AppState> {
        Arc::new(AppState::new(
            root.to_path_buf(),
            StoreManager::open(root).await.unwrap(),
            HashMap::new(),
            "http://127.0.0.1:4399".to_string(),
            Some("daemon-secret".to_string()),
        ))
    }

    async fn create_session(state: &Arc<AppState>, root: &std::path::Path) -> SessionId {
        let workspace = state
            .global_store()
            .create_workspace(
                "ask-user".to_string(),
                root.join("workspace").to_string_lossy().to_string(),
                ctx_core::models::VcsKind::Git,
            )
            .await
            .unwrap();
        let store = state.store_for_workspace(workspace.id).await.unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                root.join("worktree").to_string_lossy().to_string(),
                "deadbeef".to_string(),
                None,
            )
            .await
            .unwrap();
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
            .unwrap();
        let task = store
            .create_task(workspace.id, "task".to_string(), None)
            .await
            .unwrap();
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
            .unwrap();
        state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await
            .unwrap();
        session.id
    }

    #[tokio::test]
    async fn submit_ask_user_answer_fulfills_pending_question_and_records_notice() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path()).await;
        let session_id = create_session(&state, root.path()).await;
        let receiver = state
            .core
            .ask_user_question
            .begin(session_id.0.to_string(), "tool-1".to_string())
            .await;

        let mut answers = HashMap::new();
        answers.insert("choice".to_string(), "ship".to_string());
        submit_ask_user_answer(
            &state,
            session_id,
            SubmitAskUserAnswer {
                tool_call_id: " tool-1 ".to_string(),
                outcome: AskUserQuestionOutcome::Submitted,
                answers: answers.clone(),
            },
        )
        .await
        .unwrap();

        let answer = receiver.await.unwrap();
        assert_eq!(answer.outcome, AskUserQuestionOutcome::Submitted);
        assert_eq!(answer.answers, answers);

        let store = state.store_for_session(session_id).await.unwrap();
        let events = store.list_session_events(session_id).await.unwrap();
        assert!(events.iter().any(|event| {
            event.payload_json.get("kind").and_then(|v| v.as_str())
                == Some("ask_user_question_answered")
                && event
                    .payload_json
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    == Some("tool-1")
        }));
    }

    #[tokio::test]
    async fn submit_ask_user_answer_rejects_missing_tool_call_id() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path()).await;
        let session_id = create_session(&state, root.path()).await;

        let error = submit_ask_user_answer(
            &state,
            session_id,
            SubmitAskUserAnswer {
                tool_call_id: "   ".to_string(),
                outcome: AskUserQuestionOutcome::Submitted,
                answers: HashMap::new(),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, SubmitAskUserAnswerError::MissingToolCallId));
    }

    #[tokio::test]
    async fn submit_ask_user_answer_rejects_without_pending_question() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path()).await;
        let session_id = create_session(&state, root.path()).await;

        let error = submit_ask_user_answer(
            &state,
            session_id,
            SubmitAskUserAnswer {
                tool_call_id: "tool-1".to_string(),
                outcome: AskUserQuestionOutcome::Submitted,
                answers: HashMap::new(),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, SubmitAskUserAnswerError::NoPendingQuestion));
    }

    #[tokio::test]
    async fn submit_ask_user_answer_rejects_missing_session() {
        let root = tempfile::tempdir().unwrap();
        let state = test_state(root.path()).await;

        let error = submit_ask_user_answer(
            &state,
            SessionId::new(),
            SubmitAskUserAnswer {
                tool_call_id: "tool-1".to_string(),
                outcome: AskUserQuestionOutcome::Submitted,
                answers: HashMap::new(),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, SubmitAskUserAnswerError::SessionNotFound));
    }
}
