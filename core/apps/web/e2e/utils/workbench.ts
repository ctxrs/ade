import { expect } from "playwright/test";
import fs from "fs";
import path from "path";
import type { APIRequestContext, Page } from "playwright/test";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

type CreateWorkspaceArgs = {
  page: Page;
  request: APIRequestContext;
  repo: string;
  workspaceName: string;
  token?: string;
};

const readId = (v: unknown): string => (typeof v === "string" ? v : "");

type WorkspaceSummary = {
  id?: unknown;
};

const normalizePath = (value: string): string => {
  if (!value) return "";
  try {
    return fs.realpathSync(value);
  } catch {
    return path.resolve(value);
  }
};

export async function createWorkspaceAndOpenWorkbench(opts: CreateWorkspaceArgs): Promise<string> {
  const { page, request, repo, workspaceName, token } = opts;
  const repoPath = normalizePath(repo);
  const authToken = token ?? AUTH_TOKEN;
  const headers = authToken ? { authorization: `Bearer ${authToken}` } : undefined;
  const createResp = await request.post("/api/workspaces", {
    headers,
    data: {
      root_path: repoPath,
      name: workspaceName,
    },
  });
  expect(createResp.ok(), `failed to create workspace for ${repoPath}: ${createResp.status()}`).toBeTruthy();
  const created = (await createResp.json()) as WorkspaceSummary;
  const workspaceId = readId(created.id);
  expect(workspaceId).not.toBe("");

  const query = new URLSearchParams();
  if (authToken) query.set("token", authToken);
  const workspaceUrl = query.size > 0 ? `/workspaces/${workspaceId}?${query.toString()}` : `/workspaces/${workspaceId}`;
  await page.goto(workspaceUrl, { waitUntil: "domcontentloaded" });
  await expect(page).toHaveURL(new RegExp(`/workspaces/${workspaceId}(\\?.*)?$`), { timeout: 20_000 });
  await expect(page.locator(".wb-main")).toBeVisible({ timeout: 20_000 });

  return workspaceId;
}
