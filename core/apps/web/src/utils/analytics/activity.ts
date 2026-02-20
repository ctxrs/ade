import { captureProductEvent } from "./client";
import type { AnalyticsEnvTarget, AnalyticsProperties } from "./types";

const FIRST_TURN_SUBMITTED_ONCE_KEY = "ctx.analytics.first_turn_submitted.install_once.v1";
const FIRST_TURN_COMPLETED_ONCE_KEY = "ctx.analytics.first_turn_completed.install_once.v1";

const markOnce = (key: string): boolean => {
  if (typeof window === "undefined") return true;
  try {
    if (window.localStorage.getItem(key)) return false;
    window.localStorage.setItem(key, "1");
    return true;
  } catch {
    return true;
  }
};

const capture = (eventName: string, properties: AnalyticsProperties): boolean => {
  return captureProductEvent(eventName, 1, properties);
};

export const trackAppOpened = (props?: { downloadId?: string }): void => {
  capture("app_opened", {
    ...(props?.downloadId ? { download_id: props.downloadId } : {}),
  });
};

export const trackWorkspaceCreated = (workspaceKind: "local" | "remote"): void => {
  capture("workspace_created", { workspace_kind: workspaceKind });
};

export const trackWorkspaceCreateSubmitted = (props: {
  workspaceKind: "local" | "remote";
  source: "wizard" | "launcher" | "api" | "unknown";
}): void => {
  capture("workspace_create_submitted", {
    workspace_kind: props.workspaceKind,
    source: props.source,
  });
};

export const trackWorkspaceCreateSucceeded = (props: {
  workspaceKind: "local" | "remote";
  source: "wizard" | "launcher" | "api" | "unknown";
}): void => {
  capture("workspace_create_succeeded", {
    workspace_kind: props.workspaceKind,
    source: props.source,
  });
};

export const trackWorkspaceCreateFailed = (props: {
  workspaceKind: "local" | "remote";
  source: "wizard" | "launcher" | "api" | "unknown";
  failureKind: "network_error" | "request_error" | "unknown";
}): void => {
  capture("workspace_create_failed", {
    workspace_kind: props.workspaceKind,
    source: props.source,
    failure_kind: props.failureKind,
  });
};

export const trackWorkspaceOpened = (workspaceKind: "local" | "remote"): void => {
  capture("workspace_opened", { workspace_kind: workspaceKind });
};

export const trackWizardStarted = (props: {
  wizardKey: "workspace_setup";
}): void => {
  capture("wizard_started", {
    wizard_key: props.wizardKey,
  });
};

export const trackWizardStepViewed = (props: {
  wizardKey: "workspace_setup";
  stepKey: string;
  stepIndex: number;
}): void => {
  capture("wizard_step_viewed", {
    wizard_key: props.wizardKey,
    step_key: props.stepKey,
    step_index: props.stepIndex,
  });
};

export const trackWizardStepCompleted = (props: {
  wizardKey: "workspace_setup";
  stepKey: string;
  stepIndex: number;
}): void => {
  capture("wizard_step_completed", {
    wizard_key: props.wizardKey,
    step_key: props.stepKey,
    step_index: props.stepIndex,
  });
};

export const trackWizardCompleted = (props: {
  wizardKey: "workspace_setup";
  workspaceKind: "local" | "remote" | "unknown";
}): void => {
  capture("wizard_completed", {
    wizard_key: props.wizardKey,
    workspace_kind: props.workspaceKind,
  });
};

export const trackWizardAbandoned = (props: {
  wizardKey: "workspace_setup";
  lastStepKey: string;
  lastStepIndex: number;
}): void => {
  capture("wizard_abandoned", {
    wizard_key: props.wizardKey,
    last_step_key: props.lastStepKey,
    last_step_index: props.lastStepIndex,
  });
};

export const trackSessionCreated = (props: {
  providerId: string;
  modelId?: string;
  envTarget?: AnalyticsEnvTarget;
}): void => {
  capture("session_created", {
    provider_id: props.providerId,
    ...(props.modelId ? { model_id: props.modelId } : {}),
    ...(props.envTarget ? { env_target: props.envTarget } : {}),
  });
};

