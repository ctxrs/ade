import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import type { APIRequestContext, Locator, Page } from "playwright/test";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

type TerminalState = {
  done: boolean;
  terminalStatus: string | null;
  assistantMessages: number;
  errorMessage: string | null;
};

const TERMINAL_TURN_STATUSES = new Set(["completed", "failed", "interrupted"]);
const REQUEST_TIMEOUT_MS = 60_000;

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const readString = (value: unknown): string => (typeof value === "string" ? value : "");

const firstText = (...values: unknown[]): string => {
  for (const value of values) {
    const text = readString(value).trim();
    if (text) return text;
  }
  return "";
};

const normalizeErrorMessage = (raw: string): string => raw.replace(/\s+/g, " ").trim();

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

async function ensureGeminiProviderReady(request: APIRequestContext): Promise<{ ok: true } | { ok: false; reason: string }> {
  const providersResp = await request.get("/api/providers", { timeout: REQUEST_TIMEOUT_MS });
  if (!providersResp.ok()) {
    return { ok: false, reason: `failed to read providers (${providersResp.status()})` };
  }
  const providers = asArray(await providersResp.json()).map((entry) => asRecord(entry));
  const gemini = providers.find((entry) => readString(entry.provider_id) === "gemini");
  if (!gemini) {
    return { ok: false, reason: "gemini provider not listed by /api/providers" };
  }
  if (gemini.installed !== true || readString(gemini.health) !== "ok") {
    const diagnostics = asArray(gemini.diagnostics)
      .map((entry) => readString(entry).trim())
      .filter((entry) => entry.length > 0);
    return {
      ok: false,
      reason: diagnostics[0] ?? `gemini provider unavailable (installed=${String(gemini.installed)}, health=${readString(gemini.health) || "unknown"})`,
    };
  }
  return { ok: true };
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

async function verifyGeminiProviderForWorkspace(opts: {
  request: APIRequestContext;
  workspaceId: string;
}): Promise<{ ok: true } | { ok: false; reason: string }> {
  const { request, workspaceId } = opts;
  const resp = await request.post(`/api/workspaces/${workspaceId}/providers/gemini/verify`, {
    data: {},
    timeout: 30_000,
  });
  if (!resp.ok()) {
    const payload = asRecord(await resp.json().catch(() => ({})));
    return {
      ok: false,
      reason: normalizeErrorMessage(
        firstText(payload.error, payload.message, `verify request failed (${resp.status()})`) || "verify request failed",
      ),
    };
  }
  const payload = asRecord(await resp.json());
  const status = firstText(payload.status).toLowerCase();
  if (status !== "ok") {
    return {
      ok: false,
      reason: normalizeErrorMessage(firstText(payload.message, `verify status=${status}`)),
    };
  }
  return { ok: true };
}

async function resolveGeminiModelId(opts: {
  request: APIRequestContext;
  workspaceId: string;
}): Promise<{ ok: true; modelId: string } | { ok: false; reason: string }> {
  const { request, workspaceId } = opts;
  const optionsResp = await request.get(`/api/workspaces/${workspaceId}/providers/gemini/options`, {
    timeout: REQUEST_TIMEOUT_MS,
  });
  if (!optionsResp.ok()) {
    return { ok: false, reason: `failed to read gemini options (${optionsResp.status()})` };
  }
  const options = asRecord(await optionsResp.json());
  const models = asRecord(options.models);
  const currentModelId = firstText(models.current_model_id, models.currentModelId);
  if (currentModelId) {
    return { ok: true, modelId: currentModelId };
  }

  const firstModelId = asArray(models.models)
    .map((entry) => asRecord(entry))
    .map((entry) => firstText(entry.id, entry.model_id, entry.modelId, entry.name))
    .find((entry) => entry.length > 0);
  if (!firstModelId) {
    return { ok: false, reason: "gemini options did not return a model id" };
  }
  return { ok: true, modelId: firstModelId };
}

function toTerminalState(snapshot: Record<string, unknown>): TerminalState {
  const head = asRecord(snapshot.head);
  const activity = asRecord(head.activity);
  const turns = asArray(head.turns).map((entry) => asRecord(entry));
  const messages = asArray(head.messages).map((entry) => asRecord(entry));

  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : {};
  const terminalStatus = firstText(lastTurn.status, activity.last_turn_status).toLowerCase() || null;
  const assistantMessages = messages.filter((message) => {
    if (readString(message.role) !== "assistant") return false;
    return readString(message.content).trim().length > 0;
  }).length;

  const done = terminalStatus
    ? TERMINAL_TURN_STATUSES.has(terminalStatus)
    : (activity.is_working !== true && assistantMessages > 0);

  const errorMessage = terminalStatus === "failed" || terminalStatus === "interrupted"
    ? normalizeErrorMessage(firstText(lastTurn.status, "gemini run failed"))
    : null;

  return {
    done,
    terminalStatus,
    assistantMessages,
    errorMessage,
  };
}

async function waitForTerminalState(opts: {
  request: APIRequestContext;
  sessionId: string;
}): Promise<TerminalState> {
  const { request, sessionId } = opts;
  let resolved: TerminalState | null = null;

  await expect
    .poll(
      async () => {
        const resp = await request.get(`/api/sessions/${sessionId}/head?include_events=1&limit=80`, {
          timeout: REQUEST_TIMEOUT_MS,
        });
        if (!resp.ok()) return "";
        const state = toTerminalState({ head: asRecord(await resp.json()) });
        if (!state.done) return "";
        resolved = state;
        return "done";
      },
      { timeout: 180_000, intervals: [1_000, 2_000, 3_000] },
    )
    .toBe("done");

  if (!resolved) {
    throw new Error(`gemini session ${sessionId} did not reach terminal state`);
  }
  return resolved;
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

  const providerReady = await ensureGeminiProviderReady(request);
  expect(providerReady.ok, providerReady.ok ? undefined : providerReady.reason).toBe(true);
  if (!providerReady.ok) return;
  console.warn("[gemini-provider-api-key-real] provider ready");

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

  const verify = await verifyGeminiProviderForWorkspace({ request, workspaceId });
  if (!verify.ok) {
    console.warn(`[gemini-provider-api-key-real] verify warning: ${verify.reason}`);
  }
  console.warn("[gemini-provider-api-key-real] verify step done");

  const modelSelection = await resolveGeminiModelId({ request, workspaceId });
  const modelId = modelSelection.ok ? modelSelection.modelId : "gemini-2.5-flash";
  if (!modelSelection.ok) {
    console.warn(`[gemini-provider-api-key-real] model discovery warning: ${modelSelection.reason}`);
    console.warn(`[gemini-provider-api-key-real] using fallback model id: ${modelId}`);
  }
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

  const terminal = await waitForTerminalState({ request, sessionId });
  console.warn(`[gemini-provider-api-key-real] terminal status: ${terminal.terminalStatus ?? "unknown"}`);
  expect(terminal.terminalStatus, terminal.errorMessage ?? "gemini run did not complete").toBe("completed");
  expect(terminal.assistantMessages).toBeGreaterThan(0);
});
