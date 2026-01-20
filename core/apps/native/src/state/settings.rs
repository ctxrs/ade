use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as AnyhowContext, Result};
use directories::ProjectDirs;
use gpui::{AppContext as _, Context, Entity, Image, Subscription, Window};
use gpui_tokio::Tokio;
use serde::{Deserialize, Serialize};
use tokio::fs;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{
    WorkspaceAttachment, WorkspaceAttachmentKind,
};
use ctx_providers::adapters::ProviderStatus;

use super::super::harness_catalog::build_harness_logo_map;
use crate::theme::ThemeColors;

use super::workspace::WorkspaceItem;

use ctx_client::{
    Client, CreateWorkspaceAttachmentRequest, DeleteWorkspaceAttachmentRequest, DictationProvider,
    EnableMobileAccessResponse, InstallInfo, InstallProgressEvent, InstallStateKind,
    MobileAccessStatus, ProviderOptions, PublicResourceGovernanceLimits,
    PublicResourceGovernanceStatus, PublicSettings, ResourceGovernanceMode,
ResourceUtilizationSnapshot,
    UpdateDictationSettingsRequest, UpdateLiveKitDictationSettingsRequest,
    UpdateResourceGovernanceSettingsRequest, UpdateSettingsRequest,
    UpdateTelemetrySettingsRequest, UpdateTitleGenerationSettingsRequest,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::select::{SelectEvent, SelectState, SearchableVec};

#[derive(Clone)]
pub(crate) struct InstallSession {
    pub(crate) state: InstallStateKind,
    pub(crate) pct: Option<u8>,
    pub(crate) last_stage: Option<String>,
    pub(crate) last_message: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopEditorSettings {
    target: String,
    #[serde(default)]
    custom_command: Option<String>,
    #[serde(default)]
    remote_authority: Option<String>,
}

impl Default for DesktopEditorSettings {
    fn default() -> Self {
        Self {
            target: "system".to_string(),
            custom_command: None,
            remote_authority: None,
        }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSectionGroup {
    Main,
    Advanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SettingsSection {
    General,
    AgentHarnesses,
    HarnessSubscriptions,
    ModelsRouting,
    Sandboxing,
    WorktreeBootstrap,
    AgentSystemPrompt,
    WorkspaceAttachments,
    MergeQueue,
    ContextPack,
    ResourceGovernance,
    MobileAccess,
    ResourceUtilization,
    Dictation,
    TitleGeneration,
    Billing,
    TeamEnterprise,
    UsageAnalytics,
}

impl SettingsSection {
    pub(crate) fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "General",
            SettingsSection::AgentHarnesses => "Agent Harnesses",
            SettingsSection::HarnessSubscriptions => "Harness Subscriptions",
            SettingsSection::ModelsRouting => "Models & Routing",
            SettingsSection::Sandboxing => "Sandboxing",
            SettingsSection::WorktreeBootstrap => "Worktree Bootstrap",
            SettingsSection::AgentSystemPrompt => "Agent System Prompt",
            SettingsSection::WorkspaceAttachments => "Workspace Attachments",
            SettingsSection::MergeQueue => "Merge Queue",
            SettingsSection::ContextPack => "ctx pack",
            SettingsSection::ResourceGovernance => "Resource Limits",
            SettingsSection::MobileAccess => "Mobile Access",
            SettingsSection::ResourceUtilization => "Resource Utilization",
            SettingsSection::Dictation => "Dictation",
            SettingsSection::TitleGeneration => "Title Generation",
            SettingsSection::Billing => "Billing",
            SettingsSection::TeamEnterprise => "Team & Enterprise",
            SettingsSection::UsageAnalytics => "Usage Analytics",
        }
    }

    pub(crate) fn group(self) -> SettingsSectionGroup {
        match self {
            SettingsSection::Dictation
            | SettingsSection::TitleGeneration
            | SettingsSection::Billing
            | SettingsSection::TeamEnterprise
            | SettingsSection::UsageAnalytics => SettingsSectionGroup::Advanced,
            _ => SettingsSectionGroup::Main,
        }
    }
}

pub(crate) const SETTINGS_SECTIONS: &[SettingsSection] = &[
    SettingsSection::General,
    SettingsSection::AgentHarnesses,
    SettingsSection::HarnessSubscriptions,
    SettingsSection::ModelsRouting,
    SettingsSection::Sandboxing,
    SettingsSection::WorktreeBootstrap,
    SettingsSection::AgentSystemPrompt,
    SettingsSection::WorkspaceAttachments,
    SettingsSection::MergeQueue,
    SettingsSection::ContextPack,
    SettingsSection::ResourceGovernance,
    SettingsSection::MobileAccess,
    SettingsSection::ResourceUtilization,
    SettingsSection::Dictation,
    SettingsSection::TitleGeneration,
    SettingsSection::Billing,
    SettingsSection::TeamEnterprise,
    SettingsSection::UsageAnalytics,
];

#[derive(Clone)]
pub(crate) struct LabeledOption {
    pub(crate) value: String,
    pub(crate) label: String,
}

impl gpui_component::select::SelectItem for LabeledOption {
    type Value = String;

    fn title(&self) -> gpui::SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SettingsInputKind {
    Search,
    EditorCustom,
    EditorRemote,
    AttachmentSource,
    AttachmentName,
    AttachmentRevision,
    DocsSource,
    DocsName,
    ResourceCpu,
    ResourceMemHigh,
    ResourceMemMax,
    DictationBaseUrl,
    DictationApiKey,
    DictationApiSecret,
    DictationLanguage,
    TitleBaseUrl,
    TitleApiKey,
    TitleModel,
    BillingEmail,
    BillingPassword,
}

#[derive(Clone, Copy)]
pub(crate) enum SettingsSelectKind {
    EditorTarget,
    Workspace,
    ResourceMode,
    DictationModel,
}

pub(crate) struct SettingsState {
    pub(crate) colors: ThemeColors,
    #[allow(dead_code)]
    pub(crate) is_dark: bool,
    pub(crate) harness_logos: HashMap<String, Arc<Image>>,
    pub(crate) shell_handle: Option<Entity<crate::app::ShellView>>,
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
    pub(crate) active_section: SettingsSection,
    pub(crate) saving: bool,
    pub(crate) save_error: Option<String>,
    pub(crate) save_seq: u64,
    pub(crate) telemetry_enabled: bool,
    pub(crate) telemetry_endpoint: String,
    pub(crate) telemetry_hydrated: bool,
    pub(crate) telemetry_save_seq: u64,
    pub(crate) editor_target: String,
    pub(crate) editor_custom_command: String,
    pub(crate) editor_remote_authority: String,
    pub(crate) editor_loaded: bool,
    pub(crate) editor_saving: bool,
    pub(crate) editor_error: Option<String>,
    pub(crate) editor_hydrated: bool,
    pub(crate) editor_save_seq: u64,
    pub(crate) attachments: Vec<WorkspaceAttachment>,
    pub(crate) attachments_loading: bool,
    pub(crate) attachments_error: Option<String>,
    pub(crate) attachment_source: String,
    pub(crate) attachment_name: String,
    pub(crate) attachment_revision: String,
    pub(crate) docs_attachment_source: String,
    pub(crate) docs_attachment_name: String,
    pub(crate) attachment_busy: bool,
    pub(crate) attachment_sync_busy: bool,
    pub(crate) docs_attachment_busy: bool,
    pub(crate) attachment_delete_busy: HashMap<String, bool>,
    pub(crate) resource_snapshot: Option<ResourceUtilizationSnapshot>,
    pub(crate) resource_loading: bool,
    pub(crate) resource_error: Option<String>,
    pub(crate) resource_effective: Option<PublicResourceGovernanceLimits>,
    pub(crate) resource_status: Option<PublicResourceGovernanceStatus>,
    pub(crate) resource_poll_token: u64,
    pub(crate) expanded_process_pids: HashMap<u32, bool>,
    pub(crate) mobile_status: Option<MobileAccessStatus>,
    pub(crate) mobile_status_loading: bool,
    pub(crate) mobile_status_error: Option<String>,
    pub(crate) mobile_enable_error: Option<String>,
    pub(crate) mobile_enable_busy: bool,
    pub(crate) mobile_qr: Option<EnableMobileAccessResponse>,
    pub(crate) dictation_enabled: bool,
    pub(crate) dictation_secret_set: bool,
    pub(crate) dictation_base_url: String,
    pub(crate) dictation_api_key: String,
    pub(crate) dictation_api_secret: String,
    pub(crate) dictation_language: String,
    pub(crate) dictation_model: String,
    pub(crate) dictation_save_seq: u64,
    pub(crate) title_base_url: String,
    pub(crate) title_api_key: String,
    pub(crate) title_model: String,
    pub(crate) title_use_json: bool,
    pub(crate) title_save_seq: u64,
    pub(crate) resource_mode: ResourceGovernanceMode,
    pub(crate) resource_enabled: bool,
    pub(crate) resource_cpu_quota_pct: String,
    pub(crate) resource_memory_high_gb: String,
    pub(crate) resource_memory_max_gb: String,
    pub(crate) resource_save_seq: u64,
    pub(crate) inputs_dirty: bool,
    pub(crate) dictation_hydrated: bool,
    pub(crate) title_hydrated: bool,
    pub(crate) resource_hydrated: bool,
    pub(crate) billing_email: String,
    pub(crate) billing_password: String,
    pub(crate) search_input: Option<Entity<InputState>>,
    pub(crate) editor_custom_input: Option<Entity<InputState>>,
    pub(crate) editor_remote_input: Option<Entity<InputState>>,
    pub(crate) attachment_source_input: Option<Entity<InputState>>,
    pub(crate) attachment_name_input: Option<Entity<InputState>>,
    pub(crate) attachment_revision_input: Option<Entity<InputState>>,
    pub(crate) docs_source_input: Option<Entity<InputState>>,
    pub(crate) docs_name_input: Option<Entity<InputState>>,
    pub(crate) resource_cpu_input: Option<Entity<InputState>>,
    pub(crate) resource_memory_high_input: Option<Entity<InputState>>,
    pub(crate) resource_memory_max_input: Option<Entity<InputState>>,
    pub(crate) dictation_base_url_input: Option<Entity<InputState>>,
    pub(crate) dictation_api_key_input: Option<Entity<InputState>>,
    pub(crate) dictation_api_secret_input: Option<Entity<InputState>>,
    pub(crate) dictation_language_input: Option<Entity<InputState>>,
    pub(crate) title_base_url_input: Option<Entity<InputState>>,
    pub(crate) title_api_key_input: Option<Entity<InputState>>,
    pub(crate) title_model_input: Option<Entity<InputState>>,
    pub(crate) billing_email_input: Option<Entity<InputState>>,
    pub(crate) billing_password_input: Option<Entity<InputState>>,
    pub(crate) editor_target_select: Option<Entity<SelectState<SearchableVec<LabeledOption>>>>,
    pub(crate) workspace_select: Option<Entity<SelectState<SearchableVec<LabeledOption>>>>,
    pub(crate) resource_mode_select: Option<Entity<SelectState<SearchableVec<LabeledOption>>>>,
    pub(crate) dictation_model_select: Option<Entity<SelectState<SearchableVec<LabeledOption>>>>,
    pub(crate) input_subscriptions: Vec<Subscription>,
}

impl SettingsState {
    pub(crate) fn new(colors: ThemeColors, is_dark: bool) -> Self {
        Self {
            colors,
            is_dark,
            harness_logos: build_harness_logo_map(is_dark),
            shell_handle: None,
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
            active_section: SettingsSection::ContextPack,
            saving: false,
            save_error: None,
            save_seq: 0,
            telemetry_enabled: true,
            telemetry_endpoint: String::new(),
            telemetry_hydrated: false,
            telemetry_save_seq: 0,
            editor_target: "system".to_string(),
            editor_custom_command: String::new(),
            editor_remote_authority: String::new(),
            editor_loaded: false,
            editor_saving: false,
            editor_error: None,
            editor_hydrated: false,
            editor_save_seq: 0,
            attachments: Vec::new(),
            attachments_loading: false,
            attachments_error: None,
            attachment_source: String::new(),
            attachment_name: String::new(),
            attachment_revision: String::new(),
            docs_attachment_source: String::new(),
            docs_attachment_name: String::new(),
            attachment_busy: false,
            attachment_sync_busy: false,
            docs_attachment_busy: false,
            attachment_delete_busy: HashMap::new(),
            resource_snapshot: None,
            resource_loading: false,
            resource_error: None,
            resource_effective: None,
            resource_status: None,
            resource_poll_token: 0,
            expanded_process_pids: HashMap::new(),
            mobile_status: None,
            mobile_status_loading: false,
            mobile_status_error: None,
            mobile_enable_error: None,
            mobile_enable_busy: false,
            mobile_qr: None,
            dictation_enabled: true,
            dictation_secret_set: false,
            dictation_base_url: "https://agent-gateway.livekit.cloud/v1".to_string(),
            dictation_api_key: String::new(),
            dictation_api_secret: String::new(),
            dictation_language: "en".to_string(),
            dictation_model: "auto".to_string(),
            dictation_save_seq: 0,
            title_base_url: "https://openrouter.ai/api/v1".to_string(),
            title_api_key: String::new(),
            title_model: "google/gemini-3-flash-preview".to_string(),
            title_use_json: false,
            title_save_seq: 0,
            resource_mode: ResourceGovernanceMode::Auto,
            resource_enabled: false,
            resource_cpu_quota_pct: String::new(),
            resource_memory_high_gb: String::new(),
            resource_memory_max_gb: String::new(),
            resource_save_seq: 0,
            inputs_dirty: false,
            dictation_hydrated: false,
            title_hydrated: false,
            resource_hydrated: false,
            billing_email: String::new(),
            billing_password: String::new(),
            search_input: None,
            editor_custom_input: None,
            editor_remote_input: None,
            attachment_source_input: None,
            attachment_name_input: None,
            attachment_revision_input: None,
            docs_source_input: None,
            docs_name_input: None,
            resource_cpu_input: None,
            resource_memory_high_input: None,
            resource_memory_max_input: None,
            dictation_base_url_input: None,
            dictation_api_key_input: None,
            dictation_api_secret_input: None,
            dictation_language_input: None,
            title_base_url_input: None,
            title_api_key_input: None,
            title_model_input: None,
            billing_email_input: None,
            billing_password_input: None,
            editor_target_select: None,
            workspace_select: None,
            resource_mode_select: None,
            dictation_model_select: None,
            input_subscriptions: Vec::new(),
        }
    }

    pub(crate) fn start_load(&mut self, cx: &mut Context<Self>) {
        self.refresh_workspaces(cx);
        self.refresh_providers(cx);
        self.refresh_settings(cx);
        self.refresh_editor_settings(cx);
        self.refresh_workspace_attachments(cx);
        self.refresh_resource_utilization(cx);
        self.refresh_mobile_access_status(cx);
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

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
                                root_path: workspace.root_path,
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

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.settings_loading = false;
                match result {
                    Ok(settings) => {
                        view.apply_settings_snapshot(settings);
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

    pub(crate) fn refresh_editor_settings(&mut self, cx: &mut Context<Self>) {
        self.editor_loaded = false;
        self.editor_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move { load_editor_settings().await });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(settings) => view.apply_editor_settings(settings),
                    Err(err) => {
                        view.editor_loaded = true;
                        view.editor_error = Some(err.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_editor_settings(&mut self, settings: DesktopEditorSettings) {
        self.editor_target = settings.target;
        self.editor_custom_command = settings.custom_command.unwrap_or_default();
        self.editor_remote_authority = settings.remote_authority.unwrap_or_default();
        self.editor_loaded = true;
        self.editor_hydrated = false;
        self.inputs_dirty = true;
    }

    fn build_editor_settings(&self) -> DesktopEditorSettings {
        let target = self.editor_target.trim().to_string();
        let custom_command = if target == "custom" {
            let value = self.editor_custom_command.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        } else {
            None
        };
        let remote_authority = self.editor_remote_authority.trim();
        DesktopEditorSettings {
            target,
            custom_command,
            remote_authority: if remote_authority.is_empty() {
                None
            } else {
                Some(remote_authority.to_string())
            },
        }
    }

    fn schedule_editor_save(&mut self, cx: &mut Context<Self>) {
        if !self.editor_loaded {
            return;
        }
        if !self.editor_hydrated {
            self.editor_hydrated = true;
            return;
        }

        self.editor_save_seq += 1;
        let seq = self.editor_save_seq;
        let payload = self.build_editor_settings();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            tokio::time::sleep(Duration::from_millis(350)).await;

            let should_save = this
                .update(cx, |view, cx| {
                    if view.editor_save_seq != seq {
                        return false;
                    }
                    view.editor_saving = true;
                    view.editor_error = None;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !should_save {
                return;
            }

            let result = save_editor_settings(&payload).await;
            this.update(cx, |view, cx| {
                if view.editor_save_seq != seq {
                    return;
                }
                view.editor_saving = false;
                match result {
                    Ok(saved) => view.apply_editor_settings(saved),
                    Err(err) => view.editor_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn schedule_telemetry_save(&mut self, cx: &mut Context<Self>) {
        if !self.telemetry_hydrated {
            self.telemetry_hydrated = true;
            return;
        }
        self.telemetry_save_seq += 1;
        let seq = self.telemetry_save_seq;
        let payload = self.telemetry_payload();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            tokio::time::sleep(Duration::from_millis(250)).await;
            this.update(cx, |view, cx| {
                if view.telemetry_save_seq != seq {
                    return;
                }
                view.save_settings_patch(
                    UpdateSettingsRequest {
                        telemetry: Some(payload),
                        dictation: None,
                        title_generation: None,
                        resource_governance: None,
                        provider_guard: None,
                        subagents: None,
                        compaction: None,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn schedule_dictation_save(&mut self, cx: &mut Context<Self>) {
        if !self.dictation_hydrated {
            self.dictation_hydrated = true;
            return;
        }
        if !self.dictation_can_save() {
            return;
        }
        self.dictation_save_seq += 1;
        let seq = self.dictation_save_seq;
        let payload = self.dictation_payload();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            tokio::time::sleep(Duration::from_millis(450)).await;
            this.update(cx, |view, cx| {
                if view.dictation_save_seq != seq {
                    return;
                }
                view.save_settings_patch(
                    UpdateSettingsRequest {
                        telemetry: None,
                        dictation: Some(payload),
                        title_generation: None,
                        resource_governance: None,
                        provider_guard: None,
                        subagents: None,
                        compaction: None,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn schedule_title_save(&mut self, cx: &mut Context<Self>) {
        if !self.title_hydrated {
            self.title_hydrated = true;
            return;
        }
        self.title_save_seq += 1;
        let seq = self.title_save_seq;
        let payload = self.title_payload();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            tokio::time::sleep(Duration::from_millis(450)).await;
            this.update(cx, |view, cx| {
                if view.title_save_seq != seq {
                    return;
                }
                view.save_settings_patch(
                    UpdateSettingsRequest {
                        telemetry: None,
                        dictation: None,
                        title_generation: Some(payload),
                        resource_governance: None,
                        provider_guard: None,
                        subagents: None,
                        compaction: None,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn schedule_resource_save(&mut self, cx: &mut Context<Self>) {
        if !self.resource_hydrated {
            self.resource_hydrated = true;
            return;
        }
        if !self.resource_governance_can_save() {
            return;
        }
        self.resource_save_seq += 1;
        let seq = self.resource_save_seq;
        let payload = self.resource_payload();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            tokio::time::sleep(Duration::from_millis(450)).await;
            this.update(cx, |view, cx| {
                if view.resource_save_seq != seq {
                    return;
                }
                view.save_settings_patch(
                    UpdateSettingsRequest {
                        telemetry: None,
                        dictation: None,
                        title_generation: None,
                        resource_governance: Some(payload),
                        provider_guard: None,
                        subagents: None,
                        compaction: None,
                    },
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn telemetry_payload(&self) -> UpdateTelemetrySettingsRequest {
        UpdateTelemetrySettingsRequest {
            enabled: self.telemetry_enabled,
            endpoint: self.telemetry_endpoint.trim().to_string(),
        }
    }

    fn dictation_payload(&self) -> UpdateDictationSettingsRequest {
        let language = self.dictation_language.trim();
        let api_secret = self.dictation_api_secret.trim();
        UpdateDictationSettingsRequest {
            enabled: self.dictation_enabled,
            provider: if self.dictation_enabled {
                DictationProvider::LiveKitInference
            } else {
                DictationProvider::Disabled
            },
            livekit: Some(UpdateLiveKitDictationSettingsRequest {
                base_url: self.dictation_base_url.trim().to_string(),
                api_key: self.dictation_api_key.trim().to_string(),
                api_secret: if api_secret.is_empty() {
                    None
                } else {
                    Some(api_secret.to_string())
                },
                model: self.dictation_model.trim().to_string(),
                language: if language.is_empty() {
                    "en".to_string()
                } else {
                    language.to_string()
                },
            }),
        }
    }

    fn title_payload(&self) -> UpdateTitleGenerationSettingsRequest {
        UpdateTitleGenerationSettingsRequest {
            base_url: self.title_base_url.trim().to_string(),
            api_key: self.title_api_key.trim().to_string(),
            model: self.title_model.trim().to_string(),
            use_json: self.title_use_json,
        }
    }

    fn resource_payload(&self) -> UpdateResourceGovernanceSettingsRequest {
        let cpu_quota_pct = self
            .resource_cpu_quota_pct
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| *v > 0.0)
            .map(|v| v.round() as u32);
        let memory_high_mb = parse_gib(&self.resource_memory_high_gb);
        let memory_max_mb = parse_gib(&self.resource_memory_max_gb);
        UpdateResourceGovernanceSettingsRequest {
            enabled: self.resource_enabled,
            mode: self.resource_mode.clone(),
            cpu_quota_pct: if matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
                cpu_quota_pct
            } else {
                None
            },
            memory_high_mb: if matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
                memory_high_mb
            } else {
                None
            },
            memory_max_mb: if matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
                memory_max_mb
            } else {
                None
            },
        }
    }

    pub(crate) fn dictation_can_save(&self) -> bool {
        if !self.dictation_enabled {
            return true;
        }
        if self.dictation_api_key.trim().is_empty() {
            return false;
        }
        if !self.dictation_secret_set && self.dictation_api_secret.trim().is_empty() {
            return false;
        }
        true
    }

    pub(crate) fn resource_governance_can_save(&self) -> bool {
        if !self.resource_enabled {
            return true;
        }
        if !matches!(self.resource_mode, ResourceGovernanceMode::Custom) {
            return true;
        }
        let high = parse_gib(&self.resource_memory_high_gb);
        let max = parse_gib(&self.resource_memory_max_gb);
        if let (Some(high), Some(max)) = (high, max) {
            return high <= max;
        }
        true
    }

    pub(crate) fn set_telemetry_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.telemetry_enabled == enabled {
            return;
        }
        self.telemetry_enabled = enabled;
        self.schedule_telemetry_save(cx);
        cx.notify();
    }

    pub(crate) fn set_dictation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.dictation_enabled == enabled {
            return;
        }
        self.dictation_enabled = enabled;
        self.schedule_dictation_save(cx);
        cx.notify();
    }

    pub(crate) fn set_title_use_json(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.title_use_json == enabled {
            return;
        }
        self.title_use_json = enabled;
        self.schedule_title_save(cx);
        cx.notify();
    }

    pub(crate) fn set_resource_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.resource_enabled == enabled {
            return;
        }
        self.resource_enabled = enabled;
        self.schedule_resource_save(cx);
        cx.notify();
    }



    pub(crate) fn apply_resource_governance_now(&mut self, cx: &mut Context<Self>) {
        let payload = self.resource_payload();
        self.save_settings_patch(
            UpdateSettingsRequest {
                telemetry: None,
                dictation: None,
                title_generation: None,
                resource_governance: Some(payload),
                provider_guard: None,
                subagents: None,
                        compaction: None,
            },
            cx,
        );
    }

    pub(crate) fn toggle_process_expanded(&mut self, pid: u32, cx: &mut Context<Self>) {
        let entry = self.expanded_process_pids.entry(pid).or_insert(false);
        *entry = !*entry;
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn update_theme(&mut self, colors: ThemeColors, is_dark: bool) {
        self.colors = colors;
        if self.is_dark != is_dark {
            self.is_dark = is_dark;
            self.harness_logos = build_harness_logo_map(is_dark);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_shell_handle(&mut self, handle: Entity<crate::app::ShellView>) {
        self.shell_handle = Some(handle);
    }

    fn apply_settings_snapshot(&mut self, settings: PublicSettings) {
        if let Some(telemetry) = settings.telemetry.as_ref() {
            self.telemetry_enabled = telemetry.enabled;
            self.telemetry_endpoint = telemetry.endpoint.clone();
        } else {
            self.telemetry_enabled = true;
            self.telemetry_endpoint.clear();
        }

        if let Some(dictation) = settings.dictation.as_ref() {
            let livekit = dictation.livekit.as_ref();
            self.dictation_enabled = dictation.enabled;
            self.dictation_base_url = livekit
                .map(|l| l.base_url.as_str())
                .unwrap_or("https://agent-gateway.livekit.cloud/v1")
                .to_string();
            self.dictation_api_key = livekit
                .map(|l| l.api_key.as_str())
                .unwrap_or("")
                .to_string();
            self.dictation_language = livekit
                .map(|l| l.language.as_str())
                .unwrap_or("en")
                .to_string();
            self.dictation_model = normalize_dictation_model(
                livekit
                    .map(|l| l.model.as_str())
                    .unwrap_or("auto"),
            );
            self.dictation_secret_set = livekit.map(|l| l.api_secret_set).unwrap_or(false);
            if self.dictation_secret_set {
                self.dictation_api_secret.clear();
            }
        } else {
            self.dictation_enabled = false;
            self.dictation_base_url = "https://agent-gateway.livekit.cloud/v1".to_string();
            self.dictation_api_key.clear();
            self.dictation_api_secret.clear();
            self.dictation_language = "en".to_string();
            self.dictation_model = "auto".to_string();
            self.dictation_secret_set = false;
        }

        if let Some(title) = settings.title_generation.as_ref() {
            self.title_base_url = title
                .base_url
                .as_str()
                .trim()
                .to_string();
            self.title_api_key = title.api_key.as_str().trim().to_string();
            self.title_model = title.model.as_str().trim().to_string();
            self.title_use_json = title.use_json;
        } else {
            self.title_base_url = "https://openrouter.ai/api/v1".to_string();
            self.title_api_key.clear();
            self.title_model = "google/gemini-3-flash-preview".to_string();
            self.title_use_json = false;
        }

        if let Some(resource) = settings.resource_governance.as_ref() {
            self.resource_enabled = resource.enabled;
            self.resource_mode = resource.mode.clone();
            self.resource_cpu_quota_pct = resource
                .cpu_quota_pct
                .map(|v| v.to_string())
                .unwrap_or_default();
            self.resource_memory_high_gb = format_gib(resource.memory_high_mb);
            self.resource_memory_max_gb = format_gib(resource.memory_max_mb);
            self.resource_effective = resource.effective.clone();
            self.resource_status = resource.status.clone();
        } else {
            self.resource_enabled = false;
            self.resource_mode = ResourceGovernanceMode::Auto;
            self.resource_cpu_quota_pct.clear();
            self.resource_memory_high_gb.clear();
            self.resource_memory_max_gb.clear();
            self.resource_effective = None;
            self.resource_status = None;
        }

        self.settings = Some(settings);
        self.inputs_dirty = true;
        self.telemetry_hydrated = false;
        self.dictation_hydrated = false;
        self.title_hydrated = false;
        self.resource_hydrated = false;
    }

    pub(crate) fn sync_input_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.inputs_dirty {
            return;
        }
        if let Some(input) = self.editor_custom_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.editor_custom_command.clone(), window, cx);
            });
        }
        if let Some(input) = self.editor_remote_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.editor_remote_authority.clone(), window, cx);
            });
        }
        if let Some(input) = self.attachment_source_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.attachment_source.clone(), window, cx);
            });
        }
        if let Some(input) = self.attachment_name_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.attachment_name.clone(), window, cx);
            });
        }
        if let Some(input) = self.attachment_revision_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.attachment_revision.clone(), window, cx);
            });
        }
        if let Some(input) = self.docs_source_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.docs_attachment_source.clone(), window, cx);
            });
        }
        if let Some(input) = self.docs_name_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.docs_attachment_name.clone(), window, cx);
            });
        }
        if let Some(input) = self.resource_cpu_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.resource_cpu_quota_pct.clone(), window, cx);
            });
        }
        if let Some(input) = self.resource_memory_high_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.resource_memory_high_gb.clone(), window, cx);
            });
        }
        if let Some(input) = self.resource_memory_max_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.resource_memory_max_gb.clone(), window, cx);
            });
        }
        if let Some(input) = self.dictation_base_url_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.dictation_base_url.clone(), window, cx);
            });
        }
        if let Some(input) = self.dictation_api_key_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.dictation_api_key.clone(), window, cx);
            });
        }
        if let Some(input) = self.dictation_api_secret_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.dictation_api_secret.clone(), window, cx);
            });
        }
        if let Some(input) = self.dictation_language_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.dictation_language.clone(), window, cx);
            });
        }
        if let Some(input) = self.title_base_url_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.title_base_url.clone(), window, cx);
            });
        }
        if let Some(input) = self.title_api_key_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.title_api_key.clone(), window, cx);
            });
        }
        if let Some(input) = self.title_model_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.title_model.clone(), window, cx);
            });
        }
        if let Some(input) = self.billing_email_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.billing_email.clone(), window, cx);
            });
        }
        if let Some(input) = self.billing_password_input.as_ref() {
            input.update(cx, |state, cx| {
                state.set_value(self.billing_password.clone(), window, cx);
            });
        }

        self.inputs_dirty = false;
        self.telemetry_hydrated = true;
        self.dictation_hydrated = true;
        self.title_hydrated = true;
        self.resource_hydrated = true;
        if self.editor_loaded {
            self.editor_hydrated = true;
        }
    }

    fn handle_input_event(
        &mut self,
        kind: SettingsInputKind,
        input: &Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                match kind {
                    SettingsInputKind::Search => {}
                    SettingsInputKind::EditorCustom => {
                        if self.editor_custom_command == value {
                            return;
                        }
                        self.editor_custom_command = value;
                        self.schedule_editor_save(cx);
                    }
                    SettingsInputKind::EditorRemote => {
                        if self.editor_remote_authority == value {
                            return;
                        }
                        self.editor_remote_authority = value;
                        self.schedule_editor_save(cx);
                    }
                    SettingsInputKind::AttachmentSource => {
                        if self.attachment_source == value {
                            return;
                        }
                        self.attachment_source = value;
                    }
                    SettingsInputKind::AttachmentName => {
                        if self.attachment_name == value {
                            return;
                        }
                        self.attachment_name = value;
                    }
                    SettingsInputKind::AttachmentRevision => {
                        if self.attachment_revision == value {
                            return;
                        }
                        self.attachment_revision = value;
                    }
                    SettingsInputKind::DocsSource => {
                        if self.docs_attachment_source == value {
                            return;
                        }
                        self.docs_attachment_source = value;
                    }
                    SettingsInputKind::DocsName => {
                        if self.docs_attachment_name == value {
                            return;
                        }
                        self.docs_attachment_name = value;
                    }
                    SettingsInputKind::ResourceCpu => {
                        if self.resource_cpu_quota_pct == value {
                            return;
                        }
                        self.resource_cpu_quota_pct = value;
                        self.schedule_resource_save(cx);
                    }
                    SettingsInputKind::ResourceMemHigh => {
                        if self.resource_memory_high_gb == value {
                            return;
                        }
                        self.resource_memory_high_gb = value;
                        self.schedule_resource_save(cx);
                    }
                    SettingsInputKind::ResourceMemMax => {
                        if self.resource_memory_max_gb == value {
                            return;
                        }
                        self.resource_memory_max_gb = value;
                        self.schedule_resource_save(cx);
                    }
                    SettingsInputKind::DictationBaseUrl => {
                        if self.dictation_base_url == value {
                            return;
                        }
                        self.dictation_base_url = value;
                        self.schedule_dictation_save(cx);
                    }
                    SettingsInputKind::DictationApiKey => {
                        if self.dictation_api_key == value {
                            return;
                        }
                        self.dictation_api_key = value;
                        self.schedule_dictation_save(cx);
                    }
                    SettingsInputKind::DictationApiSecret => {
                        if self.dictation_api_secret == value {
                            return;
                        }
                        self.dictation_api_secret = value;
                        self.schedule_dictation_save(cx);
                    }
                    SettingsInputKind::DictationLanguage => {
                        if self.dictation_language == value {
                            return;
                        }
                        self.dictation_language = value;
                        self.schedule_dictation_save(cx);
                    }
                    SettingsInputKind::TitleBaseUrl => {
                        if self.title_base_url == value {
                            return;
                        }
                        self.title_base_url = value;
                        self.schedule_title_save(cx);
                    }
                    SettingsInputKind::TitleApiKey => {
                        if self.title_api_key == value {
                            return;
                        }
                        self.title_api_key = value;
                        self.schedule_title_save(cx);
                    }
                    SettingsInputKind::TitleModel => {
                        if self.title_model == value {
                            return;
                        }
                        self.title_model = value;
                        self.schedule_title_save(cx);
                    }
                    SettingsInputKind::BillingEmail => {
                        if self.billing_email == value {
                            return;
                        }
                        self.billing_email = value;
                    }
                    SettingsInputKind::BillingPassword => {
                        if self.billing_password == value {
                            return;
                        }
                        self.billing_password = value;
                    }
                }
                cx.notify();
            }
            InputEvent::Focus | InputEvent::Blur => {
                if matches!(kind, SettingsInputKind::Search) {
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    fn handle_select_event(
        &mut self,
        kind: SettingsSelectKind,
        event: &SelectEvent<SearchableVec<LabeledOption>>,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(value) = event;
        let Some(value) = value.as_ref() else {
            return;
        };
        match kind {
            SettingsSelectKind::EditorTarget => {
                if self.editor_target == *value {
                    return;
                }
                self.editor_target = value.clone();
                self.schedule_editor_save(cx);
            }
            SettingsSelectKind::Workspace => {
                self.set_selected_workspace_by_value(value, cx);
            }
            SettingsSelectKind::ResourceMode => {
                let mode = match value.as_str() {
                    "custom" => ResourceGovernanceMode::Custom,
                    _ => ResourceGovernanceMode::Auto,
                };
                if self.resource_mode == mode {
                    return;
                }
                self.resource_mode = mode;
                self.schedule_resource_save(cx);
            }
            SettingsSelectKind::DictationModel => {
                if self.dictation_model == *value {
                    return;
                }
                self.dictation_model = value.clone();
                self.schedule_dictation_save(cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn ensure_input_state(
        slot: &mut Option<Entity<InputState>>,
        placeholder: &str,
        kind: SettingsInputKind,
        input_subscriptions: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let placeholder = placeholder.to_string();
        if let Some(state) = slot.as_ref() {
            let placeholder = placeholder.clone();
            state.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx);
            });
            return state.clone();
        }
        let input_placeholder = placeholder.clone();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(input_placeholder));
        let subscription = cx.subscribe_in(&input, window, move |view, state, event, _, cx| {
            view.handle_input_event(kind, state, event, cx);
        });
        input_subscriptions.push(subscription);
        *slot = Some(input.clone());
        input
    }

    pub(crate) fn ensure_select_state(
        slot: &mut Option<Entity<SelectState<SearchableVec<LabeledOption>>>>,
        items: Vec<LabeledOption>,
        selected: Option<String>,
        kind: SettingsSelectKind,
        input_subscriptions: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<LabeledOption>>> {
        let items = SearchableVec::from(items);
        if let Some(state) = slot.as_ref() {
            let selected = selected.clone();
            let items = items.clone();
            state.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                if let Some(value) = selected.as_ref() {
                    state.set_selected_value(value, window, cx);
                }
            });
            return state.clone();
        }
        let state = cx.new(|cx| SelectState::new(items, None, window, cx));
        if let Some(selected) = selected.as_ref() {
            state.update(cx, |state, cx| {
                state.set_selected_value(selected, window, cx);
            });
        }
        let subscription = cx.subscribe_in(&state, window, move |view, _, event, _, cx| {
            view.handle_select_event(kind, event, cx);
        });
        input_subscriptions.push(subscription);
        *slot = Some(state.clone());
        state
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
        self.refresh_workspace_attachments(cx);
        self.refresh_resource_utilization(cx);
        cx.notify();
    }

    fn set_selected_workspace_by_value(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some((index, _)) = self
            .workspaces
            .iter()
            .enumerate()
            .find(|(_, ws)| ws.id.0.to_string() == value)
        {
            self.select_workspace(index, cx);
        }
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

        let provider_id_for_task = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let options = client
                .get_provider_options(workspace_id, &provider_id_for_task)
                .await?;
            Ok(options)
        });

        let provider_id_for_update = provider_id.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.opts_busy.remove(&provider_id_for_update);
                match result {
                    Ok(options) => {
                        view.provider_options
                            .insert(provider_id_for_update.clone(), options);
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

        let provider_id_for_task = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            client
                .authenticate_provider_for_workspace(workspace_id, &provider_id_for_task, None)
                .await?;
            Ok(())
        });

        let provider_id_for_update = provider_id.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.auth_busy.remove(&provider_id_for_update);
                if let Err(err) = result {
                    view.provider_error = Some(err.to_string());
                }
                view.ensure_provider_options(provider_id_for_update.clone(), true, cx);
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

        let provider_id_for_task = provider_id.clone();
        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            client
                .verify_provider_for_workspace(workspace_id, &provider_id_for_task)
                .await?;
            Ok(())
        });

        let provider_id_for_update = provider_id.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.verify_busy.remove(&provider_id_for_update);
                if let Err(err) = result {
                    view.provider_error = Some(err.to_string());
                }
                view.ensure_provider_options(provider_id_for_update.clone(), true, cx);
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

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
        let installs = self
            .providers
            .iter()
            .filter_map(|provider| {
                let running = provider
                    .details
                    .get("install_running")
                    .map(|value| value == "true")
                    .unwrap_or(false);
                if !running {
                    return None;
                }
                let install_id = provider.details.get("install_id")?;
                if self.installs.contains_key(&provider.provider_id) {
                    return None;
                }
                Some((provider.provider_id.clone(), install_id.to_string()))
            })
            .collect::<Vec<_>>();

        for (provider_id, install_id) in installs {
            self.attach_install(provider_id, install_id, cx);
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
                state: InstallStateKind::Running,
                pct: None,
                last_stage: None,
                last_message: None,
                error: None,
            });
        cx.notify();

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
        let provider_ids = self
            .providers
            .iter()
            .map(|provider| provider.provider_id.clone())
            .collect::<Vec<_>>();
        for provider_id in provider_ids {
            self.ensure_provider_options(provider_id, false, cx);
        }
    }

    pub(crate) fn set_active_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.active_section == section {
            return;
        }
        self.active_section = section;
        if section == SettingsSection::WorkspaceAttachments {
            self.refresh_workspace_attachments(cx);
        }
        if section == SettingsSection::ResourceUtilization {
            self.start_resource_polling(cx);
        } else {
            self.resource_poll_token += 1;
        }
        if section == SettingsSection::MobileAccess {
            self.refresh_mobile_access_status(cx);
        }
        cx.notify();
    }

    fn start_resource_polling(&mut self, cx: &mut Context<Self>) {
        self.resource_poll_token += 1;
        let token = self.resource_poll_token;
        self.refresh_resource_utilization(cx);

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            loop {
                tokio::time::sleep(Duration::from_millis(3000)).await;
                let keep = this
                    .update(cx, |view, cx| {
                        if view.resource_poll_token != token
                            || view.active_section != SettingsSection::ResourceUtilization
                        {
                            return false;
                        }
                        view.refresh_resource_utilization(cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn save_settings_patch(
        &mut self,
        patch: UpdateSettingsRequest,
        cx: &mut Context<Self>,
    ) {
        self.save_seq += 1;
        let seq = self.save_seq;
        self.saving = true;
        self.save_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let settings = client.update_settings(&patch).await?;
            Ok(settings)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.save_seq != seq {
                    return;
                }
                view.saving = false;
                match result {
                    Ok(settings) => view.apply_settings_snapshot(settings),
                    Err(err) => view.save_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn refresh_workspace_attachments(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace else {
            self.attachments.clear();
            self.attachments_loading = false;
            return;
        };
        self.attachments_loading = true;
        self.attachments_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let attachments = client.list_workspace_attachments(workspace_id).await?;
            Ok(attachments)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.attachments_loading = false;
                match result {
                    Ok(attachments) => {
                        view.attachments = attachments;
                        view.attachment_source.clear();
                        view.attachment_name.clear();
                        view.attachment_revision.clear();
                        view.inputs_dirty = true;
                    }
                    Err(err) => view.attachments_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn sync_workspace_attachments(
        &mut self,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.attachment_sync_busy {
            return;
        }
        self.attachment_sync_busy = true;
        self.attachments_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let attachments = client
                .sync_workspace_attachments(workspace_id, refresh)
                .await?;
            Ok(attachments)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.attachment_sync_busy = false;
                match result {
                    Ok(attachments) => {
                        view.attachments = attachments;
                        view.docs_attachment_source.clear();
                        view.docs_attachment_name.clear();
                        view.inputs_dirty = true;
                    }
                    Err(err) => view.attachments_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn handle_add_attachment(&mut self, cx: &mut Context<Self>) {
        let source = self.attachment_source.trim().to_string();
        let revision = self.attachment_revision.trim().to_string();
        let mut name = self.attachment_name.trim().to_string();
        if source.is_empty() {
            self.attachments_error = Some("Repository URL is required.".to_string());
            cx.notify();
            return;
        }
        if name.is_empty() {
            name = guess_attachment_name(&source);
        }
        if name.is_empty() {
            self.attachments_error = Some("Attachment name is required.".to_string());
            cx.notify();
            return;
        }
        let req = CreateWorkspaceAttachmentRequest {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name,
            source,
            revision: if revision.is_empty() { None } else { Some(revision) },
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        };
        self.add_workspace_attachment(req, cx);
    }

    pub(crate) fn handle_add_docs_attachment(&mut self, cx: &mut Context<Self>) {
        let source = self.docs_attachment_source.trim().to_string();
        let mut name = self.docs_attachment_name.trim().to_string();
        if source.is_empty() {
            self.attachments_error = Some("Docs URL is required.".to_string());
            cx.notify();
            return;
        }
        if name.is_empty() {
            name = guess_attachment_name(&source);
        }
        if name.is_empty() {
            self.attachments_error = Some("Attachment name is required.".to_string());
            cx.notify();
            return;
        }
        let req = CreateWorkspaceAttachmentRequest {
            kind: WorkspaceAttachmentKind::DocMirror,
            name,
            source,
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        };
        self.add_docs_attachment(req, cx);
    }

    pub(crate) fn add_workspace_attachment(
        &mut self,
        req: CreateWorkspaceAttachmentRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.attachment_busy {
            return;
        }
        self.attachment_busy = true;
        self.attachments_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let attachments = client
                .create_workspace_attachment(workspace_id, &req)
                .await?;
            Ok(attachments)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.attachment_busy = false;
                match result {
                    Ok(attachments) => view.attachments = attachments,
                    Err(err) => view.attachments_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn add_docs_attachment(
        &mut self,
        req: CreateWorkspaceAttachmentRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self.docs_attachment_busy {
            return;
        }
        self.docs_attachment_busy = true;
        self.attachments_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let attachments = client
                .create_workspace_attachment(workspace_id, &req)
                .await?;
            Ok(attachments)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.docs_attachment_busy = false;
                match result {
                    Ok(attachments) => view.attachments = attachments,
                    Err(err) => view.attachments_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn remove_workspace_attachment(
        &mut self,
        id: String,
        req: DeleteWorkspaceAttachmentRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self.selected_workspace else {
            return;
        };
        if self
            .attachment_delete_busy
            .get(&id)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        self.attachment_delete_busy.insert(id.clone(), true);
        self.attachments_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let attachments = client
                .delete_workspace_attachment(workspace_id, &req)
                .await?;
            Ok(attachments)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.attachment_delete_busy.remove(&id);
                match result {
                    Ok(attachments) => view.attachments = attachments,
                    Err(err) => view.attachments_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn refresh_resource_utilization(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace else {
            self.resource_snapshot = None;
            self.resource_loading = false;
            self.resource_error = None;
            return;
        };
        self.resource_loading = true;
        self.resource_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let snapshot = client.get_resource_utilization(workspace_id).await?;
            Ok(snapshot)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.resource_loading = false;
                match result {
                    Ok(snapshot) => view.resource_snapshot = Some(snapshot),
                    Err(err) => view.resource_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn refresh_mobile_access_status(&mut self, cx: &mut Context<Self>) {
        self.mobile_status_loading = true;
        self.mobile_status_error = None;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let status = client.get_mobile_access_status().await?;
            Ok(status)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.mobile_status_loading = false;
                match result {
                    Ok(status) => view.mobile_status = Some(status),
                    Err(err) => view.mobile_status_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn enable_mobile_access(
        &mut self,
        supabase_token: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(token) = supabase_token else {
            self.mobile_enable_error = Some("Sign in required to manage mobile access.".to_string());
            cx.notify();
            return;
        };
        self.mobile_enable_error = None;
        self.mobile_enable_busy = true;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            let resp = client.enable_mobile_access(&token).await?;
            Ok(resp)
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.mobile_enable_busy = false;
                match result {
                    Ok(resp) => {
                        view.mobile_status = Some(resp.status.clone());
                        view.mobile_qr = Some(resp);
                    }
                    Err(err) => view.mobile_enable_error = Some(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn disable_mobile_access(
        &mut self,
        supabase_token: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(token) = supabase_token else {
            self.mobile_enable_error = Some("Sign in required to manage mobile access.".to_string());
            cx.notify();
            return;
        };
        self.mobile_enable_error = None;
        self.mobile_enable_busy = true;
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let client = Client::from_env()?;
            client.disable_mobile_access(&token).await?;
            Ok(())
        });

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            let result = task.await;
            this.update(cx, |view, cx| {
                view.mobile_enable_busy = false;
                if let Err(err) = result {
                    view.mobile_enable_error = Some(err.to_string());
                }
                view.mobile_qr = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

}

async fn load_editor_settings() -> Result<DesktopEditorSettings> {
    let path = editor_settings_path()?;
    let data = match fs::read_to_string(&path).await {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DesktopEditorSettings::default())
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading editor settings {}", path.display())
            })
        }
    };

    let parsed = serde_json::from_str::<DesktopEditorSettings>(&data).unwrap_or_default();
    Ok(parsed)
}

async fn save_editor_settings(settings: &DesktopEditorSettings) -> Result<DesktopEditorSettings> {
    let path = editor_settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.ok();
    }
    let tmp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings)?;
    fs::write(&tmp_path, bytes).await?;
    fs::rename(&tmp_path, &path).await?;
    Ok(settings.clone())
}

fn editor_settings_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("rs", "ctx", "ctx").context("resolving project dirs")?;
    Ok(dirs.data_dir().join("desktop-settings.json"))
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


fn normalize_dictation_model(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "auto" {
        return "auto".to_string();
    }
    match trimmed {
        "elevenlabs/scribe-v2-realtime" => "elevenlabs/scribe_v2_realtime".to_string(),
        "deepgram/flux" => "deepgram/flux-general".to_string(),
        other => other.to_string(),
    }
}

fn guess_attachment_name(source: &str) -> String {
    let mut cleaned = source.trim().to_string();
    while cleaned.ends_with('/') || cleaned.ends_with('\\') {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        return String::new();
    }
    let slash_idx = cleaned
        .rfind('/')
        .or_else(|| cleaned.rfind('\\'))
        .or_else(|| cleaned.rfind(':'));
    let mut name = slash_idx
        .map(|idx| cleaned[idx + 1..].to_string())
        .unwrap_or(cleaned);
    if name.ends_with(".git") {
        name.truncate(name.len().saturating_sub(4));
    }
    name
}

fn format_gib(mb: Option<u32>) -> String {
    let Some(mb) = mb else {
        return String::new();
    };
    if mb == 0 {
        return String::new();
    }
    let gb = mb as f64 / 1024.0;
    if gb >= 10.0 {
        format!("{:.0}", gb)
    } else {
        format!("{:.1}", gb)
    }
}

fn parse_gib(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = trimmed.parse::<f64>().ok()?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return None;
    }
    Some((parsed * 1024.0).round() as u32)
}
