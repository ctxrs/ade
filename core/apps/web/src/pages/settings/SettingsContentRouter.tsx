import type { Dispatch, SetStateAction } from "react";
import type { DesktopEditorSettings } from "../../utils/desktop";
import type {
  DevRestartProvidersResult,
  EnableMobileAccessResponse,
  MobileAccessStatus,
  ResourceGovernanceLimits,
  ResourceGovernanceSettings,
  ResourceGovernanceStatus,
  ResourceUtilization,
  SandboxingSettings,
  Workspace,
} from "../../api/client";
import type { User } from "@supabase/supabase-js";
import type { ClientSettingsState } from "../../state/clientSettings";
import { GeneralSettingsSection } from "./sections/GeneralSettingsSection";
import { NotificationsSettingsSection } from "./sections/NotificationsSettingsSection";
import { AnalyticsSettingsSection } from "./sections/AnalyticsSettingsSection";
import { WorktreeBootstrapSection } from "./sections/WorktreeBootstrapSection";
import { AgentSystemPromptSection } from "./sections/AgentSystemPromptSection";
import { WorkspaceAttachmentsSection } from "./sections/WorkspaceAttachmentsSection";
import { ContainerNetworkSection } from "./sections/ContainerNetworkSection";
import { MergeQueueSection } from "./sections/MergeQueueSection";
import { ResourceGovernanceSection } from "./sections/ResourceGovernanceSection";
import { MobileAccessSection } from "./sections/MobileAccessSection";
import { ResourceUtilizationSection } from "./sections/ResourceUtilizationSection";
import { DictationSection } from "./sections/DictationSection";
import { TitleGenerationSection } from "./sections/TitleGenerationSection";
import { BillingSection } from "./sections/BillingSection";
import { HarnessAuthenticationSection } from "./sections/HarnessAuthenticationSection";
import { CodexAccountsSection } from "./sections/CodexAccountsSection";
import { DevToolsSection } from "./sections/DevToolsSection";
import { SandboxingSection } from "./sections/SandboxingSection";
import type { SectionId } from "../SettingsPage.types";

