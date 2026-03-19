import { defineConfig } from "playwright/test";

const baseURL = process.env.ANCHORSTREAM_ACCEPTANCE_BASE_URL ?? "https://127.0.0.1:5194";
const authToken =
  process.env.ANCHORSTREAM_WORKSPACE_TOKEN ??
  process.env.ANCHORSTREAM_AUTH_TOKEN ??
  process.env.CTX_E2E_AUTH_TOKEN ??
  "00000000-0000-4000-8000-000000000003";
const workers = Number(process.env.ANCHORSTREAM_ACCEPTANCE_WORKERS ?? "1");

export default defineConfig({
  testDir: "./e2e",
  timeout: 120_000,
  expect: {
    timeout: 20_000,
  },
  workers: Number.isFinite(workers) && workers > 0 ? workers : 1,
  outputDir: "./e2e/test-results/anchorstream-acceptance",
  reporter: [[process.env.CTX_E2E_REPORTER ?? "dot"]],
  use: {
    baseURL,
    headless: true,
    ignoreHTTPSErrors: true,
    extraHTTPHeaders: {
      authorization: `Bearer ${authToken}`,
    },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "off",
  },
});
