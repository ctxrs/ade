const BASE_MS = Date.UTC(2025, 0, 1, 0, 0, 0);

export const isoAt = (offsetSeconds: number): string =>
  new Date(BASE_MS + offsetSeconds * 1000).toISOString();

