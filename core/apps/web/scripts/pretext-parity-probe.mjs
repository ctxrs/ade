import fs from "node:fs";
import process from "node:process";
import { chromium } from "playwright";

function parseArgs(argv) {
  const options = {
    kind: "assistant",
    width: 788,
    complete: true,
    baseUrl: process.env.CTX_WEBAPP_URL ?? "http://127.0.0.1:5177",
    workspaceId: process.env.CTX_WORKSPACE_ID ?? "",
    token: process.env.CTX_AUTH_TOKEN ?? "",
    content: "",
    contentFile: "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--kind") {
      options.kind = argv[index + 1] ?? options.kind;
      index += 1;
      continue;
    }
    if (arg === "--width") {
      options.width = Number.parseInt(argv[index + 1] ?? String(options.width), 10);
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
    if (arg === "--help") {
      options.help = true;
    }
  }

  return options;
}

function printHelp() {
  process.stdout.write(`Usage: pnpm pretext:probe --workspace-id <id> [options]

Options:
  --kind assistant|markdown   Probe assistant row parity or raw markdown parity.
  --width <px>                Viewport/text width to measure. Default: 788.
  --base-url <url>            Dev webapp URL. Default: CTX_WEBAPP_URL or http://127.0.0.1:5177.
  --workspace-id <id>         Workspace to open in the dev webapp.
  --token <token>             Optional auth token for first load.
  --content <text>            Inline content to measure.
  --content-file <path>       Read content from a file instead of stdin.
  --incomplete                Measure assistant content as streaming/incomplete.

Examples:
  pnpm pretext:probe --workspace-id <id> --token <token> --kind assistant --content-file /tmp/message.md
  cat /tmp/message.md | pnpm pretext:probe --workspace-id <id> --kind markdown
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

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    return;
  }
  if (!options.workspaceId) {
    throw new Error("Missing --workspace-id (or CTX_WORKSPACE_ID).");
  }
  if (!Number.isFinite(options.width) || options.width <= 0) {
    throw new Error(`Invalid --width: ${options.width}`);
  }

  const content = readContent(options);
  if (content.length === 0) {
    throw new Error("No content provided. Use --content, --content-file, or stdin.");
  }

  const workspaceUrl = new URL(`/workspaces/${options.workspaceId}`, options.baseUrl);
  workspaceUrl.searchParams.set("ctxE2E", "1");
  if (options.token) {
    workspaceUrl.searchParams.set("token", options.token);
  }

  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    await page.goto(workspaceUrl.toString(), { waitUntil: "domcontentloaded", timeout: 30000 });
    await page.waitForFunction(() => Boolean(window.__ctxE2E), { timeout: 30000 });

    const result = await page.evaluate(
      async ({ kind, width, content, complete }) => {
        if (kind === "markdown") {
          const measurements = await window.__ctxE2E.measureMarkdownParity?.(
            [{ name: "probe", markdown: content }],
            width,
          );
          return {
            kind,
            width,
            content,
            measurement: measurements?.[0] ?? null,
          };
        }

        const measurement = await window.__ctxE2E.measureAssistantParity?.({
          content,
          viewportWidth: width,
          isComplete: complete,
        });
        return {
          kind,
          width,
          content,
          measurement,
        };
      },
      {
        kind: options.kind,
        width: options.width,
        content,
        complete: options.complete,
      },
    );

    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
