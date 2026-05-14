use ctx_core::ids::RunId;
use ctx_core::models::Message;

pub(in crate::daemon::sessions::subagents) struct PersistedSubagentPrompt {
    pub(in crate::daemon::sessions::subagents) run_id: RunId,
    pub(in crate::daemon::sessions::subagents) saved_message: Message,
    pub(in crate::daemon::sessions::subagents) last_event_seq: i64,
}