export function SettingsContentRouter(props: {
  active: SectionId;
  loaded: boolean;
  loadError: string | null;
  theme: "system" | "light" | "dark";
  onThemeChange: (next: "system" | "light" | "dark") => void;
  editorSettings: DesktopEditorSettings;
  setEditorSettings: Dispatch<SetStateAction<DesktopEditorSettings>>;
  editorLoaded: boolean;
  editorError: string | null;
  clientSettingsError: string | null;
  showRemoteAuthority: boolean;
  isDesktopApp: () => boolean;
  desktopTurnNotifications: boolean;
  clientSettingsState: ClientSettingsState;
  clientSettingsSaving: boolean;
  onToggleTurnNotifications: (next: boolean) => void | Promise<void>;
  telemetryEnabled: boolean;
  setTelemetryEnabled: (next: boolean) => void;
  workspaceId: string | null;
  themeVariant: "light" | "dark";
  resourceGovernance: {
    enabled: boolean;
    setEnabled: (value: boolean) => void;
    mode: ResourceGovernanceSettings["mode"];
    setMode: (value: ResourceGovernanceSettings["mode"]) => void;
    cpuQuotaPct: string;
    setCpuQuotaPct: (value: string) => void;
    memoryHighGb: string;
    setMemoryHighGb: (value: string) => void;
    memoryMaxGb: string;
    setMemoryMaxGb: (value: string) => void;
    effective: ResourceGovernanceLimits | null;
    status: ResourceGovernanceStatus | null;
    canSave: boolean;
    payload: ResourceGovernanceSettings;
    onApplyNow: (payload: ResourceGovernanceSettings) => void | Promise<void>;
  };
  saving: boolean;
  supabaseConfigured: boolean;
  billing: {
    checkoutStatus: string | null;
    billingUser: User | null;
    billingEmail: string;
    setBillingEmail: (value: string) => void;
    billingPassword: string;
    setBillingPassword: (value: string) => void;
    billingBusy: boolean;
    billingError: string | null;
    entitlementsBusy: boolean;
    plan: "free_local" | "pro" | "team" | "enterprise";
    proEnabled: boolean;
    onSignIn: () => void | Promise<void>;
    onSignUp: () => void | Promise<void>;
    onSignOut: () => void | Promise<void>;
    onStartCheckout: (interval: "month" | "year") => void | Promise<void>;
    onOpenPortal: () => void | Promise<void>;
  };
  mobileAccess: {
    billingUser: User | null;
    entitlementsBusy: boolean;
    proEnabled: boolean;
    mobileStatus: MobileAccessStatus | null;
    mobileStatusBusy: boolean;
    mobileStatusError: string | null;
    mobileEnableBusy: boolean;
    mobileEnableError: string | null;
    mobileQr: EnableMobileAccessResponse | null;
    qrFgColor: string;
    onEnable: () => void | Promise<void>;
    onDisable: () => void | Promise<void>;
  };
  resourceUtilization: {
    workspaces: Workspace[];
    snapshot: ResourceUtilization | null;
    loading: boolean;
    error: string | null;
    expandedProcessPids: Record<number, boolean>;
    onToggleExpanded: (pid: number) => void;
  };
  providerControlMode: SandboxingSettings["provider_control_mode"];
  setProviderControlMode: (value: SandboxingSettings["provider_control_mode"]) => void;
  devTools: {
    enabled: boolean;
    restartBusy: boolean;
    restartError: string | null;
    restartResults: DevRestartProvidersResult[] | null;
    onRestart: (mode: "drain" | "immediate") => void | Promise<void>;
  };
}) {
  const {
    active,
    loaded,
    loadError,
    theme,
    onThemeChange,
    editorSettings,
    setEditorSettings,
    editorLoaded,
    editorError,
    clientSettingsError,
    showRemoteAuthority,
    isDesktopApp,
    desktopTurnNotifications,
    clientSettingsState,
    clientSettingsSaving,
    onToggleTurnNotifications,
    telemetryEnabled,
    setTelemetryEnabled,
    workspaceId,
    themeVariant,
    resourceGovernance,
    saving,
    supabaseConfigured,
    billing,
    mobileAccess,
    resourceUtilization,
    providerControlMode,
    setProviderControlMode,
    devTools,
  } = props;

  if (!loaded) return <div className="settings-empty">Loading…</div>;
  if (loadError) return <div className="settings-empty settings-empty-error">{loadError}</div>;

  if (active === "general") {
    return (
      <GeneralSettingsSection
        theme={theme}
        onThemeChange={onThemeChange}
        editorSettings={editorSettings}
        setEditorSettings={setEditorSettings}
        editorLoaded={editorLoaded}
        editorError={editorError}
        clientSettingsError={clientSettingsError}
        showRemoteAuthority={showRemoteAuthority}
        isDesktopApp={isDesktopApp}
      />
    );
  }

  if (active === "notifications") {
    return (
      <NotificationsSettingsSection
        isDesktopApp={isDesktopApp}
        desktopTurnNotifications={desktopTurnNotifications}
        clientSettingsState={clientSettingsState}
        clientSettingsSaving={clientSettingsSaving}
        clientSettingsError={clientSettingsError}
        onToggleTurnNotifications={async (next) => {
          await onToggleTurnNotifications(next);
        }}
      />
    );
  }

  if (active === "analytics") {
    return (
      <AnalyticsSettingsSection
        telemetryEnabled={telemetryEnabled}
        loaded={loaded}
        setTelemetryEnabled={setTelemetryEnabled}
      />
    );
  }

  if (active === "worktree_bootstrap") {
    return <WorktreeBootstrapSection workspaceId={workspaceId} active />;
  }

  if (active === "agent_system_prompt") {
    return <AgentSystemPromptSection workspaceId={workspaceId} active themeVariant={themeVariant} />;
  }

  if (active === "workspace_attachments") {
    return <WorkspaceAttachmentsSection workspaceId={workspaceId} active />;
  }

  if (active === "container_network") {
    return <ContainerNetworkSection workspaceId={workspaceId} active themeVariant={themeVariant} />;
  }

  if (active === "merge_queue") {
    return <MergeQueueSection workspaceId={workspaceId} active />;
  }

  if (active === "resource_governance") {
    return (
      <ResourceGovernanceSection
        loaded={loaded}
        enabled={resourceGovernance.enabled}
        onEnabledChange={resourceGovernance.setEnabled}
        mode={resourceGovernance.mode}
        onModeChange={resourceGovernance.setMode}
        cpuQuotaPct={resourceGovernance.cpuQuotaPct}
        onCpuQuotaPctChange={resourceGovernance.setCpuQuotaPct}
        memoryHighGb={resourceGovernance.memoryHighGb}
        onMemoryHighGbChange={resourceGovernance.setMemoryHighGb}
        memoryMaxGb={resourceGovernance.memoryMaxGb}
        onMemoryMaxGbChange={resourceGovernance.setMemoryMaxGb}
        effective={resourceGovernance.effective}
        status={resourceGovernance.status}
        saving={saving}
        canSave={resourceGovernance.canSave}
        payload={resourceGovernance.payload}
        onApplyNow={resourceGovernance.onApplyNow}
      />
    );
  }

  if (active === "mobile_access") {
    return (
      <MobileAccessSection
        supabaseConfigured={supabaseConfigured}
        billingUser={mobileAccess.billingUser}
        entitlementsBusy={mobileAccess.entitlementsBusy}
        proEnabled={mobileAccess.proEnabled}
        mobileStatus={mobileAccess.mobileStatus}
        mobileStatusBusy={mobileAccess.mobileStatusBusy}
        mobileStatusError={mobileAccess.mobileStatusError}
        mobileEnableBusy={mobileAccess.mobileEnableBusy}
        mobileEnableError={mobileAccess.mobileEnableError}
        mobileQr={mobileAccess.mobileQr}
        qrFgColor={mobileAccess.qrFgColor}
        onEnable={mobileAccess.onEnable}
        onDisable={mobileAccess.onDisable}
      />
    );
  }

  if (active === "resource_utilization") {
    return (
      <ResourceUtilizationSection
        workspaceId={workspaceId}
        workspaces={resourceUtilization.workspaces}
        resourceSnapshot={resourceUtilization.snapshot}
        resourceLoading={resourceUtilization.loading}
        resourceError={resourceUtilization.error}
        expandedProcessPids={resourceUtilization.expandedProcessPids}
        onToggleExpanded={resourceUtilization.onToggleExpanded}
      />
    );
  }

  if (active === "dictation") return <DictationSection active />;
  if (active === "title_generation") return <TitleGenerationSection active />;

  if (active === "billing") {
    return (
      <BillingSection
        supabaseConfigured={supabaseConfigured}
        checkoutStatus={billing.checkoutStatus}
        billingUser={billing.billingUser}
        billingEmail={billing.billingEmail}
        onBillingEmailChange={billing.setBillingEmail}
        billingPassword={billing.billingPassword}
        onBillingPasswordChange={billing.setBillingPassword}
        billingBusy={billing.billingBusy}
        billingError={billing.billingError}
        entitlementsBusy={billing.entitlementsBusy}
        plan={billing.plan}
        proEnabled={billing.proEnabled}
        onSignIn={billing.onSignIn}
        onSignUp={billing.onSignUp}
        onSignOut={billing.onSignOut}
        onStartCheckout={billing.onStartCheckout}
        onOpenPortal={billing.onOpenPortal}
      />
    );
  }

  if (active === "agent_harnesses") {
    return <HarnessAuthenticationSection workspaceId={workspaceId} active />;
  }

  if (active === "harness_subscriptions") {
    return <CodexAccountsSection active />;
  }

  if (active === "dev_tools") {
    return (
      <DevToolsSection
        devToolsEnabled={devTools.enabled}
        devRestartBusy={devTools.restartBusy}
        devRestartError={devTools.restartError}
        devRestartResults={devTools.restartResults}
        onRestart={devTools.onRestart}
      />
    );
  }

  if (active === "sandboxing") {
    return (
      <SandboxingSection
        loaded={loaded}
        providerControlMode={providerControlMode}
        onProviderControlModeChange={setProviderControlMode}
      />
    );
  }

  if (active === "models_routing" || active === "context_pack" || active === "team_enterprise" || active === "usage_analytics") {
    return null;
  }

  return <div className="settings-empty">No settings yet.</div>;
}
