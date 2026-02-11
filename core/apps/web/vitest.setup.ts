import "@testing-library/jest-dom";

function createMemoryStorage(): Storage {
  const store = new Map<string, string>();
  return {
    get length() {
      return store.size;
    },
    clear() {
      store.clear();
    },
    getItem(key: string) {
      return store.has(key) ? store.get(key) ?? null : null;
    },
    key(index: number) {
      if (!Number.isInteger(index) || index < 0 || index >= store.size) return null;
      return Array.from(store.keys())[index] ?? null;
    },
    removeItem(key: string) {
      store.delete(key);
    },
    setItem(key: string, value: string) {
      store.set(key, String(value));
    },
  };
}

function hasValidStorageApi(value: unknown): value is Storage {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<Storage>;
  return (
    typeof candidate.getItem === "function" &&
    typeof candidate.setItem === "function" &&
    typeof candidate.removeItem === "function" &&
    typeof candidate.clear === "function" &&
    typeof candidate.key === "function"
  );
}

function ensureStorage(name: "localStorage" | "sessionStorage"): void {
  const current = (globalThis as Record<string, unknown>)[name];
  if (hasValidStorageApi(current)) return;
  const replacement = createMemoryStorage();
  Object.defineProperty(globalThis, name, {
    configurable: true,
    enumerable: true,
    writable: true,
    value: replacement,
  });
  if (typeof window !== "undefined") {
    Object.defineProperty(window, name, {
      configurable: true,
      enumerable: true,
      writable: true,
      value: replacement,
    });
  }
}

ensureStorage("localStorage");
ensureStorage("sessionStorage");

// Some UI dependencies (notably xterm) probe canvas APIs at import time.
// JSDOM doesn't implement canvas, so provide a small stub to keep unit tests
// focused on app behavior.
if (typeof HTMLCanvasElement !== "undefined") {
  // eslint-disable-next-line no-extend-native
  (HTMLCanvasElement.prototype as any).getContext ??= () => {
    return {
      fillStyle: "",
      fillRect: () => {},
      clearRect: () => {},
      getImageData: () => ({ data: new Uint8ClampedArray([0, 0, 0, 255]) }),
      putImageData: () => {},
      measureText: () => ({ width: 0 }),
    };
  };
}
