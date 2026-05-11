use ctx_core::models::{
    SessionHeadDelta, WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage,
};

use super::super::super::replay::with_stream_rev;
use super::super::bump_latest_snapshot_rev;
use super::runtime::WorkspaceStreamSendRuntime;

pub(in crate::api::ws) struct SequencedControlMessage {
    pub(in crate::api::ws) message: WorkspaceActiveSnapshotStreamMessage,
    pub(in crate::api::ws) is_snapshot: bool,
}

#[derive(Default)]
pub(in crate::api::ws) struct WorkspaceStreamSequencer {
    stream_seq: i64,
}

impl WorkspaceStreamSequencer {
    pub(in crate::api::ws) fn sequence_control_message(
        &mut self,
        message: WorkspaceActiveSnapshotStreamMessage,
    ) -> SequencedControlMessage {
        let is_snapshot = matches!(
            message,
            WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
        );
        let message = match message {
            WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => message,
            other => {
                self.stream_seq += 1;
                with_stream_rev(other, self.stream_seq)
            }
        };
        SequencedControlMessage {
            message,
            is_snapshot,
        }
    }

    pub(in crate::api::ws) fn sequence_heads_batch(
        &mut self,
        runtime: &WorkspaceStreamSendRuntime,
        snapshot_rev: i64,
        deltas: Vec<SessionHeadDelta>,
    ) -> WorkspaceActiveSnapshotStreamMessage {
        let latest_rev = runtime
            .latest_snapshot_rev()
            .load(std::sync::atomic::Ordering::Relaxed);
        let snapshot_rev = snapshot_rev.max(latest_rev);
        bump_latest_snapshot_rev(runtime.latest_snapshot_rev(), snapshot_rev);
        self.stream_seq += 1;
        WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            rev: self.stream_seq,
            snapshot_rev,
            deltas,
        }
    }

    pub(in crate::api::ws) fn sequence_summary_event(
        &mut self,
        event: WorkspaceActiveSnapshotEvent,
    ) -> WorkspaceActiveSnapshotStreamMessage {
        self.stream_seq += 1;
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: self.stream_seq,
            event: Box::new(event),
        }
    }
}
