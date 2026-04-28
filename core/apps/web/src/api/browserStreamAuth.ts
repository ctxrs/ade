export type BrowserStreamScope =
  | { kind: "workspace_active_snapshot"; workspaceId: string }
  | { kind: "workspace_stream"; workspaceId: string }
  | { kind: "execution_launch"; jobId: string }
  | { kind: "dictation_livekit" }
  | { kind: "provider_install"; installId: string };

const BROWSER_STREAM_TOKEN_TTL_MS = 5 * 60 * 1000;
const encoder = new TextEncoder();

export const serializeBrowserStreamScope = (scope: BrowserStreamScope): string => {
  switch (scope.kind) {
    case "workspace_active_snapshot":
      return `workspace_active_snapshot:${scope.workspaceId}`;
    case "workspace_stream":
      return `workspace_stream:${scope.workspaceId}`;
    case "execution_launch":
      return `execution_launch:${scope.jobId}`;
    case "dictation_livekit":
      return "dictation_livekit";
    case "provider_install":
      return `provider_install:${scope.installId}`;
  }
};

const requireSubtleCrypto = (): SubtleCrypto => {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new Error("Browser stream auth requires Web Crypto.");
  }
  return subtle;
};

const hexEncode = (bytes: Uint8Array): string =>
  Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");

export const deriveBrowserStreamToken = async (
  authToken: string,
  scope: BrowserStreamScope,
  expiresAt: number,
): Promise<string> => {
  const subtle = requireSubtleCrypto();
  const payload = encoder.encode(
    `ctx-browser-stream|${serializeBrowserStreamScope(scope)}|${expiresAt}|${authToken}`,
  );
  const digest = await subtle.digest("SHA-256", payload);
  return hexEncode(new Uint8Array(digest));
};

export const setBrowserStreamQueryToken = async (
  query: URLSearchParams,
  authToken: string | null | undefined,
  scope: BrowserStreamScope,
): Promise<void> => {
  if (!authToken) return;
  const expiresAt = Math.floor((Date.now() + BROWSER_STREAM_TOKEN_TTL_MS) / 1000);
  query.set("expires_at", String(expiresAt));
  query.set("token", await deriveBrowserStreamToken(authToken, scope, expiresAt));
};
