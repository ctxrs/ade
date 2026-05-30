use std::time::Duration;

use super::DaemonShutdownHost;

async fn trigger_daemon_shutdown(host: DaemonShutdownHost, reason: &str) {
    tracing::info!("daemon shutdown requested: {reason}");
    let _ = host.acquire_shutdown_drain(reason).await;
    if let Err(err) = host.reconcile_running_turns_with_reason(reason).await {
        tracing::warn!("failed to reconcile running turns during daemon shutdown: {err:#}");
    }
    host.shutdown_provider_adapters(reason).await;
    match host.save_or_stop_selected_shared_substrate().await {
        Ok(Some(record)) => {
            tracing::info!(
                shutdown_reason = reason,
                substrate = ?record.substrate,
                shutdown_outcome = ?record.shutdown_outcome,
                shutdown_detail = ?record.shutdown_reason,
                save_error_present = record.save_error_present,
                saved_state_written_on_shutdown = record.saved_state_written_on_shutdown,
                simulated = record.simulated,
                "shared substrate save-or-stop requested for daemon shutdown"
            );
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                "failed to save-or-stop shared substrate during daemon shutdown: {err:#}"
            );
        }
    }
    host.broadcast_shutdown();
}

pub(in crate::daemon) fn spawn_deferred_daemon_shutdown(
    host: DaemonShutdownHost,
    reason: String,
    delay: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        trigger_daemon_shutdown(host, &reason).await;
    });
}

pub(in crate::daemon) fn spawn_process_shutdown_listener(host: DaemonShutdownHost) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(signal) => signal,
                    Err(err) => {
                        tracing::warn!("failed to register SIGTERM handler: {err:#}");
                        return;
                    }
                };
            let ctrl_c_host = host.clone();
            let sigterm_host = host.clone();
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if let Err(err) = result {
                        tracing::warn!("failed to listen for ctrl_c: {err:#}");
                        return;
                    }
                    trigger_daemon_shutdown(ctrl_c_host, "ctrl_c").await;
                }
                _ = sigterm.recv() => {
                    trigger_daemon_shutdown(sigterm_host, "sigterm").await;
                }
            }
        }

        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                tracing::warn!("failed to listen for ctrl_c: {err:#}");
                return;
            }
            trigger_daemon_shutdown(host, "ctrl_c").await;
        }
    });
}
