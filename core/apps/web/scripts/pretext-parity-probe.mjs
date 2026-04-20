import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";
import {
  resolveProbeWidths,
  selectProbeDebugWidth,
  shouldCleanupProbeScratchWorkspace,
  summarizeProbeMeasurements,
} from "./pretext-parity-probe-shared.mjs";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_REPO_ROOT = path.resolve(SCRIPT_DIR, "../../../..");

function parseArgs(argv) {
  const options = {
    kind: "assistant",
    browser: "chromium",
    width: 788,
    widths: "",
    widthRange: "",
    complete: true,
    expanded: true,
    baseUrl: process.env.CTX_E2E_BASE_URL ?? process.env.CTX_WEBAPP_URL ?? "http://127.0.0.1:4417",
    workspaceId: process.env.CTX_WORKSPACE_ID ?? "",
    repoRoot: process.env.CTX_REPO_ROOT ?? DEFAULT_REPO_ROOT,
    workspaceName: process.env.CTX_PROBE_WORKSPACE_NAME ?? "",
    token: process.env.CTX_E2E_AUTH_TOKEN ?? process.env.CTX_AUTH_TOKEN ?? "ctx-e2e-auth-token",
    content: "",
    contentFile: "",
    debug: false,
    debugWidth: "",
    driftThreshold: 1,
    debugTarget: "*",
    keepWorkspace: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--kind") {
      options.kind = argv[index + 1] ?? options.kind;
      index += 1;
      continue;
    }
    if (arg === "--width") {
      options.width = Number.parseFloat(argv[index + 1] ?? String(options.width));
      index += 1;
      continue;
    }
    if (arg === "--widths") {
      options.widths = argv[index + 1] ?? options.widths;
      index += 1;
      continue;
    }
    if (arg === "--width-range") {
      options.widthRange = argv[index + 1] ?? options.widthRange;
      index += 1;
      continue;
    }
    if (arg === "--browser") {
      options.browser = argv[index + 1] ?? options.browser;
      index += 1;
      continue;
    }
    if (arg === "--base-url") {
      options.baseUrl = argv[index + 1] ?? options.baseUrl;
      index += 1;
      continue;
    }
    if (arg === "--workspace-id") {
      options.workspaceId = argv[index + 1] ?? options.workspaceId;
      index += 1;
      continue;
    }
    if (arg === "--repo-root") {
      options.repoRoot = argv[index + 1] ?? options.repoRoot;
      index += 1;
      continue;
    }
    if (arg === "--workspace-name") {
      options.workspaceName = argv[index + 1] ?? options.workspaceName;
      index += 1;
      continue;
    }
    if (arg === "--token") {
      options.token = argv[index + 1] ?? options.token;
      index += 1;
      continue;
    }
    if (arg === "--content") {
      options.content = argv[index + 1] ?? options.content;
      index += 1;
      continue;
    }
    if (arg === "--content-file") {
      options.contentFile = argv[index + 1] ?? options.contentFile;
      index += 1;
      continue;
    }
    if (arg === "--incomplete") {
      options.complete = false;
      continue;
    }
    if (arg === "--collapsed") {
      options.expanded = false;
      continue;
    }
    if (arg === "--expanded") {
      options.expanded = true;
      continue;
    }
    if (arg === "--debug") {
      options.debug = true;
      continue;
    }
    if (arg === "--debug-width") {
      options.debugWidth = argv[index + 1] ?? options.debugWidth;
      index += 1;
      continue;
    }
    if (arg === "--drift-threshold") {
      options.driftThreshold = Number.parseFloat(argv[index + 1] ?? String(options.driftThreshold));
      index += 1;
      continue;
    }
    if (arg === "--keep-workspace") {
      options.keepWorkspace = true;
      continue;
    }
    if (arg === "--debug-target") {
      options.debugTarget = argv[index + 1] ?? options.debugTarget;
      index += 1;
      continue;
    }
    if (arg === "--help") {
      options.help = true;
    }
  }

  return options;
}

