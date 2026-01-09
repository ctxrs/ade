#!/usr/bin/env node

import { connect, expect } from "./native-driver.mjs";

const DEFAULT_HTTP_URL = process.env.CTX_NATIVE_HTTP ?? "http://127.0.0.1:6160";
const DEFAULT_WS_URL =
  process.env.CTX_NATIVE_WS ?? "ws://127.0.0.1:6160/ws";

function parseArgs(argv) {
  const args = { httpUrl: DEFAULT_HTTP_URL, wsUrl: DEFAULT_WS_URL };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--http") {
      args.httpUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--http=")) {
      args.httpUrl = arg.slice("--http=".length);
      continue;
    }
    if (arg === "--ws") {
      args.wsUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--ws=")) {
      args.wsUrl = arg.slice("--ws=".length);
      continue;
    }
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));
const app = await connect({ httpUrl: args.httpUrl, wsUrl: args.wsUrl });

const page = app.page;

const composer = page.locator("#composer-input");
await composer.click();
await composer.type("hello from native automation");
await page.keyboard.press("Enter");
await expect(composer).toBeVisible();
await page.screenshot({ name: "native-example" });

await app.close();
