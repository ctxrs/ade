import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

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

  const snapshotStreamPromise = page.waitForEvent("websocket", (ws) =>
    /\/api\/workspaces\/[^/]+\/active_snapshot\/stream/.test(ws.url()),
  );
  const workspaceId = await createWorkspaceAndOpenWorkbench({
    page,
    request: page.request,
    repo,
    workspaceName,
  });
  const snapshotStream = await snapshotStreamPromise;
  await expect.poll(() => requests.length).toBeGreaterThan(0);
  expect(workspaceId).not.toEqual("");

  const apiRequests = requests.filter((r) => r.url.includes("/api/"));
  expect(apiRequests.length).toBeLessThanOrEqual(30);

  const snapshotRequests = apiRequests.filter((r) => {
    if (r.method !== "GET") return false;
    const pathname = new URL(r.url).pathname;
    return pathname === `/api/workspaces/${workspaceId}/active_snapshot`;
  });
  expect(snapshotRequests.length).toBe(0);
  expect(snapshotStream.url()).toContain(`/api/workspaces/${workspaceId}/active_snapshot/stream`);
  const trackRequests = apiRequests.filter(
    (r) => /\/api\/tasks\/[^/]+\/tracks/.test(r.url) || /\/api\/tracks\/[^/]+/.test(r.url),
  );
  expect(trackRequests.length).toBe(0);
});
