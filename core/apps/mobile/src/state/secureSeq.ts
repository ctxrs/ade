import AsyncStorage from "@react-native-async-storage/async-storage";

const SEQ_KEY_PREFIX = "contextMobileSecureSeq.v1.";

const seqCache = new Map<string, number>();
const seqLocks = new Map<string, Promise<number>>();

const loadStoredSeq = async (deviceId: string): Promise<number> => {
  try {
    const raw = await AsyncStorage.getItem(`${SEQ_KEY_PREFIX}${deviceId}`);
    const parsed = raw ? Number.parseInt(raw, 10) : 0;
    return Number.isFinite(parsed) ? parsed : 0;
  } catch {
    return 0;
  }
};

const persistSeq = async (deviceId: string, seq: number): Promise<void> => {
  try {
    await AsyncStorage.setItem(`${SEQ_KEY_PREFIX}${deviceId}`, String(seq));
  } catch {
    // best effort only
  }
};

export const nextSecureSeq = async (deviceId: string): Promise<number> => {
  const pending = seqLocks.get(deviceId) ?? Promise.resolve(0);
  const nextPromise = pending.then(async () => {
    let current = seqCache.get(deviceId);
    if (!Number.isFinite(current as number)) {
      current = undefined;
    }
    if (current === undefined) {
      current = await loadStoredSeq(deviceId);
    }
    const now = Date.now();
    const next = Math.max(now, (current ?? 0) + 1);
    seqCache.set(deviceId, next);
    await persistSeq(deviceId, next);
    return next;
  });
  seqLocks.set(deviceId, nextPromise.catch(() => 0));
  return nextPromise;
};

