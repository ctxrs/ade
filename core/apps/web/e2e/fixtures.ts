import { test as base, expect, chromium } from "playwright/test";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

const test = base.extend({
  context: async ({ context }, use) => {
    await context.addInitScript((token: string) => {
      window.sessionStorage.setItem("ctxAuthToken", token);
    }, AUTH_TOKEN);
    await use(context);
  },
});

export { test, expect, chromium };
