use std::sync::Arc;

use ctx_merge_queue::MergeQueueRuntime;
use ctx_observability::ops_events::OpsEvents;
use ctx_store::{Store, StoreManager};

#[cfg(any(test, feature = "test-support"))]
use crate::daemon::DaemonState;
use crate::daemon::{
    merge_queue::MergeQueueRouteHost,
    merge_queue_route_handles::{
        MergeQueueNoticePublicationEffect, MergeQueueNoticePublicationFuture,
        MergeQueueNoticeSessionEvent,
    },
    task_session_effects::SessionPublicationEffects,
    ProtectedWorkspaceStoreLookup, SessionStoreLookup,
};

pub(in crate::daemon) struct MergeQueueRouteHostParts {
    pub(in crate::daemon) stores: StoreManager,
    pub(in crate::daemon) global_store: Store,
    pub(in crate::daemon) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(in crate::daemon) session_stores: SessionStoreLookup,
    pub(in crate::daemon) merge_queue: Arc<MergeQueueRuntime>,
    pub(in crate::daemon) ops_events: OpsEvents,
    pub(in crate::daemon) session_publication: SessionPublicationEffects,
}

pub(in crate::daemon) fn merge_queue_route_host_from_parts(
    parts: MergeQueueRouteHostParts,
) -> Arc<MergeQueueRouteHost> {
    let publisher = parts.session_publication;
    let publish_merge_queue_notice: MergeQueueNoticePublicationEffect =
        Arc::new(move |notice_event: MergeQueueNoticeSessionEvent| {
            let publisher = publisher.clone();
            Box::pin(async move { publisher.publish_merge_queue_notice(notice_event).await })
                as MergeQueueNoticePublicationFuture
        });
    Arc::new(MergeQueueRouteHost::new(
        parts.stores,
        parts.global_store,
        parts.workspace_stores,
        parts.session_stores,
        parts.merge_queue,
        parts.ops_events,
        publish_merge_queue_notice,
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub(in crate::daemon) fn merge_queue_route_host_from_state(
    state: &DaemonState,
) -> Arc<MergeQueueRouteHost> {
    let workspace_stores = ProtectedWorkspaceStoreLookup::new(
        state.core.stores.clone(),
        Arc::clone(&state.sessions),
        Arc::clone(&state.transport.merge_queue),
    );
    let session_stores =
        SessionStoreLookup::new(state.global_store().clone(), workspace_stores.clone());
    merge_queue_route_host_from_parts(MergeQueueRouteHostParts {
        stores: state.core.stores.clone(),
        global_store: state.global_store().clone(),
        workspace_stores,
        session_stores,
        merge_queue: Arc::clone(&state.transport.merge_queue),
        ops_events: state.telemetry.ops_events.clone(),
        session_publication: state.session_publication.clone(),
    })
}
