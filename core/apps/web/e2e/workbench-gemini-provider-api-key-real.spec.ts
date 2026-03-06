import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { Locator, Page } from "playwright/test";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import {
  ensureProviderInstalledAndHealthy,
  resolveWorkspaceProviderModelId,
  verifyProviderForWorkspace,
  waitForTerminalState,
} from "../src/testing/providerRuntime";

const REQUEST_TIMEOUT_MS = 60_000;
const INSTALL_TARGET = "host";

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const readString = (value: unknown): string => (typeof value === "string" ? value : "");

async function openHarnessMenu(page: Page): Promise<Locator> {
  const harnessButton = page
    .locator(
      ".wb-new-composer-stack .wb-switcher-harness, .wb-new-composer-stack button[title='Harness'], button[title='Agents']",
    )
    .first();
  await expect(harnessButton).toBeVisible({ timeout: 20_000 });
  const menu = page.locator(".wb-harness-menu");
  if (!(await menu.isVisible().catch(() => false))) {
    await harnessButton.click();
    await expect(menu).toBeVisible({ timeout: 10_000 });
  }
  return menu;
}

async function configureGeminiApiKeyViaModal(page: Page, apiKey: string): Promise<void> {
  console.warn("[gemini-provider-api-key-real] opening harness menu");
  const menu = await openHarnessMenu(page);
  await menu.getByLabel("Search agents").fill("gemini", { timeout: 10_000 });
  const geminiRowButton = menu
    .locator(".wb-harness-row .wb-harness-row-main")
    .filter({ hasText: /Gemini/i })
    .first();
  await expect(geminiRowButton).toBeVisible({ timeout: 20_000 });
  await geminiRowButton.click({ timeout: 10_000 });
  console.warn("[gemini-provider-api-key-real] gemini row selected");

  const modal = page.locator(".settings-harness-modal");
  const modalVisible = await modal.waitFor({ state: "visible", timeout: 3_000 }).then(() => true).catch(() => false);
  if (!modalVisible) {
    console.warn("[gemini-provider-api-key-real] modal not shown; assuming existing auth");
    return;
  }
  console.warn("[gemini-provider-api-key-real] auth modal visible");

  console.warn("[gemini-provider-api-key-real] selecting API Key auth mode");
  await modal.getByRole("button", { name: "API Key" }).click({ timeout: 10_000 });
  console.warn("[gemini-provider-api-key-real] API Key mode selected");
  await expect(modal.getByRole("link", { name: "Google AI Studio" })).toHaveAttribute(
    "href",
    "https://aistudio.google.com/app/apikey",
  );
  console.warn("[gemini-provider-api-key-real] AI Studio link verified");
  await expect(modal.getByRole("combobox", { name: "Gemini auth mode" })).toContainText("Gemini API Key");
  console.warn("[gemini-provider-api-key-real] auth mode verified");

  const apiKeyInput = modal.locator("input[type='password']").first();
  await expect(apiKeyInput).toBeVisible({ timeout: 10_000 });
  await apiKeyInput.fill(apiKey, { timeout: 10_000 });
  console.warn("[gemini-provider-api-key-real] API key field filled");
  const labelInput = modal
    .locator("label.settings-harness-modal-label")
    .filter({ hasText: "Label (optional)" })
    .locator("input")
    .first();
  if ((await labelInput.count()) > 0) {
    await labelInput.fill("Gemini API key E2E", { timeout: 10_000 });
  }
  console.warn("[gemini-provider-api-key-real] label field filled");

  console.warn("[gemini-provider-api-key-real] submitting API key");
  await modal.getByRole("button", { name: "Add API key" }).click({ timeout: 10_000 });
  console.warn("[gemini-provider-api-key-real] submit clicked");
  const modalClosed = await modal.waitFor({ state: "hidden", timeout: 30_000 }).then(() => true).catch(() => false);
  if (!modalClosed) {
    console.warn("[gemini-provider-api-key-real] modal still open after submit");
    const errorLocator = page.locator(".settings-row-error").first();
    const errorText = (await errorLocator.count()) > 0
      ? await errorLocator.textContent({ timeout: 1_000 }).catch(() => null)
      : null;
    if (errorText && errorText.trim().length > 0) {
      console.warn(`[gemini-provider-api-key-real] modal warning: ${errorText.trim()}`);
    }
    const closeButton = modal.getByRole("button", { name: "Close" });
    if ((await closeButton.count()) > 0) {
      await closeButton.click({ timeout: 10_000 });
    }
    await expect(modal).toBeHidden({ timeout: 10_000 });
  }
  console.warn("[gemini-provider-api-key-real] modal closed");
}

