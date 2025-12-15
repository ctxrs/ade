import { chromium } from "playwright";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const workspaceId = process.env.CONTEXT_WORKSPACE_ID || "c21034ed-c1ef-4287-9e0e-ee8dbf4d2a64";
const baseUrl = process.env.CONTEXT_BASE_URL || "http://127.0.0.1:4399";
const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../../../../../images");

fs.mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });

try {
  await page.goto(`${baseUrl}/workspaces/${workspaceId}`, { waitUntil: "networkidle" });

  // Ensure the "new agent" composer is visible even if a prior task is selected.
  const newAgent = page.getByRole("button", { name: "New Agent" });
  if (await newAgent.isVisible().catch(() => false)) {
    await newAgent.click();
  }

  await page.locator(".wb-new-composer-card").waitFor({ state: "visible" });
  await page.waitForTimeout(200);

  await page.screenshot({ path: path.join(outDir, "tmp-ui-new-session.png"), fullPage: true });

  // Open harness menu
  const switchers = page.locator(".wb-new-composer-card .wb-switcher");
  await switchers.nth(1).click();
  await page.waitForTimeout(150);
  await page.screenshot({ path: path.join(outDir, "tmp-ui-new-session-harness-menu.png"), fullPage: true });

  // Open mode menu
  await switchers.nth(0).click();
  await page.waitForTimeout(150);
  await page.screenshot({ path: path.join(outDir, "tmp-ui-new-session-mode-menu.png"), fullPage: true });

  // Open model menu (only if present)
  if (await switchers.nth(2).isVisible().catch(() => false)) {
    const waitOpts = page
      .waitForResponse((r) => r.url().includes("/providers/") && r.url().includes("/options") && r.status() === 200, {
        timeout: 3000,
      })
      .catch(() => null);
    await switchers.nth(2).click();
    await waitOpts;
    // If the provider already had options cached, the response might not happen; wait a bit for UI to populate.
    await page.waitForTimeout(250);
    await page.screenshot({ path: path.join(outDir, "tmp-ui-new-session-model-menu.png"), fullPage: true });
  }
} finally {
  await browser.close();
}

console.log(`Wrote screenshots to ${outDir}`);
