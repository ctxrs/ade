import { sha256Hex } from "./sha256";

export type BrowserCapabilityScope =
  | { kind: "blob"; blobId: string }
  | { kind: "session_artifact"; sessionId: string; artifactId: string };

const BROWSER_CAPABILITY_TOKEN_TTL_MS = 60 * 60 * 1000;
const encoder = new TextEncoder();

export const serializeBrowserCapabilityScope = (
  scope: BrowserCapabilityScope,
): string => {
  switch (scope.kind) {
    case "blob":
      return `blob:${scope.blobId}`;
    case "session_artifact":
      return `session_artifact:${scope.sessionId}:${scope.artifactId}`;
  }
};

export const deriveBrowserCapabilityToken = (
  authToken: string,
  scope: BrowserCapabilityScope,
  expiresAt: number,
): string =>
  sha256Hex(
    encoder.encode(
      `ctx-browser-capability|${serializeBrowserCapabilityScope(scope)}|${expiresAt}|${authToken}`,
    ),
  );

export const setBrowserCapabilityQueryToken = (
  query: URLSearchParams,
  authToken: string | null | undefined,
  scope: BrowserCapabilityScope,
): void => {
  if (!authToken) return;
  const expiresAt = Math.floor((Date.now() + BROWSER_CAPABILITY_TOKEN_TTL_MS) / 1000);
  query.set("expires_at", String(expiresAt));
  query.set("token", deriveBrowserCapabilityToken(authToken, scope, expiresAt));
};
