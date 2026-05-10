use std::sync::Arc;

use anyhow::Context;
use ctx_core::models::{Session, SessionEventType};
use ctx_session_service::title_generation;

use super::TitleGenerationOutcome;
use crate::daemon::AppState;

pub(crate) async fn apply_session_title_update(
    state: &Arc<AppState>,
    session: &Session,
    outcome: TitleGenerationOutcome,
) -> anyhow::Result<()> {
    let store = state.store_for_session(session.id).await?;
    let updated = store
        .update_session_title(session.id, outcome.title.clone())
        .await
        .context("updating session title")?;
    if !updated {
        return Ok(());
    }

    if let Ok(Some(updated_session)) = store.get_session(session.id).await {
        state.sessions.remember_session_meta(&updated_session).await;
    }

    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    let mut task_updated = false;
    if let Ok(Some(task)) = store.get_task(session.task_id).await {
        let title = task.title.trim();
        if (title.is_empty() || title == title_generation::DEFAULT_SESSION_TITLE)
            && store
                .update_task_title(session.task_id, outcome.title.clone())
                .await
                .unwrap_or(false)
        {
            task_updated = true;
        }
    }

    if task_updated {
        if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
            tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
        }
    }

    let notice = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "title_generated",
                "title": outcome.title,
                "source": outcome.source.as_str(),
            }),
        )
        .await;
    if let Ok(event) = notice {
        state.publish_event(event).await;
    }

    Ok(())
}
