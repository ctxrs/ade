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

test("createTotpCode follows RFC6238 SHA-1 vectors", () => {
  const { createTotpCode, decodeBase32Secret, normalizeBase32Secret } = loadHelper();
  assert.equal(normalizeBase32Secret("GEZD GNBV-GY3TQOJQ===="), "GEZDGNBVGY3TQOJQ");
  assert.equal(decodeBase32Secret("MZXW6===").toString("utf8"), "foo");
  assert.equal(
    createTotpCode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", {
      timestampMs: 59_000,
      digits: 8,
    }),
    "94287082",
  );
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

test("fillBrowserAuthField prefers webdriver element interactions for email entry", async () => {
  let executeCalled = false;
  const setValues = [];
  const emailElement = {
    elementId: "email-1",
    isDisplayed: async () => true,
    scrollIntoView: async () => {},
    click: async () => {},
    clearValue: async () => {},
    setValue: async (value) => {
      setValues.push(value);
    },
    getAttribute: async () => "",
  };
  global.browser = {
    $$: async (selector) => (selector === "input[type='email']" ? [emailElement] : []),
    execute: async () => {
      executeCalled = true;
      throw new Error("webdriver execute fallback should not be used");
    },
  };

  const { fillBrowserAuthField } = loadHelper();
  const result = await fillBrowserAuthField("email", "user@example.com");

  assert.deepEqual(result, { ok: true, filled: 1, mode: "webdriver" });
  assert.deepEqual(setValues, ["user@example.com"]);
  assert.equal(executeCalled, false);
});

test("submitVisibleAuthStep prefers webdriver button clicks before DOM-execute fallback", async () => {
  let clicked = false;
  let executeCalled = false;
  const continueButton = {
    elementId: "button-1",
    isDisplayed: async () => true,
    scrollIntoView: async () => {},
    click: async () => {
      clicked = true;
    },
    getText: async () => "Continue",
    getAttribute: async () => "",
  };
  global.browser = {
    $$: async (selector) => (selector === "button[type='submit']" ? [continueButton] : []),
    execute: async () => {
      executeCalled = true;
      throw new Error("webdriver execute fallback should not be used");
    },
  };

  const { submitVisibleAuthStep } = loadHelper();
  const result = await submitVisibleAuthStep(["continue"]);

  assert.equal(result.ok, true);
  assert.equal(result.strategy, "webdriver-button");
  assert.equal(clicked, true);
  assert.equal(executeCalled, false);
});

test("driveCodexOpenAiLoginWithCredentials classifies inbox code prompts as blocked email challenges", async () => {
  let navigatedUrl = "";
  global.browser = {
    url: async (href) => {
      navigatedUrl = href;
    },
    execute: async () => ({
      url: "https://chat.openai.com/challenge",
      title: "Check your inbox",
      bodyText: "Check your inbox. Enter the code we sent to continue.",
      inputs: [
        {
          tag: "input",
          type: "text",
          name: "code",
          autocomplete: "one-time-code",
          inputmode: "numeric",
          label: "Verification code",
          maxLength: 6,
        },
      ],
      buttons: ["Continue"],
    }),
  };

  const { driveCodexOpenAiLoginWithCredentials } = loadHelper();
  await assert.rejects(
    () => driveCodexOpenAiLoginWithCredentials({
      authUrl: "https://chat.openai.com/oauth/authorize?state=secret",
      email: "user@example.com",
      password: "super-secret-password",
      totpSecret: "JBSWY3DPEHPK3PXP",
      timeoutMs: 100,
      pollMs: 0,
    }),
    (error) => {
      assert.equal(error.name, "ProviderOAuthBlockedError");
      assert.equal(error.blockedReason, "email_challenge_required");
      assert.match(error.message, /email verification code challenge/i);
      assert.equal(error.challenge?.kind, "email_challenge_required");
      assert.equal(error.challenge?.state?.title, "Check your inbox");
      return true;
    },
  );
  assert.equal(navigatedUrl, "https://chat.openai.com/oauth/authorize?state=secret");
});
