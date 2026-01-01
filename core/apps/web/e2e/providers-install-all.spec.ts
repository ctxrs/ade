import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("providers: install all completes for supported providers", async ({ page, request }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  await createWorkspaceAndOpenWorkbench({ page, request, repo, workspaceName });

  await page.goto("/providers");
  await expect(page.getByRole("heading", { name: "Providers" })).toBeVisible();

  const providersResp = await request.get("/api/providers");
  expect(providersResp.ok()).toBeTruthy();
  const providers = (await providersResp.json()) as any[];
  const installable = providers.filter((p) => p.details?.install_supported === "true");
  expect(installable.length).toBeGreaterThan(0);

  const installIds = new Map<string, string>();
  const now = new Date().toISOString();

  await page.route("**/api/providers/install/*/stream", (route) =>
    route.fulfill({ status: 204, body: "" }),
  );
  await page.route("**/api/providers/install/*/events", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/providers/install/*", (route) => {
    const url = new URL(route.request().url());
    const installId = url.pathname.split("/").pop() ?? "";
    const providerId = installIds.get(installId) ?? "unknown";
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        install_id: installId,
        provider_id: providerId,
        state: "succeeded",
        started_at: now,
        finished_at: now,
        last_event: null,
      }),
    });
  });
  await page.route("**/api/providers/install_all", (route) => {
    const payload = installable.map((p) => {
      const installId = `install-${p.provider_id}`;
      installIds.set(installId, p.provider_id);
      return { provider_id: p.provider_id, install_id: installId };
    });
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(payload),
    });
  });

  await page.getByRole("button", { name: "Install all" }).click();

  for (const provider of installable) {
    const card = page.locator("ul.list > li.card").filter({ hasText: provider.provider_id });
    await expect(card.getByText("Install: succeeded")).toBeVisible({ timeout: 20_000 });
  }
});
