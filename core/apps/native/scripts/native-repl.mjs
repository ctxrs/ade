#!/usr/bin/env node

import repl from "node:repl";
import { connect, expect } from "./native-driver.mjs";

const DEFAULT_HTTP_URL = "http://127.0.0.1:6160";
const DEFAULT_WS_URL = "ws://127.0.0.1:6160/ws";

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/native-repl.mjs [options]

Options:
  --http <url>      Automation HTTP base URL (default: ${DEFAULT_HTTP_URL})
  --ws <url>        Automation WS URL (default: ${DEFAULT_WS_URL} derived from --http)
  -h, --help        Show this help

REPL helpers:
  page, app, expect, help()
`);
}

function parseArgs(argv) {
  const args = {
    httpUrl: DEFAULT_HTTP_URL,
    wsUrl: undefined,
    showHelp: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      args.showHelp = true;
      continue;
    }
    if (arg === "--http") {
      if (i + 1 >= argv.length) {
        throw new Error("--http requires a value");
      }
      args.httpUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--http=")) {
      args.httpUrl = arg.slice("--http=".length);
      continue;
    }
    if (arg === "--ws") {
      if (i + 1 >= argv.length) {
        throw new Error("--ws requires a value");
      }
      args.wsUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--ws=")) {
      args.wsUrl = arg.slice("--ws=".length);
      continue;
    }

    throw new Error(`Unknown argument: ${arg}`);
  }

  return args;
}

function printReplHelp() {
  console.log(`Examples:
  const composer = page.locator("#composer");
  await composer.click();
  await composer.type("hello");
  await page.keyboard.press("Enter");
  await expect(page.locator("#status")).toHaveText("ready");
  await page.screenshot({ name: "workbench" });

Use app.rpc("method", { ... }) for raw JSON-RPC calls.
`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.showHelp) {
    printHelp();
    return;
  }

  let app;
  try {
    app = await connect({ httpUrl: args.httpUrl, wsUrl: args.wsUrl });
  } catch (err) {
    console.error(`Failed to connect: ${err.message}`);
    process.exitCode = 1;
    return;
  }

  const page = app.page;
  const replServer = repl.start({
    prompt: "ctx-native> ",
  });
  replServer.context.app = app;
  replServer.context.page = page;
  replServer.context.expect = expect;
  replServer.context.help = printReplHelp;

  printReplHelp();

  replServer.on("exit", async () => {
    await app.close();
  });
}

main().catch((err) => {
  console.error(err.message);
  process.exitCode = 1;
});
