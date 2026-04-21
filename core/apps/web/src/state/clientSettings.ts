import { uiStateDelete, uiStateGet, uiStateSet } from "./uiStateStore";

export type ClientSettingsV1 = {
  v: 1;
  desktopNotifications: {
    turnCompleted: boolean;
  };
};

export type ClientSettings = {
  v: 2;
  desktopNotifications: {
    turnCompleted: boolean;
    turnFailed: boolean;
    badgeUnreadCount: boolean;
  };
};

export type ClientSettingsState = {
  loaded: boolean;
  settings: ClientSettings;
};

const CLIENT_SETTINGS_KEY_V1 = "client.settings.v1";
const CLIENT_SETTINGS_KEY = "client.settings.v2";

const DEFAULT_SETTINGS: ClientSettings = {
  v: 2,
  desktopNotifications: {
    turnCompleted: true,
    turnFailed: true,
    badgeUnreadCount: true,
  },
};

let state: ClientSettingsState = {
  loaded: false,
  settings: DEFAULT_SETTINGS,
};

let loadPromise: Promise<ClientSettingsState> | null = null;
const listeners = new Set<() => void>();

const emit = () => {
  for (const listener of listeners) {
    listener();
  }
};

const normalizeV1 = (raw: unknown): ClientSettingsV1 | null => {
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as Partial<ClientSettingsV1>;
  if (rec.v !== 1) return null;
  const desktopNotifications = (rec.desktopNotifications ?? {}) as Partial<ClientSettingsV1["desktopNotifications"]>;
  const turnCompleted =
    typeof desktopNotifications.turnCompleted === "boolean"
      ? desktopNotifications.turnCompleted
      : false;
  return {
    v: 1,
    desktopNotifications: {
      turnCompleted,
    },
  };
};

const normalizeV2 = (raw: unknown): ClientSettings | null => {
  if (!raw || typeof raw !== "object") return null;
  const rec = raw as Partial<ClientSettings>;
  if (rec.v !== 2) return null;
  const desktopNotifications = (rec.desktopNotifications ?? {}) as Partial<ClientSettings["desktopNotifications"]>;
  return {
    v: 2,
    desktopNotifications: {
      turnCompleted:
        typeof desktopNotifications.turnCompleted === "boolean"
          ? desktopNotifications.turnCompleted
          : DEFAULT_SETTINGS.desktopNotifications.turnCompleted,
      turnFailed:
        typeof desktopNotifications.turnFailed === "boolean"
          ? desktopNotifications.turnFailed
          : DEFAULT_SETTINGS.desktopNotifications.turnFailed,
      badgeUnreadCount:
        typeof desktopNotifications.badgeUnreadCount === "boolean"
          ? desktopNotifications.badgeUnreadCount
          : DEFAULT_SETTINGS.desktopNotifications.badgeUnreadCount,
    },
  };
};

const migrateV1ToV2 = (legacy: ClientSettingsV1): ClientSettings => {
  const turnCompleted = legacy.desktopNotifications.turnCompleted;
  return {
    v: 2,
    desktopNotifications: {
      turnCompleted,
      turnFailed: turnCompleted,
      badgeUnreadCount: turnCompleted,
    },
  };
};

export function getClientSettingsState(): ClientSettingsState {
  return state;
}

export function getClientSettings(): ClientSettings {
  return state.settings;
}

export function subscribeClientSettings(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export async function loadClientSettings(): Promise<ClientSettingsState> {
  if (state.loaded) return state;
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    let next = DEFAULT_SETTINGS;
    try {
      const rawV2 = await uiStateGet(CLIENT_SETTINGS_KEY);
      const normalizedV2 = normalizeV2(rawV2);
      if (normalizedV2) {
        next = normalizedV2;
      } else {
        const rawV1 = await uiStateGet(CLIENT_SETTINGS_KEY_V1);
        const legacy = normalizeV1(rawV1);
        if (legacy) {
          next = migrateV1ToV2(legacy);
          await uiStateSet(CLIENT_SETTINGS_KEY, next);
          await uiStateDelete(CLIENT_SETTINGS_KEY_V1);
        }
      }
    } catch (err) {
      console.warn("client settings load failed, using defaults", err);
    }
    state = { loaded: true, settings: next };
    emit();
    return state;
  })();
  return loadPromise;
}

export async function updateClientSettings(
  patch: Omit<Partial<ClientSettings>, "desktopNotifications"> & {
    desktopNotifications?: Partial<ClientSettings["desktopNotifications"]>;
  },
): Promise<ClientSettingsState> {
  const next: ClientSettings = {
    ...state.settings,
    ...patch,
    v: 2,
    desktopNotifications: {
      ...state.settings.desktopNotifications,
      ...patch.desktopNotifications,
    },
  };
  state = { loaded: true, settings: next };
  emit();
  try {
    await uiStateSet(CLIENT_SETTINGS_KEY, next);
  } catch (err) {
    console.warn("client settings save failed", err);
  }
  return state;
}
