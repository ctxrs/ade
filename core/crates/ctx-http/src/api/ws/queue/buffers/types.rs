use super::*;

pub(crate) enum NextWorkspaceStreamItem {
    Control(StreamQueueEntry<WorkspaceActiveSnapshotStreamMessage>),
    HeadsBatch {
        snapshot_rev: i64,
        deltas: Vec<SessionHeadDelta>,
    },
    SummaryBatch {
        events: Vec<WorkspaceActiveSnapshotEvent>,
    },
}

#[derive(Debug)]
pub(crate) enum HeadBatchPushError {
    SessionLimit { session_id: SessionId, limit: usize },
    TotalLimit { limit: usize },
}

#[derive(Debug)]
pub(crate) enum SummaryBatchPushError {
    TotalLimit { limit: usize },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SummaryBatchPushOutcome {
    Enqueued,
    Replaced,
}
