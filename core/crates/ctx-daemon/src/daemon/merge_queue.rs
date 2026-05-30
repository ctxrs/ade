use std::sync::Arc;

mod host;
mod route_contract;
mod submit_route;

pub(in crate::daemon) use host::MergeQueueRouteHost;

pub(in crate::daemon) fn spawn_merge_queue_runner(host: Arc<MergeQueueRouteHost>) {
    ctx_merge_queue::spawn_merge_queue_runner::<MergeQueueRouteHost>(host);
}

#[cfg(test)]
mod tests;
