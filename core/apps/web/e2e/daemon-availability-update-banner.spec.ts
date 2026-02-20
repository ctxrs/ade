import { test, expect } from "./fixtures";

const okHealth = {
  version: "1.0.0",
  daemon_version: "1.0.0",
  pid: 1,
  data_root: "/tmp",
  daemon_url: "http://127.0.0.1:4399",
  auth_required: false,
  compatibility: {
    desktop_exact_version: "1.0.0",
    mobile_api_min: 1,
    mobile_api_max: 1,
  },
};

test("update banner shows when updates are available", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.removeItem("ctx_update_check_v1");
  });
  await page.route("**/api/health", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(okHealth),
    });
  });
  await page.route("**/api/updates/check**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        channel: "stable",
        base_url: "https://example.com",
        current_version: "1.0.0",
        latest_version: "9.9.9",
        update_available: true,
      }),
    });
  });

  await page.goto("/workspaces", { waitUntil: "domcontentloaded" });
  await page.waitForResponse((response) => response.url().includes("/api/health") && response.status() === 200, {
    timeout: 20000,
  });
  await page.waitForResponse(
    (response) => response.url().includes("/api/updates/check") && response.status() === 200,
    { timeout: 20000 },
  );
  await expect(page.getByText(/Update available:\s*9\.9\.9\./)).toBeVisible({ timeout: 20000 });
});

test("daemon availability overlay appears on health failures", async ({ page }) => {
  await page.route("**/api/health", async (route) => {
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: "daemon down" }),
    });
  });

  await page.goto("/workspaces", { waitUntil: "domcontentloaded" });
  await expect(page.getByText("ctx daemon unavailable")).toBeVisible();
  await expect(
    page.getByText("The daemon is not reachable. Start it, then retry this screen."),
  ).toBeVisible();
});
