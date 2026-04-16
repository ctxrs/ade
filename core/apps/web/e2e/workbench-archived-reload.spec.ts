import { test, expect } from "./fixtures";
import { seedDummyWorkspace } from "./utils/seedDummyWorkspace";

test("workbench: archived tasks load after reload", async ({ page, request }) => {
  const seed = await seedDummyWorkspace(request, {
    tasks: 2,
    sessionsPerTask: 0,
    turnsPerSession: 0,
    throttleMs: 0,
  });

  await page.goto(`/workspaces/${seed.workspaceId}`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1000);

  for (const taskId of seed.taskIds) {
    const resp = await request.post(`/api/tasks/${taskId}/archive`, {});
    expect(resp.ok()).toBeTruthy();
  }

  await page.waitForTimeout(3000);
  await page.reload({ waitUntil: "domcontentloaded" });
  const archivedToggle = page.getByRole("button", { name: "Archived Tasks" });
  await archivedToggle.waitFor({ timeout: 10000 });
  const expanded = await archivedToggle.getAttribute("aria-expanded");
  if (expanded === "true") {
    await archivedToggle.click();
  }
  await archivedToggle.click();

  await expect(page.locator(".wb-task-row-archived", { hasText: "fixture task 1" })).toBeVisible({
    timeout: 20000,
  });
});
