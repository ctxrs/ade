export function resolveLocalOrigin(origin: string | null): string | null {
  if (!origin) return null;
  try {
    const url = new URL(origin);
    const host = url.hostname.toLowerCase();
    if (host === "localhost" || host === "127.0.0.1" || host === "::1") {
      return url.origin;
    }
    if (isCgnat(host)) {
      return url.origin;
    }
  } catch {
    return null;
  }
  return null;
}

function isCgnat(host: string): boolean {
  const parts = host.split(".");
  if (parts.length !== 4) return false;
  const a = Number(parts[0]);
  const b = Number(parts[1]);
  if (!Number.isInteger(a) || !Number.isInteger(b)) return false;
  return a === 100 && b >= 64 && b <= 127;
}
