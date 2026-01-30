#!/usr/bin/env node
import fs from "node:fs/promises";
import process from "node:process";

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

function diffLines(expected, actual) {
  const exp = expected.split(/\r?\n/);
  const act = actual.split(/\r?\n/);
  const max = Math.max(exp.length, act.length);
  for (let i = 0; i < max; i += 1) {
    if (exp[i] !== act[i]) {
      return {
        line: i + 1,
        expected: exp[i] ?? "<missing>",
        actual: act[i] ?? "<missing>"
      };
    }
  }
  return null;
}

async function main() {
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

  let failed = 0;
  for (const name of files) {
    const inputPath = new URL(name, inputsDir);
    const expectedPath = new URL(name, expectedDir);

    let expectedText = null;
    try {
      expectedText = await fs.readFile(expectedPath, "utf8");
    } catch (err) {
      console.error(`missing expected fixture: ${expectedPath.pathname}`);
      failed += 1;
      continue;
    }

    const inputText = await fs.readFile(inputPath, "utf8");
    const records = parseJsonl(inputText);
    const events = translateClaudeEventsToCrp(records, {});
    const actualText = serializeJsonl(events);

    const diff = diffLines(expectedText, actualText);
    if (diff) {
      console.error(`fixture mismatch: ${name} (line ${diff.line})`);
      console.error(`expected: ${diff.expected}`);
      console.error(`actual:   ${diff.actual}`);
      failed += 1;
    }
  }

  if (failed > 0) {
    console.error(`fixture verification failed (${failed})`);
    process.exit(1);
  }

  console.log(`fixture verification passed (${files.length})`);
}

await main();
