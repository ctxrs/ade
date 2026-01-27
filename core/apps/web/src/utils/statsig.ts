import { useEffect, useState } from "react";
import { randomUuid } from "./randomUuid";

const CLIENT_KEY = import.meta.env.VITE_STATSIG_CLIENT_KEY as string | undefined;
const USER_ID_KEY = "ctx-statsig-user-id";

let initPromise: Promise<void> | null = null;
let initDone = false;
let client: StatsigClientLike | null = null;

type StatsigClientLike = {
  initializeAsync: () => Promise<void>;
  checkGate: (gate: string) => boolean;
};
type StatsigUser = {
  userID: string;
};

const loadStatsigClient = async (): Promise<{
  StatsigClient: new (clientKey: string, user: StatsigUser) => StatsigClientLike;
}> => {
  const moduleId = "@statsig/js-client";
  return import(/* @vite-ignore */ moduleId);
};

type FeatureFlagOverrides = Record<string, boolean>;

const readOverrides = (): FeatureFlagOverrides | null => {
  if (typeof globalThis === "undefined") return null;
  const overrides = (globalThis as any).__CTX_FEATURE_FLAGS__;
  if (!overrides || typeof overrides !== "object") return null;
  return overrides as FeatureFlagOverrides;
};

const getOverrideValue = (gate: string): boolean | null => {
  const overrides = readOverrides();
  if (!overrides) return null;
  if (!Object.prototype.hasOwnProperty.call(overrides, gate)) return null;
  return Boolean(overrides[gate]);
};

const getUserId = (): string => {
  if (typeof window === "undefined") return "server";
  try {
    const existing = window.localStorage.getItem(USER_ID_KEY);
    if (existing) return existing;
    const next = randomUuid();
    window.localStorage.setItem(USER_ID_KEY, next);
    return next;
  } catch {
    try {
      return randomUuid();
    } catch {
      return `anon-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    }
  }
};

const buildUser = (): StatsigUser => ({
  userID: getUserId(),
});

export const initStatsig = (): Promise<void> | null => {
  if (!CLIENT_KEY) return null;
  if (typeof window === "undefined") return null;
  if (initPromise) return initPromise;
  initPromise = loadStatsigClient()
    .then((mod) => {
      client = new mod.StatsigClient(CLIENT_KEY, buildUser());
      return client.initializeAsync();
    })
    .then(() => {
      initDone = true;
    })
    .catch(() => {
      initDone = false;
    });
  return initPromise;
};

export const checkStatsigGate = (gate: string, fallback = false): boolean => {
  const override = getOverrideValue(gate);
  if (override !== null) return override;
  if (!CLIENT_KEY || !initDone || !client) return fallback;
  try {
    return client.checkGate(gate);
  } catch {
    return fallback;
  }
};

export const useStatsigGate = (gate: string, fallback = false): boolean => {
  const override = getOverrideValue(gate);
  const [value, setValue] = useState<boolean>(override ?? fallback);

  useEffect(() => {
    const nextOverride = getOverrideValue(gate);
    if (nextOverride !== null) {
      setValue(nextOverride);
      return;
    }
    const init = initStatsig();
    if (!init) return;
    let active = true;
    init.then(() => {
      if (!active) return;
      setValue(checkStatsigGate(gate, fallback));
    });
    return () => {
      active = false;
    };
  }, [gate, fallback]);

  return value;
};
