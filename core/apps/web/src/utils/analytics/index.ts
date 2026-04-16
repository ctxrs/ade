export {
  initAnalytics,
  setAnalyticsEnabled,
  captureAnalyticsEvent,
  captureProductEvent,
} from "./client";

export { getFeatureGate, useFeatureGate } from "./featureGates";

export {
  trackAppOpened,
  trackWorkspaceCreated,
  trackWorkspaceCreateSubmitted,
  trackWorkspaceCreateSucceeded,
  trackWorkspaceCreateFailed,
  trackWorkspaceLaunchCompleted,
  trackWorkspaceOpened,
  trackWorkspaceRouteOpenedFromPending,
  trackWizardStarted,
  trackWizardStepViewed,
  trackWizardStepCompleted,
  trackWizardCompleted,
  trackWizardAbandoned,
  trackSessionCreated,
  trackTaskCreated,
  trackProviderSelected,
  trackFirstTurnSubmitted,
  trackUserMessageSent,
  trackTurnStarted,
  trackTurnCompleted,
  trackFirstTurnCompleted,
  trackProviderRunCompleted,
  trackFeatureUsed,
  trackWorkbenchPanelToggled,
  trackPlanViewed,
  trackSubscribeCtaClicked,
  trackCheckoutStarted,
  trackEntitlementActivated,
  trackFeatureGateEvaluated,
  trackExperimentExposure,
  trackRuntimeErrorObserved,
  trackSessionLoadFatalObserved,
  trackApiErrorObserved,
  trackForegroundFreshnessSlaMissed,
  trackForegroundBacklogObserved,
  trackForegroundGapRecoveryObserved,
} from "./activity";

export { sanitizeAnalyticsProperties } from "./schema";

export {
  normalizeDownloadAttributionId,
  createDownloadAttributionId,
  appendDownloadAttributionIdToUrl,
  setPendingDownloadAttributionId,
  getPendingDownloadAttributionId,
  clearPendingDownloadAttributionId,
  consumePendingDownloadAttributionId,
} from "./downloadAttribution";
