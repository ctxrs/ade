import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { execSync } from "child_process";
import { tmpdir } from "os";
import path from "path";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";
import { selectHarnessBySearch } from "./utils/harnessEndpointAuth";
import { waitForTerminalState } from "../src/testing/providerRuntime";

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

test("workbench: context window meter renders for a live fake-provider session", async ({ page, request }) => {
  test.setTimeout(120_000);

  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName: `ws-${Date.now()}`,
  });
  await selectHarnessBySearch(page, "fake", /fake/i);
  await expect(page.locator(".wb-new-composer-card .wb-context-window")).toHaveCount(0);

  let forceStaleFirstSnapshot = true;
  await page.route("**/api/sessions/*/snapshot**", async (route) => {
    if (!forceStaleFirstSnapshot) {
      await route.continue();
      return;
    }
    forceStaleFirstSnapshot = false;
    const response = await route.fetch();
    const snapshot = asRecord(await response.json());
    const head = asRecord(snapshot.head);
    const staleHead = {
      ...head,
      turns: [],
      messages: [],
      events: [],
      tool_summaries: [],
      has_more_turns: false,
      last_event_seq: 0,
    };
    await route.fulfill({
      response,
      body: JSON.stringify({ ...snapshot, head: staleHead }),
    });
  });

  const prompt = "slow-diff-test 0123456789";
  const createSessionResponsePromise = page.waitForResponse((response) =>
    response.request().method() === "POST"
    && /\/api\/tasks\/[^/]+\/sessions$/.test(response.url()),
  );
  await page.locator("textarea.wb-composer-textarea").first().fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
  const createSessionResponse = await createSessionResponsePromise;
  expect(createSessionResponse.ok()).toBe(true);
  const sessionId = String((await createSessionResponse.json() as { id?: string }).id ?? "");
  expect(sessionId).not.toBe("");

  const rows = page.locator(".wb-task-row");
  await expect(rows).toHaveCount(1, { timeout: 20_000 });
  await rows.first().click();

  const activeTextarea = page.locator(".wb-session-slot textarea.wb-active-textarea");
  await expect(activeTextarea).toBeVisible({ timeout: 20_000 });
  const terminal = await waitForTerminalState(request, sessionId, {
    timeoutMs: 60_000,
    pollMs: 1_000,
  });
  expect(terminal.terminalStatus, terminal.errorMessage ?? "fake-provider run did not complete").toBe("completed");

  const contextWindow = page.locator(".wb-session-slot .wb-context-window");
  await expect(contextWindow).toBeVisible({ timeout: 20_000 });
  await expect(contextWindow).toHaveText("7% · 7/100", { timeout: 20_000 });
  await expect(contextWindow).toHaveAttribute("title", "Context Window: 7% · 7/100");
});
