import { defineConfig } from "playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  use: {
    baseURL: "http://127.0.0.1:4399",
    headless: true,
  },
  webServer: {
    command:
      'bash -lc "pnpm -C apps/web build && cargo run -p context-http --bin context -- serve --bind 127.0.0.1:4399 --data-dir /tmp/context-e2e"',
    cwd: "../..",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
