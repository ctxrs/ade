#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { randomUUID } from "node:crypto";
import { query } from "@anthropic-ai/claude-agent-sdk";
import sdkPkg from "@anthropic-ai/claude-agent-sdk/package.json" with { type: "json" };

function parseArgs(argv) {
  const out = {};
  let i = 0;
  while (i < argv.length) {
    const arg = argv[i];
    if (!arg.startsWith("--")) {
      i += 1;
      continue;
    }
    const key = arg.slice(2);
    const next = argv[i + 1];
    if (!next || next.startsWith("--")) {
      out[key] = true;
      i += 1;
      continue;
    }
    out[key] = next;
    i += 2;
  }
  return out;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const prompt = args.prompt || "";
  if (!prompt) {
    console.error("usage: capture.mjs --prompt <text> [--scenario name] [--out path] [--err path]");
    process.exit(2);
  }

  const scenario = args.scenario || "capture";
  const cwd = args.cwd || process.cwd();
  const outPath = path.resolve(args.out || `./fixtures/inputs/${scenario}.jsonl`);
  const errPath = path.resolve(args.err || `./fixtures/inputs/${scenario}.stderr`);
  const timeoutSec = args["timeout-sec"] ? Number(args["timeout-sec"]) : 180;
  const sessionId = args["session-id"] || randomUUID();
  const model = args.model || null;
  const includePartials = args["include-partials"] !== "false";
  const interruptAfterMs = args["interrupt-after-ms"] ? Number(args["interrupt-after-ms"]) : null;

  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.mkdirSync(path.dirname(errPath), { recursive: true });

  const outFile = fs.createWriteStream(outPath, { encoding: "utf8" });
  const errFile = fs.createWriteStream(errPath, { encoding: "utf8" });

  const header = {
    record: "header",
    scenario,
    prompt,
    model,
    session_id: sessionId,
    sdk_version: sdkPkg.version,
    started_at: new Date().toISOString(),
    options: {
      cwd,
      includePartialMessages: includePartials,
      permissionMode: "default",
      settingSources: ["user", "project", "local"],
      allowDangerouslySkipPermissions: true,
      tools: { type: "preset", preset: "claude_code" }
    }
  };
  outFile.write(JSON.stringify(header) + "\n");

  const options = {
    cwd,
    includePartialMessages: includePartials,
    settingSources: ["user", "project", "local"],
    allowDangerouslySkipPermissions: true,
    permissionMode: "default",
    tools: { type: "preset", preset: "claude_code" },
    extraArgs: { "session-id": sessionId },
    stderr: (err) => {
      errFile.write(String(err));
      if (!String(err).endsWith("\n")) {
        errFile.write("\n");
      }
    },
    canUseTool: async () => ({ behavior: "allow" })
  };

  if (model) {
    options.model = model;
  }

  const q = query({ prompt, options });

  let seq = 0;
  let stopped = false;
  let timedOut = false;
  let interrupted = false;
  const start = Date.now();

  const interruptTimer = interruptAfterMs
    ? setTimeout(async () => {
        interrupted = true;
        try {
          await q.interrupt();
        } catch (err) {
          errFile.write(`interrupt error: ${err}\n`);
        }
      }, interruptAfterMs)
    : null;

  try {
    while (true) {
      if ((Date.now() - start) / 1000 > timeoutSec) {
        timedOut = true;
        try {
          await q.interrupt();
        } catch (err) {
          errFile.write(`timeout interrupt error: ${err}\n`);
        }
        break;
      }

      const { value, done } = await q.next();
      if (done || !value) {
        stopped = true;
        break;
      }

      const rec = { record: "event", seq: ++seq, event: value };
      outFile.write(JSON.stringify(rec) + "\n");
    }
  } catch (err) {
    errFile.write(`capture error: ${err}\n`);
  } finally {
    if (interruptTimer) clearTimeout(interruptTimer);
  }

  const footer = {
    record: "end",
    scenario,
    session_id: sessionId,
    stopped,
    timed_out: timedOut,
    interrupted,
    ended_at: new Date().toISOString()
  };
  outFile.write(JSON.stringify(footer) + "\n");
  outFile.end();
  errFile.end();

  console.log(`wrote ${outPath}`);
  console.log(`wrote ${errPath}`);
}

await main();