test("workbench: gemini provider API key auth can run a real task", async ({ page, request }) => {
  test.setTimeout(10 * 60_000);

  if ((process.env.CTX_E2E_TIER ?? "") !== "provider-api-auth") {
    test.skip(true, "set CTX_E2E_TIER=provider-api-auth to run gemini provider API-key e2e");
  }

  const geminiApiKey = (process.env.CTX_E2E_GEMINI_API_KEY ?? "").trim();
  if (!geminiApiKey) {
    test.skip(true, "missing CTX_E2E_GEMINI_API_KEY");
  }

  const providerStatus = await ensureProviderInstalledAndHealthy(request, "gemini", INSTALL_TARGET, {
    timeoutMs: 10 * 60_000,
    pollMs: 2_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  console.warn(
    `[gemini-provider-api-key-real] provider ready: target=${providerStatus.details.install_target || INSTALL_TARGET} health=${providerStatus.health}`,
  );

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-gemini-"));
  execSync("git init -b main", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "gemini provider auth e2e\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request,
    repo,
    workspaceName: `gemini-auth-${Date.now()}`,
  });
  console.warn(`[gemini-provider-api-key-real] workspace ready: ${workspaceId}`);

  await configureGeminiApiKeyViaModal(page, geminiApiKey);
  console.warn("[gemini-provider-api-key-real] api key submitted");

  await verifyProviderForWorkspace(request, workspaceId, "gemini", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  console.warn("[gemini-provider-api-key-real] verify step done");

  const modelId = await resolveWorkspaceProviderModelId(request, workspaceId, "gemini", {
    timeoutMs: 90_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  console.warn(`[gemini-provider-api-key-real] using model: ${modelId}`);

  const promptMarker = `gemini-provider-auth-${Date.now()}`;
  const prompt = `${promptMarker}: reply with exactly the word pong`;

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
  console.warn(`[gemini-provider-api-key-real] task created: ${taskId}`);

  const createSessionResp = await request.post(`/api/tasks/${taskId}/sessions`, {
    data: {
      provider_id: "gemini",
      model_id: modelId,
      env_target: "worktree",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(createSessionResp.ok(), `session create failed (${createSessionResp.status()})`).toBe(true);
  const sessionId = readString(asRecord(await createSessionResp.json()).id);
  expect(sessionId).not.toBe("");
  console.warn(`[gemini-provider-api-key-real] session created: ${sessionId}`);

  const messageResp = await request.post(`/api/sessions/${sessionId}/messages`, {
    data: {
      content: prompt,
      delivery: "immediate",
    },
    timeout: REQUEST_TIMEOUT_MS,
  });
  expect(messageResp.ok(), `message send failed (${messageResp.status()})`).toBe(true);
  console.warn("[gemini-provider-api-key-real] prompt sent");

  const terminal = await waitForTerminalState(request, sessionId, {
    timeoutMs: 180_000,
    pollMs: 3_000,
    requestTimeoutMs: REQUEST_TIMEOUT_MS,
  });
  console.warn(`[gemini-provider-api-key-real] terminal status: ${terminal.terminalStatus ?? "unknown"}`);
  expect(terminal.terminalStatus, terminal.errorMessage ?? "gemini run did not complete").toBe("completed");
  expect(terminal.assistantMessages).toBeGreaterThan(0);
});
