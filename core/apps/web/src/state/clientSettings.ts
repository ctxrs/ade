import { uiStateGet, uiStateSet } from "./uiStateStore";

export type ClientSettings = {
  v: 1;
  desktopNotifications: {
    turnCompleted: boolean;
  };
};

export type ClientSettingsState = {
  loaded: boolean;
  settings: ClientSettings;
};

const CLIENT_SETTINGS_KEY = "client.settings.v1";

const DEFAULT_SETTINGS: ClientSettings = {
  v: 1,
  desktopNotifications: {
    turnCompleted: false,
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

const normalizeSettings = (raw: unknown): ClientSettings => {
  if (!raw || typeof raw !== "object") return DEFAULT_SETTINGS;
  const rec = raw as Partial<ClientSettings>;
  if (rec.v !== 1) return DEFAULT_SETTINGS;
  const desktopNotifications = rec.desktopNotifications ?? {};
  const turnCompleted =
    typeof desktopNotifications.turnCompleted === "boolean"
      ? desktopNotifications.turnCompleted
      : DEFAULT_SETTINGS.desktopNotifications.turnCompleted;
  return {
    v: 1,
    desktopNotifications: {
      turnCompleted,
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
      const raw = await uiStateGet(CLIENT_SETTINGS_KEY);
      next = normalizeSettings(raw);
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
  patch: Partial<ClientSettings> & { desktopNotifications?: Partial<ClientSettings["desktopNotifications"]> },
): Promise<ClientSettingsState> {
  const next: ClientSettings = {
    ...state.settings,
    ...patch,
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
