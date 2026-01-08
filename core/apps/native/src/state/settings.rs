use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::Context;
use gpui_tokio::Tokio;

use ctx_core::ids::WorkspaceId;
use ctx_providers::adapters::ProviderStatus;

use crate::theme::ThemeColors;

use super::workspace::WorkspaceItem;

use ctx_client::{
    Client, DictationProvider, InstallInfo, InstallProgressEvent, InstallStateKind,
    ProviderOptions, PublicSettings, ResourceGovernanceMode, ResourceGovernanceStatusState,
};

#[derive(Clone)]
pub(crate) struct InstallSession {
    pub(crate) install_id: String,
    pub(crate) state: InstallStateKind,
    pub(crate) pct: Option<u8>,
    pub(crate) last_stage: Option<String>,
    pub(crate) last_message: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SettingsSummaryRow {
    pub(crate) label: String,
    pub(crate) value: String,
}

pub(crate) struct SettingsState {
    pub(crate) colors: ThemeColors,
    pub(crate) workspaces: Vec<WorkspaceItem>,
    pub(crate) selected_workspace: Option<WorkspaceId>,
    pub(crate) providers: Vec<ProviderStatus>,
    pub(crate) provider_options: HashMap<String, ProviderOptions>,
    pub(crate) installs: HashMap<String, InstallSession>,
    pub(crate) install_polling: HashSet<String>,
    pub(crate) settings: Option<PublicSettings>,
    pub(crate) workspaces_loading: bool,
    pub(crate) providers_loading: bool,
    pub(crate) settings_loading: bool,
    pub(crate) workspace_error: Option<String>,
    pub(crate) provider_error: Option<String>,
    pub(crate) settings_error: Option<String>,
    pub(crate) install_busy: Option<String>,
    pub(crate) auth_busy: HashMap<String, bool>,
    pub(crate) verify_busy: HashMap<String, bool>,
    pub(crate) opts_busy: HashMap<String, bool>,
}

impl SettingsState {
    pub(crate) fn new(colors: ThemeColors) -> Self {
        Self {
            colors,
            workspaces: Vec::new(),
            selected_workspace: None,
            providers: Vec::new(),
            provider_options: HashMap::new(),
            installs: HashMap::new(),
            install_polling: HashSet::new(),
            settings: None,
            workspaces_loading: false,
            providers_loading: false,
            settings_loading: false,
            workspace_error: None,
            provider_error: None,
            settings_error: None,
            install_busy: None,
            auth_busy: HashMap::new(),
            verify_busy: HashMap::new(),
            opts_busy: HashMap::new(),
        }
    }

    pub(crate) fn start_load(&mut self, cx: &mut Context<Self>) {
        self.refresh_workspaces(cx);
        self.refresh_providers(cx);
        self.refresh_settings(cx);
    }

