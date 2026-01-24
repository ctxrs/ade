import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("workbench active snapshot stream keeps network lean", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;
  const requests: { url: string; method: string }[] = [];
  page.on("requestfinished", (req) => {
    const url = req.url();
    if (!url.includes("/api/")) return;
    requests.push({ url, method: req.method() });
  });

  await page.goto("/");
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill(workspaceName);
  await page.getByRole("button", { name: "Add workspace" }).click();
  const workspaceLink = page
    .getByRole("listitem")
    .filter({ hasText: repo })
    .getByRole("link", { name: workspaceName });
  const snapshotPromise = page.waitForResponse((resp) =>
    /\/api\/workspaces\/[^/]+\/active_snapshot/.test(resp.url()),
  );
  await workspaceLink.click();
  await page.waitForURL(/\/workspaces\/[^/]+$/);
  await snapshotPromise;

  const url = new URL(page.url());
  const workspaceId = url.pathname.split("/").pop() ?? "";
  expect(workspaceId).not.toEqual("");

  const apiRequests = requests.filter((r) => r.url.includes("/api/"));
  expect(apiRequests.length).toBeLessThanOrEqual(30);

  const snapshotRequests = apiRequests.filter((r) =>
    r.method === "GET" && r.url.includes(`/api/workspaces/${workspaceId}/active_snapshot`),
  );
  expect(snapshotRequests.length).toBeGreaterThanOrEqual(1);
  const trackRequests = apiRequests.filter(
    (r) => /\/api\/tasks\/[^/]+\/tracks/.test(r.url) || /\/api\/tracks\/[^/]+/.test(r.url),
  );
  expect(trackRequests.length).toBe(0);
});
