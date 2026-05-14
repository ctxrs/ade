use std::sync::Arc;

use crate::daemon::DaemonState;
use ctx_core::models::Session;

mod backlog;
mod dispatch;
mod persistence;
mod types;

pub(in crate::daemon::sessions::subagents) use dispatch::dispatch_subagent_prompt;
pub(in crate::daemon::sessions::subagents) use persistence::persist_subagent_prompt;
pub(in crate::daemon::sessions::subagents) use types::PersistedSubagentPrompt;

use super::super::errors::ApiResult;

pub(in crate::daemon::sessions::subagents) async fn enqueue_subagent_prompt(
    state: &Arc<DaemonState>,
    session: &Session,
    prompt: String,
) -> ApiResult<PersistedSubagentPrompt> {
    let persisted = persist_subagent_prompt(state, session, prompt).await?;
    dispatch_subagent_prompt(state, session, &persisted.saved_message).await;
    Ok(persisted)
}
