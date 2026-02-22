#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runAcp } from "../server.js";

const argv = process.argv.slice(2);
if (argv.includes("--version") || argv.includes("-v")) {
  const here = dirname(fileURLToPath(import.meta.url));
  const packagePath = resolve(here, "../../package.json");
  const raw = readFileSync(packagePath, "utf8");
  const json = JSON.parse(raw) as { version?: string };
  const version = json.version ?? "0.0.0";
  process.stdout.write(`${version}\n`);
  process.exit(0);
}

console.log = console.error;
console.info = console.error;
console.warn = console.error;
console.debug = console.error;

process.on("unhandledRejection", (reason) => {
  console.error("Unhandled rejection:", reason);
});

runAcp();
process.stdin.resume();
