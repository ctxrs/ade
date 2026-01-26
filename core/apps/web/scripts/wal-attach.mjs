import fs from "fs";
import os from "os";
import path from "path";
import { chromium } from "playwright";

const cdpUrl = process.env.CTX_CDP_URL ?? "http://127.0.0.1:9222";
const urlHint = process.argv[2] ?? process.env.CTX_WAL_TARGET ?? "";
const outDir = process.env.CTX_WAL_ATTACH_DIR ?? path.join(os.tmpdir(), "ctx-web-wal");
const dumpLimit = Number.parseInt(process.env.CTX_WAL_DUMP_LIMIT ?? "2000", 10);

const nowMs = () => Date.now();
const stamp = new Date().toISOString().replace(/[:.]/g, "-");

const normalizeUrl = (raw) => {
  try {
    const url = new URL(raw);
    const cleanParams = new URLSearchParams();
    for (const key of url.searchParams.keys()) {
      cleanParams.append(key, "");
    }
    const search = cleanParams.toString();
    return `${url.origin}${url.pathname}${search ? `?${search}` : ""}`;
  } catch {
    return raw;
  }
};

const writeLine = (stream, payload) => {
  stream.write(`${JSON.stringify(payload)}\n`);
};

fs.mkdirSync(outDir, { recursive: true });
const outFile = path.join(outDir, `ctx-web-wal-attach-${stamp}.jsonl`);
const outStream = fs.createWriteStream(outFile, { flags: "a" });

console.log(`[wal] attaching via CDP: ${cdpUrl}`);

let browser;
try {
  browser = await chromium.connectOverCDP(cdpUrl);
} catch (err) {
  console.error(`[wal] failed to connect to CDP at ${cdpUrl}`);
  console.error(
    "[wal] launch Chrome with --remote-debugging-port=9222 (and a dedicated --user-data-dir), then retry",
  );
  console.error(err?.message ?? String(err));
  process.exit(1);
}
let targetPage = null;

for (const context of browser.contexts()) {
  for (const page of context.pages()) {
    if (!urlHint || page.url().includes(urlHint)) {
      targetPage = page;
      break;
    }
  }
  if (targetPage) break;
}

if (!targetPage) {
  console.error(`[wal] no page matched "${urlHint || "any"}"`);
  await browser.close();
  process.exit(1);
}

console.log(`[wal] attached to ${targetPage.url()}`);
writeLine(outStream, {
  kind: "wal:attach",
  ts_ms: nowMs(),
  cdp_url: cdpUrl,
  page_url: targetPage.url(),
  out_file: outFile,
});

try {
  await targetPage.waitForFunction(() => Boolean(window.__CTX_WAL__), { timeout: 10000 });
} catch {
  console.warn("[wal] window.__CTX_WAL__ not found; skipping buffer dump");
}

const walStatus = await targetPage.evaluate(() => window.__CTX_WAL__?.getStatus?.() ?? null);
if (walStatus) {
  writeLine(outStream, { kind: "wal:status", ts_ms: nowMs(), data: walStatus });
  const buffer = await targetPage.evaluate(
    (limit) => window.__CTX_WAL__?.dump?.({ limit }) ?? [],
    Number.isFinite(dumpLimit) ? dumpLimit : 2000,
  );
  for (const event of buffer) {
    writeLine(outStream, { source: "wal", ...event });
  }
  await targetPage.evaluate(() => window.__CTX_WAL__?.flush?.("manual"));
}

const redactHeaders = (headers) => {
  const blocked = new Set(["authorization", "cookie", "set-cookie", "x-api-key", "x-supabase-key"]);
  const out = {};
  for (const [key, value] of Object.entries(headers ?? {})) {
    const lower = key.toLowerCase();
    out[key] = blocked.has(lower) ? "<redacted>" : String(value ?? "");
  }
  return out;
};

targetPage.on("console", (msg) => {
  writeLine(outStream, {
    kind: "pw:console",
    ts_ms: nowMs(),
    level: msg.type(),
    text: msg.text(),
    location: msg.location(),
  });
});

targetPage.on("pageerror", (err) => {
  writeLine(outStream, {
    kind: "pw:pageerror",
    ts_ms: nowMs(),
    message: err.message,
    stack: err.stack,
  });
});

targetPage.on("request", (req) => {
  writeLine(outStream, {
    kind: "pw:request",
    ts_ms: nowMs(),
    url: normalizeUrl(req.url()),
    method: req.method(),
    resource_type: req.resourceType(),
    headers: redactHeaders(req.headers()),
  });
});

targetPage.on("response", async (res) => {
  const req = res.request();
  writeLine(outStream, {
    kind: "pw:response",
    ts_ms: nowMs(),
    url: normalizeUrl(req.url()),
    method: req.method(),
    status: res.status(),
    ok: res.ok(),
    headers: redactHeaders(res.headers()),
  });
});

targetPage.on("requestfailed", (req) => {
  writeLine(outStream, {
    kind: "pw:requestfailed",
    ts_ms: nowMs(),
    url: normalizeUrl(req.url()),
    method: req.method(),
    failure: req.failure(),
  });
});

const shutdown = async () => {
  writeLine(outStream, { kind: "wal:detach", ts_ms: nowMs() });
  outStream.end();
  await browser.close();
};

process.on("SIGINT", () => {
  shutdown().finally(() => process.exit(0));
});

process.on("SIGTERM", () => {
  shutdown().finally(() => process.exit(0));
});

console.log(`[wal] writing to ${outFile} (Ctrl+C to stop)`);
