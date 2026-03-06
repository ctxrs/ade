const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");

const HELPER_PATH = require.resolve("./specs/helpers/provider_oauth_flow.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadHelper = () => {
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  return require(HELPER_PATH);
};

test.afterEach(() => {
  delete global.browser;
  delete global.fetch;
  delete global.window;
  delete globalThis.window;
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
});

test("sanitizeAuthUrl strips query and fragment data", () => {
  const { sanitizeAuthUrl } = loadHelper();
  const result = sanitizeAuthUrl(
    "https://chat.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6543%2Fauth%2Fcallback&state=abc#frag",
  );

  assert.deepEqual(result, {
    scheme: "https",
    host: "chat.openai.com",
    path: "/oauth/authorize",
    redacted: "https://chat.openai.com/oauth/authorize",
  });
});

test("normalizeLoginStartPayload and redactPayload normalize codex login responses", () => {
  const { PROVIDER_OAUTH_DESCRIPTORS, normalizeLoginStartPayload, redactPayload } = loadHelper();
  const descriptor = PROVIDER_OAUTH_DESCRIPTORS.codex;
  const payload = {
    account_id: "acct-codex",
    auth_url: "https://chat.openai.com/oauth/authorize?state=secret",
    expected_callback_url: "http://127.0.0.1:6543/auth/callback?code=secret",
    completion_token: "token-secret",
    label: "Codex Sub",
  };

  const normalized = normalizeLoginStartPayload(descriptor, payload);
  assert.equal(normalized.loginId, "acct-codex");
  assert.equal(normalized.authUrl, payload.auth_url);
  assert.equal(normalized.expectedCallbackUrl, payload.expected_callback_url);
  assert.equal(normalized.completionToken, "token-secret");
  assert.deepEqual(normalized.redactedPayload, {
    account_id: "acct-codex",
    auth_url: {
      scheme: "https",
      host: "chat.openai.com",
      path: "/oauth/authorize",
      redacted: "https://chat.openai.com/oauth/authorize",
    },
    expected_callback_url: {
      scheme: "http",
      host: "127.0.0.1:6543",
      path: "/auth/callback",
      redacted: "http://127.0.0.1:6543/auth/callback",
    },
    completion_token: "[redacted]",
    label: "Codex Sub",
  });

  assert.deepEqual(redactPayload({ token: "abc", nested: { api_key: "def" } }), {
    token: "[redacted]",
    nested: { api_key: "[redacted]" },
  });
});

test("chooseObservedAuthUrl prefers a matching desktop-open event over login status fallback", () => {
  const { chooseObservedAuthUrl } = loadHelper();
  const observed = chooseObservedAuthUrl({
    probeEvents: [
      {
        at_ms: Date.now(),
        href: "https://chat.openai.com/oauth/authorize?state=from-shell",
      },
    ],
    fallbackAuthUrl: "https://chat.openai.com/oauth/authorize?state=from-status",
  });

  assert.equal(observed.source, "desktop_open_external");
  assert.deepEqual(observed.sanitizedAuthUrl, {
    scheme: "https",
    host: "chat.openai.com",
    path: "/oauth/authorize",
    redacted: "https://chat.openai.com/oauth/authorize",
  });
});

test("provider oauth harness records redacted artifacts through terminal success and account activation", async () => {
  const artifactDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-provider-oauth-harness-test-"));
  const artifactPath = path.join(artifactDir, "artifact.json");
  const fakeWindow = {
    __TAURI__: {
      core: {
        invoke: async (command) => {
          if (command === "desktop_get_connection" || command === "desktop_connect_local") {
            return {
              kind: "local",
              base_url: "http://127.0.0.1:4311",
              token: "desktop-token",
            };
          }
          throw new Error(`unexpected invoke command: ${String(command)}`);
        },
      },
    },
  };
  global.window = fakeWindow;
  globalThis.window = fakeWindow;

  let loginPollCount = 0;
  global.browser = {
    execute: async (fn, ...args) => await fn(...args),
  };
  global.fetch = async (url, options) => {
    const href = String(url);
    if (href.endsWith("/api/providers/codex/accounts/login/start")) {
      return new Response(
        JSON.stringify({
          account_id: "acct-e2e",
          auth_url: "https://chat.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6543%2Fauth%2Fcallback&state=secret",
          expected_callback_url: "http://127.0.0.1:6543/auth/callback?code=secret",
          completion_token: "completion-token-secret",
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (href.endsWith("/api/providers/codex/accounts/login/acct-e2e")) {
      loginPollCount += 1;
      const payload = loginPollCount === 1
        ? {
            account_id: "acct-e2e",
            auth_url: "https://chat.openai.com/oauth/authorize?state=pending",
            status: "pending",
            completion_token: "completion-token-secret",
          }
        : {
            account_id: "acct-e2e",
            auth_url: "https://chat.openai.com/oauth/authorize?state=done",
            status: "success",
          };
      return new Response(JSON.stringify(payload), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    if (href.endsWith("/api/providers/codex/accounts")) {
      return new Response(
        JSON.stringify({
          active_account_id: "acct-e2e",
          accounts: [
            {
              id: "acct-e2e",
              label: "Codex",
              email: "user@example.com",
            },
          ],
          logins: [],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }
    if (href.endsWith("/api/health") || href.endsWith("/api/providers")) {
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error(`unexpected fetch: ${href} method=${options?.method || "GET"}`);
  };

  const { createProviderOAuthHarness } = loadHelper();
  const harness = createProviderOAuthHarness({
    outputPath: artifactPath,
    pollMs: 5,
    timeoutMs: 1000,
    urlFallbackGraceMs: 0,
  });

  const started = await harness.startProviderLogin("codex", "Codex Test");
  assert.equal(started.loginId, "acct-e2e");
  const terminal = await harness.awaitLoginTerminal(started.loginId, 1000);
  assert.equal(terminal.status, "success");
  const active = await harness.assertAccountActivated("codex");
  assert.equal(active.activeAccountId, "acct-e2e");

  const artifact = JSON.parse(fs.readFileSync(artifactPath, "utf8"));
  assert.equal(artifact.schema_version, 1);
  assert.equal(artifact.sessions[0].provider_id, "codex");
  assert.equal(artifact.sessions[0].completion_token_present, true);
  assert.equal(artifact.sessions[0].terminal_status, "success");
  assert.equal(artifact.sessions[0].auth_url.redacted, "https://chat.openai.com/oauth/authorize");
  assert.ok(
    JSON.stringify(artifact).includes("completion-token-secret") === false,
    "artifacts should not contain raw completion tokens",
  );
});