function printHelp() {
  process.stdout.write(`Usage: pnpm pretext:probe [options]

Options:
  --kind assistant|message|markdown
                              Probe assistant row parity, message row parity, or raw markdown parity.
  --browser chromium|webkit   Browser engine to use. Default: chromium.
  --width <px>                Single width to measure. Default: 788.
  --widths <a,b,c>            Measure multiple explicit widths.
  --width-range <a:b:step>    Measure an inclusive width range.
  --base-url <url>            Dev webapp URL. Default: CTX_E2E_BASE_URL, CTX_WEBAPP_URL, or http://127.0.0.1:4417.
  --workspace-id <id>         Workspace to open in the dev webapp. Optional when creating a scratch probe workspace.
  --repo-root <path>          Repo root for scratch workspace creation. Default: current repo root.
  --workspace-name <name>     Optional scratch workspace name override.
  --token <token>             Auth token. Default: CTX_E2E_AUTH_TOKEN, CTX_AUTH_TOKEN, or ctx-e2e-auth-token.
  --content <text>            Inline content to measure.
  --content-file <path>       Read content from a file instead of stdin.
  --incomplete                Measure assistant content as streaming/incomplete.
  --collapsed                 Measure message content in collapsed mode.
  --expanded                  Measure message content in expanded mode. Default.
  --debug                     Return planner debug for markdown probes.
  --debug-width <px>          Force the debug rerun width when scanning multiple widths.
  --drift-threshold <px>      Threshold used for scan summaries. Default: 1.
  --debug-target <text>       Limit inline-code debug to a matching code run. Default: *.
  --keep-workspace            Keep an auto-created scratch workspace instead of deleting it.

Examples:
  pnpm pretext:probe --workspace-id <id> --kind assistant --content-file /tmp/message.md
  cat /tmp/message.md | pnpm pretext:probe --kind markdown --width-range 360:420:4 --debug
`);
}

function readContent(options) {
  if (options.content.length > 0) {
    return options.content;
  }
  if (options.contentFile.length > 0) {
    return fs.readFileSync(options.contentFile, "utf8");
  }
  return fs.readFileSync(0, "utf8");
}

async function createScratchWorkspace(options) {
  const createUrl = new URL("/api/workspaces", options.baseUrl);
  const headers = {
    "content-type": "application/json",
  };
  if (options.token) {
    headers.authorization = `Bearer ${options.token}`;
  }
  const response = await fetch(createUrl, {
    method: "POST",
    headers,
    body: JSON.stringify({
      root_path: options.repoRoot,
      name: options.workspaceName || `pretext-probe-${Date.now()}`,
    }),
  });
  if (!response.ok) {
    throw new Error(`Failed to create probe workspace: ${response.status} ${response.statusText}`);
  }
  const payload = await response.json();
  const workspaceId = typeof payload?.id === "string" ? payload.id : "";
  if (!workspaceId) {
    throw new Error("Probe workspace creation returned no workspace id.");
  }
  process.stderr.write(`Using scratch probe workspace ${workspaceId}\n`);
  return workspaceId;
}