    pub(crate) fn refresh_workspaces(&mut self, cx: &mut Context<Self>) {
        self.workspaces_loading = true;
        self.workspace_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let workspaces = client.list_workspaces().await?;
            Ok(workspaces)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.workspaces_loading = false;
                let previous_selection = view.selected_workspace;
                match result {
                    Ok(workspaces) => {
                        view.workspaces = workspaces
                            .into_iter()
                            .map(|workspace| WorkspaceItem {
                                id: workspace.id,
                                name: workspace.name,
                            })
                            .collect();
                        if let Some(selected) = view.selected_workspace {
                            if !view.workspaces.iter().any(|ws| ws.id == selected) {
                                view.selected_workspace = None;
                            }
                        }
                        if view.selected_workspace.is_none() {
                            view.selected_workspace =
                                view.workspaces.first().map(|workspace| workspace.id);
                        }
                        if view.selected_workspace != previous_selection {
                            view.provider_options.clear();
                            view.opts_busy.clear();
                            view.auth_busy.clear();
                            view.verify_busy.clear();
                        }
                        view.maybe_probe_providers(cx);
                    }
                    Err(err) => {
                        view.workspaces.clear();
                        view.selected_workspace = None;
                        view.workspace_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn refresh_providers(&mut self, cx: &mut Context<Self>) {
        self.providers_loading = true;
        self.provider_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let providers = client.list_providers().await?;
            Ok(providers)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.providers_loading = false;
                match result {
                    Ok(providers) => {
                        view.providers = providers;
                        view.attach_running_installs(cx);
                        view.maybe_probe_providers(cx);
                    }
                    Err(err) => {
                        view.providers.clear();
                        view.provider_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn refresh_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_loading = true;
        self.settings_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let settings = client.get_settings().await?;
            Ok(settings)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.settings_loading = false;
                match result {
                    Ok(settings) => {
                        view.settings = Some(settings);
                    }
                    Err(err) => {
                        view.settings = None;
                        view.settings_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn select_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let next = Some(workspace.id);
        if self.selected_workspace == next {
            return;
        }
        self.selected_workspace = next;
        self.provider_options.clear();
        self.opts_busy.clear();
        self.auth_busy.clear();
        self.verify_busy.clear();
        self.maybe_probe_providers(cx);
        cx.notify();
    }

    pub(crate) fn ensure_provider_options(
        &mut self,
        provider_id: String,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.opts_busy.get(&provider_id).copied().unwrap_or(false) {
            return;
        }
        if !force && self.provider_options.contains_key(&provider_id) {
            return;
        }

        self.opts_busy.insert(provider_id.clone(), true);
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let options = client
                .get_provider_options(workspace_id, &provider_id)
                .await?;
            Ok(options)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.opts_busy.remove(&provider_id);
                match result {
                    Ok(options) => {
                        view.provider_options.insert(provider_id.clone(), options);
                    }
                    Err(err) => {
                        view.provider_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn authenticate_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.auth_busy.get(&provider_id).copied().unwrap_or(false) {
            return;
        }

        self.auth_busy.insert(provider_id.clone(), true);
        self.provider_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            client
                .authenticate_provider_for_workspace(workspace_id, &provider_id, None)
                .await?;
            Ok(())
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.auth_busy.remove(&provider_id);
                if let Err(err) = result {
                    view.provider_error = Some(err.to_string());
                }
                view.ensure_provider_options(provider_id.clone(), true, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn verify_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.verify_busy.get(&provider_id).copied().unwrap_or(false) {
            return;
        }

        self.verify_busy.insert(provider_id.clone(), true);
        self.provider_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            client
                .verify_provider_for_workspace(workspace_id, &provider_id)
                .await?;
            Ok(())
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.verify_busy.remove(&provider_id);
                if let Err(err) = result {
                    view.provider_error = Some(err.to_string());
                }
                view.ensure_provider_options(provider_id.clone(), true, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn install_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        if self.install_busy.is_some() {
            return;
        }

        self.install_busy = Some(provider_id.clone());
        self.provider_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let resp = client.install_provider(&provider_id).await?;
            Ok(resp)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.install_busy = None;
                match result {
                    Ok(resp) => {
                        view.attach_install(resp.provider_id, resp.install_id, cx);
                    }
                    Err(err) => {
                        view.provider_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn install_all_providers(&mut self, cx: &mut Context<Self>) {
        if self.install_busy.is_some() {
            return;
        }

        self.install_busy = Some("all".to_string());
        self.provider_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let resp = client.install_all_providers().await?;
            Ok(resp)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.install_busy = None;
                match result {
                    Ok(installs) => {
                        for install in installs {
                            view.attach_install(install.provider_id, install.install_id, cx);
                        }
                    }
                    Err(err) => {
                        view.provider_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn attach_running_installs(&mut self, cx: &mut Context<Self>) {
        for provider in &self.providers {
            let running = provider
                .details
                .get("install_running")
                .map(|value| value == "true")
                .unwrap_or(false);
            if !running {
                continue;
            }
            let Some(install_id) = provider.details.get("install_id") else {
                continue;
            };
            if self.installs.contains_key(&provider.provider_id) {
                continue;
            }
            self.attach_install(
                provider.provider_id.clone(),
                install_id.to_string(),
                cx,
            );
        }
    }

    fn attach_install(&mut self, provider_id: String, install_id: String, cx: &mut Context<Self>) {
        if self.install_polling.contains(&provider_id) {
            return;
        }
        self.install_polling.insert(provider_id.clone());
        self.installs
            .entry(provider_id.clone())
            .or_insert(InstallSession {
                install_id: install_id.clone(),
                state: InstallStateKind::Running,
                pct: None,
                last_stage: None,
                last_message: None,
                error: None,
            });
        cx.notify();

        cx.spawn(|this, cx| async move {
            let client = match Client::from_env() {
                Ok(client) => client,
                Err(err) => {
                    this.update(cx, |view, cx| {
                        view.install_polling.remove(&provider_id);
                        view.provider_error = Some(err.to_string());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            if let Ok(events) = client.list_install_events(&install_id).await {
                if let Some(event) = events.last() {
                    this.update(cx, |view, cx| {
                        view.update_install_event(&provider_id, event);
                        cx.notify();
                    })
                    .ok();
                }
            }

            loop {
                match client.get_install(&install_id).await {
                    Ok(info) => {
                        let running = matches!(info.state, InstallStateKind::Running);
                        this.update(cx, |view, cx| {
                            view.update_install_state(&provider_id, info);
                            if !running {
                                view.install_polling.remove(&provider_id);
                                view.refresh_providers(cx);
                            }
                            cx.notify();
                        })
                        .ok();
                        if !running {
                            break;
                        }
                    }
                    Err(err) => {
                        this.update(cx, |view, cx| {
                            view.install_polling.remove(&provider_id);
                            view.provider_error = Some(err.to_string());
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                }

                tokio::time::sleep(Duration::from_millis(900)).await;
            }
        })
        .detach();
    }

    fn update_install_state(&mut self, provider_id: &str, info: InstallInfo) {
        let session = self.installs.entry(provider_id.to_string()).or_insert(
            InstallSession {
                install_id: info.install_id.clone(),
                state: info.state.clone(),
                pct: None,
                last_stage: None,
                last_message: None,
                error: None,
            },
        );
        session.state = info.state;
        session.error = info.error;
        if let Some(event) = info.last_event.as_ref() {
            self.update_install_event(provider_id, event);
        }
    }

    fn update_install_event(&mut self, provider_id: &str, event: &InstallProgressEvent) {
        let session = self
            .installs
            .entry(provider_id.to_string())
            .or_insert(InstallSession {
                install_id: event.install_id.clone(),
                state: InstallStateKind::Running,
                pct: None,
                last_stage: None,
                last_message: None,
                error: None,
            });
        session.pct = install_progress_pct(event);
        session.last_stage = Some(event.stage.clone());
        session.last_message = Some(event.message.clone());
    }

    fn maybe_probe_providers(&mut self, cx: &mut Context<Self>) {
        if self.selected_workspace.is_none() {
            return;
        }
        for provider in &self.providers {
            self.ensure_provider_options(provider.provider_id.clone(), false, cx);
        }
    }

    pub(crate) fn summary_rows(&self) -> Vec<SettingsSummaryRow> {
        let fallback = if self.settings_loading {
            "Loading...".to_string()
        } else if self.settings_error.is_some() {
            "Error".to_string()
        } else {
            "Unavailable".to_string()
        };

        let Some(settings) = self.settings.as_ref() else {
            return vec![
                SettingsSummaryRow {
                    label: "Dictation".to_string(),
                    value: fallback.clone(),
                },
                SettingsSummaryRow {
                    label: "Telemetry".to_string(),
                    value: fallback.clone(),
                },
                SettingsSummaryRow {
                    label: "Title Generation".to_string(),
                    value: fallback.clone(),
                },
                SettingsSummaryRow {
                    label: "Resource Governance".to_string(),
                    value: fallback,
                },
            ];
        };

        vec![
            SettingsSummaryRow {
                label: "Dictation".to_string(),
                value: dictation_summary(settings),
            },
            SettingsSummaryRow {
                label: "Telemetry".to_string(),
                value: telemetry_summary(settings),
            },
            SettingsSummaryRow {
                label: "Title Generation".to_string(),
                value: title_generation_summary(settings),
            },
            SettingsSummaryRow {
                label: "Resource Governance".to_string(),
                value: resource_governance_summary(settings),
            },
        ]
    }
}

fn install_progress_pct(event: &InstallProgressEvent) -> Option<u8> {
    let bytes = event.bytes?;
    let total = event.total_bytes?;
    if total == 0 {
        return None;
    }
    let pct = (bytes as f64 / total as f64) * 100.0;
    Some(pct.round().clamp(0.0, 100.0) as u8)
}

fn dictation_summary(settings: &PublicSettings) -> String {
    let Some(dictation) = settings.dictation.as_ref() else {
        return "Unset".to_string();
    };
    if !dictation.enabled {
        return "Disabled".to_string();
    }
    let provider = dictation_provider_label(&dictation.provider);
    let mut out = format!("Enabled ({})", provider);
    if let Some(livekit) = dictation.livekit.as_ref() {
        if !livekit.model.trim().is_empty() {
            out = format!("{}, model: {}", out, livekit.model.trim());
        }
    }
    out
}

fn telemetry_summary(settings: &PublicSettings) -> String {
    let Some(telemetry) = settings.telemetry.as_ref() else {
        return "Unset".to_string();
    };
    if telemetry.enabled {
        "Enabled".to_string()
    } else {
        "Disabled".to_string()
    }
}

fn title_generation_summary(settings: &PublicSettings) -> String {
    let Some(title_generation) = settings.title_generation.as_ref() else {
        return "Unset".to_string();
    };
    if title_generation.api_key.trim().is_empty() {
        "Not configured".to_string()
    } else {
        format!("Configured ({})", title_generation.model.trim())
    }
}

fn resource_governance_summary(settings: &PublicSettings) -> String {
    let Some(resource) = settings.resource_governance.as_ref() else {
        return "Unset".to_string();
    };
    let mode = resource_mode_label(&resource.mode);
    let mut out = if resource.enabled {
        format!("Enabled ({})", mode)
    } else {
        "Disabled".to_string()
    };
    if let Some(status) = resource.status.as_ref() {
        out = format!("{}, status: {}", out, resource_status_label(&status.state));
    }
    out
}

fn dictation_provider_label(provider: &DictationProvider) -> &'static str {
    match provider {
        DictationProvider::Disabled => "disabled",
        DictationProvider::LiveKitInference => "livekit_inference",
    }
}

fn resource_mode_label(mode: &ResourceGovernanceMode) -> &'static str {
    match mode {
        ResourceGovernanceMode::Auto => "auto",
        ResourceGovernanceMode::Custom => "custom",
    }
}

fn resource_status_label(state: &ResourceGovernanceStatusState) -> &'static str {
    match state {
        ResourceGovernanceStatusState::Disabled => "disabled",
        ResourceGovernanceStatusState::Applied => "applied",
        ResourceGovernanceStatusState::Pending => "pending",
        ResourceGovernanceStatusState::Unsupported => "unsupported",
        ResourceGovernanceStatusState::Error => "error",
    }
}
