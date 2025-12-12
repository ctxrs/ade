import { test, expect } from "@playwright/test";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import { execSync } from "child_process";

test("golden path: workspace → task → session → message", async ({ page }) => {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "file.txt"), "hello\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });

  await page.goto("/");
  await page.getByLabel("Root path").fill(repo);
  await page.getByLabel("Name (optional)").fill("ws");
  await page.getByRole("button", { name: "Add workspace" }).click();
  await page.getByRole("link", { name: "ws" }).click();

  await page.getByLabel("Task title").fill("task1");
  await page.getByRole("button", { name: "Create task" }).click();
  await page.getByRole("link", { name: "task1" }).click();

  await page.getByRole("button", { name: "Create session" }).click();
  await expect(page).toHaveURL(/sessions\/.*/);

  await page.getByPlaceholder("Send a message…").fill("hello");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("hello")).toBeVisible();
});