async function deleteScratchWorkspace(options, workspaceId) {
  const deleteUrl = new URL(`/api/workspaces/${workspaceId}`, options.baseUrl);
  const headers = {};
  if (options.token) {
    headers.authorization = `Bearer ${options.token}`;
  }
  const response = await fetch(deleteUrl, {
    method: "DELETE",
    headers,
  });
  if (!response.ok) {
    throw new Error(`Failed to delete probe workspace ${workspaceId}: ${response.status} ${response.statusText}`);
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    return;
  }
  if (!Number.isFinite(options.driftThreshold) || options.driftThreshold < 0) {
    throw new Error(`Invalid --drift-threshold: ${options.driftThreshold}`);
  }

  const widths = resolveProbeWidths(options);
  const content = readContent(options);
  if (content.length === 0) {
    throw new Error("No content provided. Use --content, --content-file, or stdin.");
  }

  let scratchWorkspaceId = "";
  const workspaceId = options.workspaceId || (await createScratchWorkspace(options));
  if (!options.workspaceId) {
    scratchWorkspaceId = workspaceId;
  }
  const workspaceUrl = new URL(`/workspaces/${workspaceId}`, options.baseUrl);
  workspaceUrl.searchParams.set("ctxE2E", "1");
  if (options.token) {
    workspaceUrl.searchParams.set("token", options.token);
  }

  const browserType = options.browser === "webkit" ? webkit : chromium;
  const browser = await browserType.launch({ headless: true });
  try {
    let browserUserAgent = "";

    async function openProbePage(needsDebug) {
      const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
      await page.addInitScript(() => {
        window.sessionStorage.setItem("ctxE2E", "1");
      });
      await page.goto(workspaceUrl.toString(), { waitUntil: "domcontentloaded", timeout: 30000 });
      await page.waitForFunction(
        ({ kind, needsDebug }) => {
          const bridge = window.__ctxE2E;
          if (!bridge) return false;
          if (kind === "markdown") {
            return (
              typeof bridge.measureMarkdownParity === "function" &&
              (!needsDebug || typeof bridge.measureMarkdownParityDebug === "function")
            );
          }
          if (kind === "message") {
            return typeof bridge.measureMessageParity === "function";
          }
          return typeof bridge.measureAssistantParity === "function";
        },
        {
          kind: options.kind,
          needsDebug,
        },
        { timeout: 30000 },
      );
      if (!browserUserAgent) {
        browserUserAgent = await page.evaluate(() => navigator.userAgent);
      }
      return page;
    }

    async function measureAtWidth(width, debugEnabled) {
      const page = await openProbePage(debugEnabled);
      try {
        return await page.evaluate(
          async ({ kind, width, content, complete, expanded, debugEnabled, debugTarget }) => {
            if (kind === "markdown") {
              if (debugEnabled) {
                return await window.__ctxE2E.measureMarkdownParityDebug?.(content, width, debugTarget);
              }
              return (await window.__ctxE2E.measureMarkdownParity?.([{ name: "probe", markdown: content }], width))
                ?.[0];
            }
            if (kind === "message") {
              return await window.__ctxE2E.measureMessageParity?.({
                content,
                viewportWidth: width,
                expanded,
              });
            }

            return await window.__ctxE2E.measureAssistantParity?.({
              content,
              viewportWidth: width,
              isComplete: complete,
            });
          },
          {
            kind: options.kind,
            width,
            content,
            complete: options.complete,
            expanded: options.expanded,
            debugEnabled,
            debugTarget: options.debugTarget,
          },
        );
      } finally {
        await page.close();
      }
    }

    if (widths.length === 1) {
      const measurement = await measureAtWidth(
        widths[0],
        options.kind === "markdown" && options.debug,
      );
      const result = {
        kind: options.kind,
        browser: browserUserAgent,
        width: widths[0],
        content,
        measurement,
      };
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
      return;
    }

    const measurements = [];
    for (const width of widths) {
      const measurement = await measureAtWidth(width, false);
      measurements.push({
        width,
        planned: measurement?.planned ?? null,
        actual: measurement?.actual ?? null,
        delta: measurement?.delta ?? null,
        viewportWidth: measurement?.viewportWidth ?? undefined,
        rowWidth: measurement?.rowWidth ?? undefined,
      });
    }

    const summary = summarizeProbeMeasurements(measurements, options.driftThreshold);
    const debugWidth =
      options.kind === "markdown" && options.debug
        ? selectProbeDebugWidth(measurements, options.debugWidth || null)
        : null;
    const debugMeasurement =
      debugWidth != null ? await measureAtWidth(debugWidth, true) : null;

    process.stdout.write(
      `${JSON.stringify(
        {
          kind: options.kind,
          browser: browserUserAgent,
          widths,
          content,
          measurements,
          summary,
          debugWidth,
          debugMeasurement,
        },
        null,
        2,
      )}\n`,
    );
  } finally {
    await browser.close();
    if (shouldCleanupProbeScratchWorkspace(options, scratchWorkspaceId)) {
      try {
        await deleteScratchWorkspace(options, scratchWorkspaceId);
      } catch (error) {
        process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
      }
    }
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
