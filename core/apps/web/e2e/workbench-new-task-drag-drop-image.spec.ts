import { test, expect } from "./fixtures";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";
import { createWorkspaceAndOpenWorkbench } from "./utils/workbench";

test("workbench: New Task composer accepts drag-dropped images after switching from a session", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  const workspaceName = `ws-${Date.now()}`;

  await createWorkspaceAndOpenWorkbench({ page, request: page.request, repo, workspaceName });

  // Choose Fake harness so the test doesn't depend on external agents.
  await page.locator(".wb-new-composer-stack").getByTitle("Harness").click();
  await page.locator(".wb-harness-menu").getByLabel("Search agents").fill("fake");
  await page.locator(".wb-harness-menu").getByRole("button", { name: /fake/i }).click();

  await page.locator(".wb-new-composer-stack textarea.wb-composer-textarea").fill("hello 1");
  await page.locator(".wb-new-composer-stack button[aria-label=\"Send\"]").click();
  await expect(page.locator(".wb-session .wb-assistant-entry")).toHaveCount(1, { timeout: 20000 });

  // Switch into the New Task composer (this previously caused the drop scope to never register).
  await page.getByRole("button", { name: "New Task" }).click();
  const newTaskTextarea = page.locator(".wb-new-composer-stack textarea.wb-composer-textarea");
  await expect(newTaskTextarea).toBeVisible({ timeout: 20000 });

  await newTaskTextarea.evaluate((el) => {
    const base64Png =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+lmZYAAAAASUVORK5CYII=";
    const bytes = Uint8Array.from(atob(base64Png), (c) => c.charCodeAt(0));
    const file = new File([bytes], "drop.png", { type: "image/png" });
    const dt = new DataTransfer();
    dt.items.add(file);

    const rect = el.getBoundingClientRect();
    const clientX = Math.floor(rect.left + rect.width / 2);
    const clientY = Math.floor(rect.top + rect.height / 2);

    el.dispatchEvent(
      new DragEvent("dragover", {
        bubbles: true,
        cancelable: true,
        clientX,
        clientY,
        dataTransfer: dt,
      }),
    );
    el.dispatchEvent(
      new DragEvent("drop", {
        bubbles: true,
        cancelable: true,
        clientX,
        clientY,
        dataTransfer: dt,
      }),
    );
  });

  await expect(page.locator(".wb-new-composer-stack .wb-attach-thumb-img")).toHaveCount(1, { timeout: 20000 });
});
