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

export const trackAppOpened = (): void => {
  capture("app_opened", {});
};

export const trackWorkspaceCreated = (workspaceKind: "local" | "remote"): void => {
  capture("workspace_created", { workspace_kind: workspaceKind });
};

export const trackWorkspaceOpened = (workspaceKind: "local" | "remote"): void => {
  capture("workspace_opened", { workspace_kind: workspaceKind });
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
