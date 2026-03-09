import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

import { createCtxPlaywrightConfig } from "./playwright.shared";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const restoreEnv = (snapshot: Record<string, string | undefined>) => {
  for (const [key, value] of Object.entries(snapshot)) {
    if (value === undefined) {
      delete process.env[key];
      continue;
    }
    process.env[key] = value;
  }
};

test("playwright shared config writes reports under e2e artifact roots", async () => {
  const trackedKeys = [
    "CTX_E2E_PORT",
    "CTX_E2E_DATA_DIR",
    "CTX_E2E_AUTH_TOKEN",
    "CTX_BUNDLE_DIR",
    "CTX_E2E_BUNDLED_ONLY",
  ];
  const snapshot = Object.fromEntries(trackedKeys.map((key) => [key, process.env[key]]));

  try {
    delete process.env.CTX_E2E_PORT;
    delete process.env.CTX_E2E_DATA_DIR;
    delete process.env.CTX_E2E_AUTH_TOKEN;

    const config = await createCtxPlaywrightConfig("all");
    expect(config.outputDir).toBe(path.resolve(__dirname, "e2e/test-results/all"));

    const reporters = Array.isArray(config.reporter) ? config.reporter : [];
    const htmlReporter = reporters.find(
      (entry) => Array.isArray(entry) && entry[0] === "html",
    );
    expect(Array.isArray(htmlReporter)).toBe(true);
    expect((htmlReporter as [string, { outputFolder?: string }])[1]?.outputFolder).toBe(
      path.resolve(__dirname, "e2e/playwright-report/all"),
    );
  } finally {
    restoreEnv(snapshot);
  }
});
