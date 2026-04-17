import { defineConfig } from "playwright/test";
import { createCtxPlaywrightConfig } from "./playwright.shared";

const baseURL = process.env.CTX_E2E_BASE_URL ?? "https://127.0.0.1:5173";
const authToken = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

process.env.CTX_E2E_BROWSER = "webkit";

const shared = await createCtxPlaywrightConfig("soak");

export default defineConfig({
  ...shared,
  use: {
    ...shared.use,
    browserName: "webkit",
    baseURL,
    ignoreHTTPSErrors: true,
    extraHTTPHeaders: {
      authorization: `Bearer ${authToken}`,
    },
  },
  webServer: undefined,
});
