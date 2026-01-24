import { expect } from "playwright/test";
import path from "path";

type CreateWorkspaceArgs = {
  page: any;
  request: any;
  repo: string;
  workspaceName: string;
  token?: string;
};

const readId = (v: any): string => {
  if (!v) return "";
  if (typeof v === "string") return v;
  if (typeof v === "object" && typeof v["0"] === "string") return v["0"];
  return "";
};

export async function createWorkspaceAndOpenWorkbench(opts: CreateWorkspaceArgs): Promise<string> {
  const { page, request, repo, workspaceName, token } = opts;

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
        const ws = workspaces.find((w) => path.resolve(String(w?.root_path ?? "")) === path.resolve(repo));
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
