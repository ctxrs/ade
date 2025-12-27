import { test, expect } from "playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench catchup snapshot+stream keeps network lean", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
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

  const workspaceId = await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });
  expect(workspaceId).not.toEqual("");

  const apiRequests = requests.filter((r) => r.url.includes("/api/"));
  expect(apiRequests.length).toBeLessThanOrEqual(30);

  const catchupRequests = apiRequests.filter((r) =>
    r.method === "GET" && r.url.includes(`/api/workspaces/${workspaceId}/catchup`),
  );
  expect(catchupRequests.length).toBeGreaterThanOrEqual(1);
  const tracksRequests = apiRequests.filter((r) => /\/api\/tasks\/[^/]+\/tracks/.test(r.url));
  const sessionsRequests = apiRequests.filter((r) => /\/api\/tracks\/[^/]+\/sessions/.test(r.url));
  expect(tracksRequests.length).toBe(0);
  expect(sessionsRequests.length).toBe(0);
});
