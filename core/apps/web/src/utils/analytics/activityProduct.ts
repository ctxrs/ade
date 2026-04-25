import type { ExecutionEnvironment } from "@ctx/types";
import type {
  AnalyticsProperties,
  AnalyticsSessionKind,
  AnalyticsSessionLocation,
  AnalyticsSessionRootKind,
} from "./types";
import {
  capture,
  FIRST_TURN_COMPLETED_ONCE_KEY,
  FIRST_TURN_SUBMITTED_ONCE_KEY,
  durationBucketForMs,
  markOnce,
  modelAnalyticsProperties,
  tokenUsageProperties,
} from "./activityShared";

export const trackSessionCreated = (props: {
  providerId: string;
  modelId?: string;
  executionEnvironment?: ExecutionEnvironment;
  sessionRootKind?: AnalyticsSessionRootKind;
  sessionLocation?: AnalyticsSessionLocation;
}): void => {
  capture("session_created", {
    provider_id: props.providerId,
    ...(props.modelId ? { model_id: props.modelId } : {}),
    ...(props.executionEnvironment ? { execution_environment: props.executionEnvironment } : {}),
    ...(props.sessionRootKind ? { session_root_kind: props.sessionRootKind } : {}),
    ...(props.sessionLocation ? { session_location: props.sessionLocation } : {}),
  });
};

export const trackTaskCreated = (props: {
  providerId: string;
  modelId?: string;
  reasoningEffort?: string | null;
  executionEnvironment?: ExecutionEnvironment;
}): void => {
  capture("task_created", {
    provider_id: props.providerId,
    ...modelAnalyticsProperties(props.modelId, props.reasoningEffort),
    ...(props.executionEnvironment ? { execution_environment: props.executionEnvironment } : {}),
    session_kind: "primary",
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

export const trackUserMessageSent = (props: {
  providerId?: string;
  modelId?: string;
  reasoningEffort?: string | null;
  executionEnvironment?: ExecutionEnvironment;
  sessionKind?: AnalyticsSessionKind;
  isFirstTurn?: boolean;
}): void => {
  capture("user_message_sent", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...modelAnalyticsProperties(props.modelId, props.reasoningEffort),
    ...(props.executionEnvironment ? { execution_environment: props.executionEnvironment } : {}),
    ...(props.sessionKind ? { session_kind: props.sessionKind } : {}),
    ...(props.isFirstTurn !== undefined ? { is_first_turn: props.isFirstTurn } : {}),
  });
};

export const trackTurnStarted = (props: {
  providerId?: string;
  modelId?: string;
  reasoningEffort?: string | null;
  executionEnvironment?: ExecutionEnvironment;
  sessionKind?: AnalyticsSessionKind;
}): void => {
  capture("turn_started", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...modelAnalyticsProperties(props.modelId, props.reasoningEffort),
    ...(props.executionEnvironment ? { execution_environment: props.executionEnvironment } : {}),
    ...(props.sessionKind ? { session_kind: props.sessionKind } : {}),
  });
};

export const trackProviderRunCompleted = (props: {
  providerId?: string;
  modelId?: string;
  status: "completed" | "failed" | "interrupted";
  durationMs?: number;
  sessionKind?: AnalyticsSessionKind;
}): void => {
  capture("provider_run_completed", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...(props.modelId ? { model_id: props.modelId } : {}),
    status: props.status,
    duration_bucket: durationBucketForMs(props.durationMs),
    ...(props.sessionKind ? { session_kind: props.sessionKind } : {}),
  });
};

export const trackTurnCompleted = (props: {
  providerId?: string;
  modelId?: string;
  reasoningEffort?: string | null;
  executionEnvironment?: ExecutionEnvironment;
  status: "completed" | "failed" | "interrupted";
  durationMs?: number;
  sessionKind?: AnalyticsSessionKind;
  metrics?: unknown;
}): void => {
  capture("turn_completed", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    ...modelAnalyticsProperties(props.modelId, props.reasoningEffort),
    ...(props.executionEnvironment ? { execution_environment: props.executionEnvironment } : {}),
    status: props.status,
    duration_bucket: durationBucketForMs(props.durationMs),
    ...(props.sessionKind ? { session_kind: props.sessionKind } : {}),
    ...tokenUsageProperties(props.metrics),
  });
};

export const trackFirstTurnCompleted = (props: {
  sessionId: string;
  providerId?: string;
  status: "completed" | "failed" | "interrupted";
  sessionKind?: AnalyticsSessionKind;
}): void => {
  if (props.status !== "completed") return;
  if (!markOnce(FIRST_TURN_COMPLETED_ONCE_KEY)) return;
  capture("first_turn_completed", {
    ...(props.providerId ? { provider_id: props.providerId } : {}),
    status: props.status,
    ...(props.sessionKind ? { session_kind: props.sessionKind } : {}),
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
