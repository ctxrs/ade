use super::*;

#[path = "serve/background.rs"]
mod background;
#[cfg(test)]
pub(in crate::daemon) use background::spawn_startup_provider_status_refresh;

pub async fn serve(bind: Vec<String>, data_dir: Option<String>) -> Result<()> {
    let data_root = match data_dir {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("resolving home dir")?;
            base.home_dir().join(".ctx")
        }
    };
    ctx_fs::permissions::ensure_private_dir(&data_root).await?;
    // Canonicalize so legacy machine mount sources resolve under shared roots on macOS
    // (e.g. /tmp -> /private/tmp). This also reduces accidental duplicate state roots.
    let data_root = tokio::fs::canonicalize(&data_root)
        .await
        .unwrap_or(data_root);
    ctx_fs::permissions::ensure_private_dir(&data_root).await?;
    ctx_fs::permissions::ensure_private_dir(&data_root.join("logs"))
        .await
        .ok();

    let _daemon_lock = auth::acquire_daemon_lock(&data_root)?;

    let global_db_path = data_root.join("db").join("db.sqlite");
    let bootstrap_store = Store::open_sqlite(&global_db_path, None).await?;
    let settings_data = ctx_settings_service::load_settings(&bootstrap_store).await?;
    bootstrap_store.close().await;
    let store_config = settings_data
        .storage
        .as_ref()
        .map(|storage| StoreManagerConfig {
            max_connections: storage.max_connections,
            workspace_max_connections: storage.max_connections,
            ..StoreManagerConfig::default()
        })
        .unwrap_or_default();
    let stores = StoreManager::open_with_config(&data_root, store_config).await?;

    retention::spawn_archived_session_data_pruner(stores.clone());

    let agent_cfg = installer::load_managed_agent_server_config_or_err(&data_root).await?;

    let providers = ctx_provider_runtime::provider_adapters::build_startup_provider_adapters(
        &data_root, &agent_cfg,
    );

    let bound = listener::bind_daemon_listeners(bind).await?;
    let daemon_url = bound.daemon_url.clone();
    let requested_binds = bound.requested_binds.clone();
    let listeners = bound.listeners;
    let public_base_url = listener::daemon_public_base_url_from_env()?;

    let mut auth = auth::load_or_init_daemon_auth(&data_root)?;
    let auth_token = Some(auth.token.clone());

    auth.daemon_url = Some(daemon_url.clone());
    auth::write_daemon_auth_file(&auth::daemon_auth_path(&data_root), &auth)?;

    let state = Arc::new(AppState::new_with_public_base_url(
        data_root,
        stores,
        providers,
        daemon_url.clone(),
        public_base_url,
        auth_token,
    ));
    state.transport.web_sessions.clone().start_reaper().await;
    state.transport.terminals.clone().start_reaper().await;
    lifecycle::spawn_cache_sweeper(state.clone());
    lifecycle::spawn_provider_worker_sweeper(state.clone());
    lifecycle::spawn_endpoint_model_catalog_sweeper(state.clone());
    if let Err(err) = reconcile_running_turns(&state).await {
        tracing::warn!(err = %err, "failed to reconcile running turns on startup");
    }
    let settings = ctx_settings_service::load_settings(state.global_store()).await?;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = settings.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = settings
        .telemetry
        .as_ref()
        .map(|t| t.enabled)
        .unwrap_or(true);
    state
        .telemetry
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }

    if let Err(err) = tool_cgroup::apply_settings(&state, &settings).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }

    background::spawn_daemon_background_services(state.clone(), requested_binds.clone());
    let app: Router = api::router(state.clone());

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
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
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
