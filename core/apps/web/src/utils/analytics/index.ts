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
  trackWorkspaceOpened,
  trackWizardStarted,
  trackWizardStepViewed,
  trackWizardStepCompleted,
  trackWizardCompleted,
  trackWizardAbandoned,
  trackSessionCreated,
  trackProviderSelected,
  trackFirstTurnSubmitted,
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
