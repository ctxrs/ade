use anyhow::{Context, Result};
use ctx_daemon::daemon;
use serde_json::json;

use crate::api;

pub async fn serve(bind: Vec<String>, data_dir: Option<String>) -> Result<()> {
    let runtime = daemon::bootstrap_daemon_runtime(bind, data_dir).await?;
    serve_runtime(runtime).await
}

async fn serve_runtime(runtime: daemon::DaemonRuntime) -> Result<()> {
    let daemon::DaemonRuntime {
        _daemon_lock,
        handle,
        listeners,
        daemon_url,
    } = runtime;
    let app = api::router(handle.clone());
    let bound_addrs = listeners
        .iter()
        .filter_map(|listener| listener.local_addr().ok())
        .map(|addr| addr.to_string())
        .collect::<Vec<_>>();
    tracing::info!("ctx daemon listening on {daemon_url} (binds={bound_addrs:?})");
    println!("{}", json!({"event":"listening","url": daemon_url}));
    let mut servers = tokio::task::JoinSet::new();
    for listener in listeners {
        let app = app.clone();
        let mut shutdown_rx = handle.core().subscribe_shutdown();
        servers.spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.recv().await;
                })
                .await
        });
    }
    while let Some(result) = servers.join_next().await {
        result.context("daemon listener task panicked")??;
    }
    Ok(())
}
