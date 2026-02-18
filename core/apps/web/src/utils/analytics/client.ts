import posthog from "posthog-js";
import { getPostHogHost, getPostHogKey, getPostHogProjectId, getPostHogUiHost } from "./config";
import { getInstallId } from "./identity";
import { buildEventEnvelope } from "./context";
import { sanitizeAnalyticsProperties } from "./schema";
import type { AnalyticsProperties } from "./types";

type PendingCapture = {
  eventName: string;
  properties: AnalyticsProperties;
};

type FeatureFlagListener = () => void;

type FeatureFlagOverrides = Record<string, boolean>;
type FeatureFlagGlobals = {
  __CTX_FEATURE_FLAGS__?: unknown;
};

type PostHogFlagMethods = {
  onFeatureFlags?: (callback: () => void) => void;
  reloadFeatureFlags?: () => void;
};

const asFlagMethods = (): PostHogFlagMethods =>
  posthog as unknown as PostHogFlagMethods;

export const MAX_PENDING_CAPTURES = 512;

const pendingCaptures: PendingCapture[] = [];
const featureFlagListeners = new Set<FeatureFlagListener>();

let initAttempted = false;
let initResolved = false;
let captureEnabled = false;

const readFeatureOverrides = (): FeatureFlagOverrides | null => {
  if (typeof globalThis === "undefined") return null;
  const raw = (globalThis as typeof globalThis & FeatureFlagGlobals).__CTX_FEATURE_FLAGS__;
  if (!raw || typeof raw !== "object") return null;
  return raw as FeatureFlagOverrides;
};

const notifyFeatureFlags = () => {
  for (const listener of featureFlagListeners) listener();
};

const flushPending = () => {
  if (!initResolved || !captureEnabled) return;
  while (pendingCaptures.length > 0) {
    const next = pendingCaptures.shift();
    if (!next) break;
    posthog.capture(next.eventName, next.properties);
  }
};

export const initAnalytics = (): void => {
  if (initAttempted) return;
  initAttempted = true;

  if (typeof window === "undefined") return;
  const key = getPostHogKey().trim();
  if (!key) return;

  posthog.init(key, {
    api_host: getPostHogHost(),
    ui_host: getPostHogUiHost(),
    person_profiles: "identified_only",
    autocapture: false,
    capture_pageview: false,
    capture_pageleave: false,
    opt_out_capturing_by_default: !captureEnabled,
    loaded: () => {
      initResolved = true;
      posthog.register({
        install_id: getInstallId(),
        posthog_project_id: getPostHogProjectId(),
      });
      const flagMethods = asFlagMethods();
      flagMethods.onFeatureFlags?.(() => {
        notifyFeatureFlags();
      });
      flagMethods.reloadFeatureFlags?.();
      flushPending();
      notifyFeatureFlags();
    },
  });
};

export const setAnalyticsEnabled = (enabled: boolean): void => {
  captureEnabled = enabled;
  if (!enabled) {
    pendingCaptures.length = 0;
  }
  if (!initAttempted) return;
  if (!enabled) {
    posthog.opt_out_capturing();
    return;
  }
  posthog.opt_in_capturing();
  flushPending();
};

export const isAnalyticsCaptureEnabled = (): boolean => captureEnabled;

export const captureAnalyticsEvent = (
  eventName: string,
  rawProperties: Record<string, unknown>,
): boolean => {
  if (!eventName.trim()) return false;
  const payload = sanitizeAnalyticsProperties(rawProperties);
  if (!captureEnabled) return false;
  if (!initResolved) {
    if (pendingCaptures.length >= MAX_PENDING_CAPTURES) {
      pendingCaptures.shift();
    }
    pendingCaptures.push({ eventName, properties: payload });
    return true;
  }
  posthog.capture(eventName, payload);
  return true;
};

export const captureProductEvent = (
  eventName: string,
  eventVersion: number,
  properties: Record<string, unknown> = {},
): boolean => {
  const envelope = buildEventEnvelope(eventVersion, sanitizeAnalyticsProperties(properties));
  return captureAnalyticsEvent(eventName, envelope);
};

export const checkFeatureGate = (gate: string, fallback = false): boolean => {
  return evaluateFeatureGate(gate, fallback).value;
};

export const evaluateFeatureGate = (
  gate: string,
  fallback = false,
): { value: boolean; reason: "override" | "posthog" | "fallback" } => {
  const overrides = readFeatureOverrides();
  if (overrides && Object.prototype.hasOwnProperty.call(overrides, gate)) {
    return { value: Boolean(overrides[gate]), reason: "override" };
  }
  if (!initResolved) return { value: fallback, reason: "fallback" };
  const evaluated = posthog.isFeatureEnabled(gate);
  if (typeof evaluated !== "boolean") {
    return { value: fallback, reason: "fallback" };
  }
  return { value: evaluated, reason: "posthog" };
};

export const subscribeFeatureFlags = (listener: FeatureFlagListener): (() => void) => {
  featureFlagListeners.add(listener);
  return () => {
    featureFlagListeners.delete(listener);
  };
};