export const trackProviderSelected = (props: {
  providerId: string;
  source: "session_create" | "provider_switch" | "unknown";
}): void => {
  capture("provider_selected", {
    provider_id: props.providerId,
    source: props.source,
  });
};

export const trackFirstTurnSubmitted = (props: {
  sessionId: string;
  providerId?: string;
  modelId?: string;
}): void => {
  if (!markOnce(FIRST_TURN_SUBMITTED_ONCE_KEY)) return;
  capture("first_turn_submitted", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...(props.modelId ? { model_id: props.modelId } : {}),
  });
};

export const trackProviderRunCompleted = (props: {
  providerId?: string;
  modelId?: string;
  status: "completed" | "failed" | "interrupted";
}): void => {
  capture("provider_run_completed", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...(props.modelId ? { model_id: props.modelId } : {}),
    status: props.status,
    duration_bucket: "unknown",
  });
};

export const trackFirstTurnCompleted = (props: {
  sessionId: string;
  providerId?: string;
  status: "completed" | "failed" | "interrupted";
}): void => {
  if (props.status !== "completed") return;
  if (!markOnce(FIRST_TURN_COMPLETED_ONCE_KEY)) return;
  capture("first_turn_completed", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    status: props.status,
  });
};

export const trackFeatureUsed = (
  featureKey: string,
  extra: AnalyticsProperties = {},
): void => {
  capture("feature_used", { feature_key: featureKey, ...extra });
};

export const trackWorkbenchPanelToggled = (props: {
  panelKey: "terminal" | "diff" | "artifacts" | "sessions";
  open: boolean;
  source: "header_button" | "menu_command" | "unknown";
}): void => {
  capture("workbench_panel_toggled", {
    panel_key: props.panelKey,
    open: props.open,
    source: props.source,
  });
};

export const trackPlanViewed = (entrySurface: string): void => {
  capture("plan_viewed", { entry_surface: entrySurface });
};

export const trackSubscribeCtaClicked = (planTarget: "month" | "year"): void => {
  capture("subscribe_cta_clicked", { plan_target: planTarget });
};

export const trackCheckoutStarted = (planTarget: "month" | "year"): void => {
  capture("checkout_started", { plan_target: planTarget });
};

export const trackEntitlementActivated = (planType: string): void => {
  capture("entitlement_activated", { plan_type: planType });
};

export const trackFeatureGateEvaluated = (props: {
  gateKey: string;
  result: boolean;
  reason: "override" | "posthog" | "fallback";
}): boolean => {
  return capture("feature_gate_evaluated", {
    gate_key: props.gateKey,
    result: props.result ? "enabled" : "disabled",
    reason: props.reason,
  });
};

export const trackExperimentExposure = (props: {
  experimentKey: string;
  variant: string;
  assignmentUnit: "install_id" | "account_id";
}): boolean => {
  return capture("experiment_exposure", {
    experiment_key: props.experimentKey,
    variant: props.variant,
    assignment_unit: props.assignmentUnit,
  });
};

export const trackRuntimeErrorObserved = (props: {
  errorKey: string;
  severity: "warning" | "error";
  signature: string;
}): void => {
  capture("runtime_error_observed", {
    error_key: props.errorKey,
    severity: props.severity,
    error_signature: props.signature,
  });
};

export const trackSessionLoadFatalObserved = (props: {
  mode: string;
  signature: string;
}): void => {
  capture("session_load_fatal_observed", {
    mode: props.mode,
    error_signature: props.signature,
  });
};

export const trackApiErrorObserved = (props: {
  errorKey: string;
  endpoint: string;
  method: string;
  statusFamily: "2xx" | "3xx" | "4xx" | "5xx" | "none";
  signature: string;
}): void => {
  capture("api_error_observed", {
    error_key: props.errorKey,
    api_endpoint: props.endpoint,
    method: props.method,
    status_family: props.statusFamily,
    error_signature: props.signature,
  });
};
