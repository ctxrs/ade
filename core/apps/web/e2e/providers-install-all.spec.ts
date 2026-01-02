import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("providers: install all completes for supported providers", async ({ page, request }) => {
  test.setTimeout(15 * 60 * 1000);
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

  const installAllResp = page.waitForResponse(
    (resp) => resp.url().includes("/api/providers/install_all") && resp.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Install all" }).click();
  const installAll = await installAllResp;
  expect(installAll.ok()).toBeTruthy();
  const installs = (await installAll.json()) as Array<{ provider_id: string; install_id: string }>;
  expect(installs.length).toBeGreaterThan(0);

  await expect
    .poll(
      async () => {
        const infos = await Promise.all(
          installs.map(async (item) => {
            const resp = await request.get(`/api/providers/install/${item.install_id}`);
            if (!resp.ok()) {
              throw new Error(`install status ${item.provider_id}: ${resp.status()}`);
            }
            return (await resp.json()) as { provider_id: string; state: string; error?: string };
          }),
        );
        const failed = infos.find((info) => info.state === "failed");
        if (failed) {
          return `failed:${failed.provider_id}:${failed.error ?? "unknown"}`;
        }
        const running = infos.some((info) => info.state === "running");
        return running ? "running" : "succeeded";
      },
      { timeout: 14 * 60 * 1000, intervals: [2000, 5000, 8000, 12000] },
    )
    .toBe("succeeded");

  for (const install of installs) {
    const card = page.locator("ul.list > li.card").filter({ hasText: install.provider_id });
    await expect(card.getByText("Install: succeeded")).toBeVisible({ timeout: 60_000 });
  }
});
