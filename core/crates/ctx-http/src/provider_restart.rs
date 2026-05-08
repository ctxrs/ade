use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use sysinfo::{Pid, Signal, System};
use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::MessageId;
use ctx_core::models::{Message, MessageDelivery, MessageRole, SessionEventType};
use ctx_providers::adapters::ProviderRestartMode;

use crate::daemon::AppState;
use ctx_settings_model::{ProviderRestartSettings, ResourceGovernanceMode, Settings};

pub use ctx_provider_runtime::provider_restart::{
    compute_effective_limits, ProviderRestartConfig, ProviderRestartEvent, ProviderRestartLimits,
    ProviderRestartRuntime, ResourceGovernanceMode as RestartGovernanceMode,
};

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_restart.clone().unwrap_or_default();
    ctx_provider_runtime::provider_restart::apply_settings(state, &map_config(&cfg)).await
}

pub fn spawn_provider_restart(state: Arc<AppState>) {
    ctx_provider_runtime::provider_restart::spawn_provider_restart(state);
}

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_restart::ProviderRestartHost for AppState {
    fn provider_restart_runtime(&self) -> &Mutex<ProviderRestartRuntime> {
        &self.providers.restart
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.core.shutdown_tx.subscribe()
    }

    async fn system_snapshot(&self) -> ctx_provider_runtime::provider_guard::SystemSnapshot {
        let (system, _disks, _cache_age_ms) = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.system_snapshot()
        };
        ctx_provider_runtime::provider_guard::SystemSnapshot {
            memory_total_bytes: system.memory_total_bytes,
            memory_used_bytes: system.memory_used_bytes,
        }
    }

    async fn provider_memory_snapshot(
        &self,
    ) -> Vec<ctx_provider_runtime::provider_guard::ProviderMemorySample> {
        let provider_processes = list_provider_processes(self).await;
        let samples = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.provider_memory_snapshot(&provider_processes)
        };
        samples
            .into_iter()
            .map(
                |sample| ctx_provider_runtime::provider_guard::ProviderMemorySample {
                    provider_id: sample.provider_id,
                    label: sample.label,
                    pid: sample.pid,
                    memory_bytes: sample.memory_bytes,
                    tool_memory_bytes: sample.tool_memory_bytes,
                },
            )
            .collect()
    }

    async fn restart_provider(&self, provider_id: &str, pid: u32) {
        let adapter = {
            let providers = self.providers.adapters.lock().await;
            providers.get(provider_id).cloned()
        };
        let mut needs_kill = true;
        if let Some(adapter) = adapter {
            match adapter
                .restart(
                    "provider restart: sustained memory usage",
                    ProviderRestartMode::Immediate,
                )
                .await
            {
                Ok(()) => needs_kill = false,
                Err(err) => {
                    tracing::warn!(provider_id, pid, "provider restart hook failed: {err:#}")
                }
            }
        } else {
            tracing::warn!(
                provider_id,
                pid,
                "provider restart failed: provider adapter missing"
            );
        }

        if needs_kill {
            let killed = signal_pids(&[pid], Signal::Kill);
            if killed == 0 {
                tracing::warn!(provider_id, pid, "provider restart failed to kill process");
            }
        }
    }

    async fn on_provider_restart_notice(state: &Arc<Self>, event: ProviderRestartEvent) {
        notify_sessions(state, &event).await;
    }
}

fn map_config(settings: &ProviderRestartSettings) -> ProviderRestartConfig {
    ProviderRestartConfig {
        enabled: settings.enabled,
        mode: Some(match settings.mode {
            ResourceGovernanceMode::Auto => RestartGovernanceMode::Auto,
            ResourceGovernanceMode::Custom => RestartGovernanceMode::Custom,
        }),
        memory_high_mb: settings.memory_high_mb,
        memory_max_mb: settings.memory_max_mb,
        interval_ms: settings.interval_ms,
        grace_period_ms: settings.grace_period_ms,
    }
}

async fn notify_sessions(
    state: &Arc<AppState>,
    event: &ctx_provider_runtime::provider_restart::ProviderRestartEvent,
) {
    let message_text = match event.kind {
        "provider_restart_warning" => {
            "Provider memory is high; restart scheduled if it stays elevated."
        }
        "provider_restart" => "Provider restart requested after sustained high memory usage.",
        _ => "Provider restart notice.",
    };
    let session_ids = state.sessions.list_running_sessions().await;
    for session_id in session_ids {
        let store = match state.store_for_session(session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = store.get_session(session_id).await.ok().flatten();
        let Some(session) = session else {
            continue;
        };
        if session.provider_id != event.sample.provider_id {
            continue;
        }

        let message_id = insert_system_message(state, &store, &session, message_text).await;
        let payload = json!({
            "provider": event.sample.provider_id,
            "kind": event.kind,
            "stage": event.stage,
            "pid": event.sample.pid,
            "memory_mb": bytes_to_mb(event.sample.memory_bytes),
            "tool_memory_mb": bytes_to_mb(event.sample.tool_memory_bytes),
            "system_total_mb": bytes_to_mb(event.system.memory_total_bytes),
            "system_used_mb": bytes_to_mb(event.system.memory_used_bytes),
            "limit_high_mb": event.limits.memory_high_mb,
            "limit_max_mb": event.limits.memory_max_mb,
            "grace_period_ms": event.limits.grace_period.as_millis() as u64,
            "restart_at_ms": event.restart_at_ms,
            "message": message_text,
            "message_id": message_id.map(|id| id.0),
        });
        match store
            .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
            .await
        {
            Ok(event) => state.publish_event(event).await,
            Err(err) => tracing::warn!(
                provider_id = %event.sample.provider_id,
                session_id = %session_id.0,
                "provider restart failed to append session event: {err:#}"
            ),
        }
    }
}

async fn insert_system_message(
    state: &AppState,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
    content: &str,
) -> Option<MessageId> {
    let now = Utc::now();
    let message_id = MessageId::new();
    let order_seq_state = state.sessions.get_order_seq_state(store, session.id).await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
    };
    let msg = Message {
        id: message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: None,
        turn_id: None,
        turn_sequence: None,
        order_seq: Some(order_seq),
        role: MessageRole::System,
        content: content.to_string(),
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: Some(now),
        created_at: now,
    };
    match store.insert_message(msg).await {
        Ok(saved) => Some(saved.id),
        Err(err) => {
            tracing::warn!(
                session_id = %session.id.0,
                "provider restart failed to insert system message: {err:#}"
            );
            None
        }
    }
}

fn signal_pids(pids: &[u32], signal: Signal) -> usize {
    let mut system = System::new();
    system.refresh_processes();
    let mut killed = 0usize;
    for pid in pids {
        if let Some(process) = system.process(Pid::from_u32(*pid)) {
            if process.kill_with(signal).unwrap_or(false) {
                killed += 1;
            }
        }
    }
    killed
}

async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
    let providers = {
        let providers = state.providers.adapters.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut processes = Vec::new();
    for adapter in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}

fn bytes_to_mb(value: u64) -> u64 {
    value / (1024 * 1024)
}
