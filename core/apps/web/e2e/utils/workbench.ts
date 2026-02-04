import { expect } from "playwright/test";
import fs from "fs";
import path from "path";

type CreateWorkspaceArgs = {
  page: any;
  request: any;
  repo: string;
  workspaceName: string;
  token?: string;
};

const readId = (v: any): string => (typeof v === "string" ? v : "");

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

  const url = token ? `/workspaces?token=${encodeURIComponent(token)}&desktop_ui=1` : "/workspaces";
  await page.goto(url);
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill(workspaceName);
  await page.getByRole("button", { name: "Add workspace" }).click();

  let workspaceId = "";
  await expect
    .poll(
      async () => {
        const workspacesResp = await request.get("/api/workspaces", {
          headers: token ? { authorization: `Bearer ${token}` } : undefined,
        });
        if (!workspacesResp.ok()) return "";
        const workspaces = (await workspacesResp.json()) as any[];
        const ws = workspaces.find(
          (w) => normalizePath(String(w?.root_path ?? "")) === repoPath,
        );
        workspaceId = readId(ws?.id);
        return workspaceId;
      },
      { timeout: 20_000 },
    )
    .not.toBe("");

  const workspaceLink = page
    .getByRole("listitem")
    .filter({ hasText: repo })
    .getByRole("link", { name: workspaceName });
  await expect(workspaceLink).toBeVisible({ timeout: 20_000 });
  await workspaceLink.click();
  await expect(page).toHaveURL(new RegExp(`/workspaces/${workspaceId}$`), { timeout: 20_000 });
  await expect(page.locator(".wb-main")).toBeVisible({ timeout: 20_000 });

  return workspaceId;
}
