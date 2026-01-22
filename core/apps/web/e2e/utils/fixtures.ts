import {
  test as base,
  expect,
  chromium,
  firefox,
  webkit,
  request as playwrightRequest,
  type APIRequestContext,
} from "playwright/test";
import { readFileSync } from "fs";
import path from "path";

const DATA_DIR = process.env.CTX_E2E_DATA_DIR ?? "";
const AUTH_FILENAME = "daemon_auth.json";
const AUTH_WAIT_MS = 10_000;
const AUTH_POLL_MS = 200;

let cachedAuthToken: string | null = null;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function readAuthToken(): Promise<string> {
  if (cachedAuthToken) return cachedAuthToken;
  if (!DATA_DIR) {
    throw new Error("CTX_E2E_DATA_DIR must be set to read daemon auth token");
  }
  const authPath = path.join(DATA_DIR, AUTH_FILENAME);
  const deadline = Date.now() + AUTH_WAIT_MS;
  while (Date.now() < deadline) {
    try {
      const raw = readFileSync(authPath, "utf8");
      const parsed = JSON.parse(raw);
      const token = String(parsed?.token ?? "").trim();
      if (token) {
        cachedAuthToken = token;
        return token;
      }
    } catch {
      // ignore and retry
    }
    await sleep(AUTH_POLL_MS);
  }
  throw new Error(`daemon auth token not found at ${authPath}`);
}

const test = base.extend<{ authToken: string }>({
  authToken: async ({}, use) => {
    const token = await readAuthToken();
    await use(token);
  },
  page: async ({ page, authToken }, use) => {
    if (authToken) {
      await page.context().setExtraHTTPHeaders({ Authorization: `Bearer ${authToken}` });
      await page.addInitScript((token) => {
        try {
          sessionStorage.setItem("ctxAuthToken", token as string);
        } catch {
          // ignore
        }
      }, authToken);
    }
    await use(page);
  },
  request: async ({ playwright }, use, testInfo) => {
    const token = await readAuthToken();
    const baseURL = testInfo.project.use.baseURL;
    const context = await playwright.request.newContext({
      baseURL,
      extraHTTPHeaders: token ? { Authorization: `Bearer ${token}` } : undefined,
    });
    await use(context);
    await context.dispose();
  },
});

export { test, expect, chromium, firefox, webkit, playwrightRequest as request };
export type { APIRequestContext };
