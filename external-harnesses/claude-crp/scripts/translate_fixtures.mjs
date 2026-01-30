#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";

// Capture new fixtures with scripts/capture.mjs, then run this script with --update.


function parseArgs(argv) {
  const args = new Set(argv);
  return {
    update: args.has("--update")
  };
}

async function loadTranslate() {
  const translatePath = new URL("../src/translate.ts", import.meta.url);
  const code = await fs.readFile(translatePath, "utf8");
  const dataUrl = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
  return import(dataUrl);
}

function parseJsonl(text) {
  return text
    .split(/\r?\n/)
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line));
}

function serializeJsonl(events) {
  if (!events.length) return "";
  return events.map((event) => JSON.stringify(event)).join("\n") + "\n";
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const inputsDir = new URL("../fixtures/inputs/", import.meta.url);
  const expectedDir = new URL("../fixtures/expected/", import.meta.url);

  const entries = await fs.readdir(inputsDir, { withFileTypes: true });
  const files = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".jsonl"))
    .map((entry) => entry.name)
    .sort();

  if (!files.length) {
    console.error("No fixture inputs found.");
    process.exit(1);
  }

  const { translateClaudeEventsToCrp } = await loadTranslate();
  await fs.mkdir(expectedDir, { recursive: true });

  let wrote = 0;
  for (const name of files) {
    const inputPath = new URL(name, inputsDir);
    const inputText = await fs.readFile(inputPath, "utf8");
    const records = parseJsonl(inputText);

    const events = translateClaudeEventsToCrp(records, {});
    const outputText = serializeJsonl(events);

    if (args.update) {
      const outPath = new URL(name, expectedDir);
      await fs.writeFile(outPath, outputText, "utf8");
      wrote += 1;
    }
  }

  if (args.update) {
    console.log(`wrote ${wrote} fixture snapshot(s)`);
  } else {
    console.log(`translated ${files.length} fixture(s) (use --update to write expected)`);
  }
}

await main();
