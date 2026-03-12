import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { test, expect } from "./fixtures";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import {
  completeClaudeOauthWithGoogleBrowserCredentials,
  createClaudeBrowserAuthContext,
} from "./utils/providerBrowserAuth";
import {
  asRecord,
  ensureProviderInstalledAndHealthy,
  readString,
  resolveWorkspaceProviderModelId,
  verifyProviderForWorkspace,
  waitForSessionWorkspaceFileContents,
  waitForTerminalState,
} from "../src/testing/providerRuntime";

const REQUEST_TIMEOUT_MS = 60_000;
const INSTALL_TARGET = "host" as const;

test("workbench: claude subscription browser OAuth can run a real task", async ({ page, request }) => {
  test.setTimeout(15 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "provider-browser-auth") {
    test.skip(true, "set CTX_E2E_TIER=provider-browser-auth to run claude browser OAuth e2e");
  }

  const googleEmail = (process.env.GOOGLE_TEST_EMAIL ?? "").trim();
  const googlePassword = process.env.GOOGLE_TEST_PASSWORD ?? "";
  if (!googleEmail || !googlePassword) {
    test.skip(true, "missing GOOGLE_TEST_EMAIL / GOOGLE_TEST_PASSWORD");
  }

  await ensureProviderInstalledAndHealthy(request, "claude-crp", INSTALL_TARGET, {
    timeoutMs: 10 * 60_000,
    pollMs: 2_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  // `claude-cli` is a bundled readiness dependency behind `claude-crp`, not the
  // user-facing provider contract this browser-auth e2e is proving.

  const authContextHandle = await createClaudeBrowserAuthContext(page.context());
  try {
    await completeClaudeOauthWithGoogleBrowserCredentials({
      context: authContextHandle.context,
      request,
      email: googleEmail,
      password: googlePassword,
      label: "Claude browser OAuth E2E",
      timeoutMs: 8 * 60_000,
      pollMs: 1_000,
    });
  } finally {
    await authContextHandle.dispose();
  }

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-claude-oauth-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "claude browser oauth e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request,
    repo,
    workspaceName: `claude-oauth-${Date.now()}`,
  });

  await verifyProviderForWorkspace(request, workspaceId, "claude-crp", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const modelId = await resolveWorkspaceProviderModelId(request, workspaceId, "claude-crp", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });

  const promptMarker = `claude-browser-oauth-${Date.now()}`;
  const prompt = [
    "This is an end to end test, so it is very important that you do exactly what I ask.",
    "Make a new file in the workspace root called hello.md and put exactly this text in it: hi. The file must contain exactly those two characters with no trailing newline or extra whitespace. If you use a shell command to write the file, use printf rather than echo -n, because echo -n is not portable and may write the literal text -n.",
    "Use only the current worktree root as the target directory. Do not write in a parent directory, and if your first attempt adds a trailing newline or uses the wrong directory, fix the file before replying.",
    "That is all. Do it now without further deliberation.",
    "After writing the file, reply with exactly: hi",
  ].join(" ");

  const createTaskResp = await request.post(`/api/workspaces/${workspaceId}/tasks`, {
    data: {
      title: promptMarker,
      create_default_session: false,
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(createTaskResp.ok(), `task create failed (${createTaskResp.status()})`).toBe(true);
  const taskId = readString(asRecord(await createTaskResp.json()).id);
  expect(taskId).not.toBe("");

  const createSessionResp = await request.post(`/api/tasks/${taskId}/sessions`, {
    data: {
      provider_id: "claude-crp",
      model_id: modelId,
      execution_environment: "host",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(createSessionResp.ok(), `session create failed (${createSessionResp.status()})`).toBe(true);
  const sessionId = readString(asRecord(await createSessionResp.json()).id);
  expect(sessionId).not.toBe("");

  const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: prompt,
      delivery: "immediate",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(messageResp.ok(), `message send failed (${messageResp.status()})`).toBe(true);

  const terminal = await waitForTerminalState(request, sessionId, {
    timeoutMs: 180_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  expect(terminal.terminalStatus, terminal.errorMessage ?? "claude run did not complete").toBe("completed");
  expect(terminal.assistantMessages).toBeGreaterThan(0);
  await waitForSessionWorkspaceFileContents(request, sessionId, "hello.md", "hi", {
    timeoutMs: 30_000,
    pollMs: 1_000,
  });
});
