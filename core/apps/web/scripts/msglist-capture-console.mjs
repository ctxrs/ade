import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

function ts() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}_${pad(d.getHours())}${pad(d.getMinutes())}${pad(
    d.getSeconds(),
  )}`;
}

// Usage:
//   node core/apps/web/scripts/msglist-capture-console.mjs "<workspaceUrl>" [taskId] [sessionId]
// Example:
//   node core/apps/web/scripts/msglist-capture-console.mjs "https://192.0.2.75:5173/workspaces/...?...&debug=1"
const url = process.argv[2];
const targetTaskId = process.argv[3] || null;
const targetSessionId = process.argv[4] || null;
if (!url) {
  // eslint-disable-next-line no-console
  console.error("missing url arg");
  process.exit(2);
}

const outPath = path.join("/tmp", `ctx_msglist_console_${ts()}.jsonl`);
const out = fs.createWriteStream(outPath, { flags: "wx" });

function write(entry) {
  out.write(`${JSON.stringify(entry)}\n`);
}

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1200, height: 750 },
});
// Enable __ctxE2E hooks without relying on query params.
await context.addInitScript(() => {
  try {
    window.sessionStorage.setItem("ctxE2E", "1");
  } catch {
    // ignore
  }
});
const page = await context.newPage();

page.on("console", async (msg) => {
  const text = msg.text();
  if (!text.includes("[MessageList]")) return;
  const args = [];
  for (const arg of msg.args()) {
    try {
      // jsonValue() works for the structured objects we log (string + plain object).
      args.push(await arg.jsonValue());
    } catch (e) {
      args.push({ __unserializable: true });
    }
  }
  write({ t: Date.now(), type: msg.type(), text, args });
});
page.on("pageerror", (err) => {
  write({ t: Date.now(), type: "pageerror", text: String(err?.stack ?? err) });
});

// Network inspection: log head/snapshot/history requests; delay the first head response for the target session
// so we can inspect what rendered before head hydration arrives.
const targetSid = targetSessionId ? String(targetSessionId) : null;
let delayedHeadOnce = false;
await page.route("**/api/sessions/**", async (route) => {
  const req = route.request();
  const url = req.url();
  const m = url.match(/\/api\/sessions\/([^/]+)\/(head|snapshot|history|events)/);
  if (!m) return route.continue();
  const sid = m[1];
  const kind = m[2];
  if (targetSid && sid !== targetSid) return route.continue();

  const referer = (req.headers()["referer"] || req.headers()["referrer"] || "").toString();
  write({ t: Date.now(), type: "net:req", kind, sid, url, method: req.method(), referer });

  const shouldDelay = kind === "head" && targetSid && !delayedHeadOnce;
  if (shouldDelay) delayedHeadOnce = true;

  const resp = await route.fetch();
  let json = null;
  try {
    json = await resp.json();
  } catch {
    // ignore
  }
  if (json && typeof json === "object") {
    const turns = Array.isArray(json.turns) ? json.turns.length : null;
    const msgs = Array.isArray(json.messages) ? json.messages.length : null;
    const evs = Array.isArray(json.events) ? json.events.length : null;
    const tools = Array.isArray(json.tool_summaries) ? json.tool_summaries.length : null;
    const hw = json.head_window || null;
    write({ t: Date.now(), type: "net:resp", kind, sid, status: resp.status(), turns, msgs, evs, tools, head_window: hw });
  } else {
    write({ t: Date.now(), type: "net:resp", kind, sid, status: resp.status() });
  }

  if (shouldDelay) {
    // Capture what the thread currently rendered before head hydration lands.
    const rendered = await page.evaluate(() => {
      const ids = Array.from(document.querySelectorAll("[data-thread-item-id]"))
        .map((el) => el.getAttribute("data-thread-item-id"))
        .filter(Boolean);
      return { count: ids.length, sample: ids.slice(0, 25) };
    }).catch(() => null);
    write({ t: Date.now(), type: "inspect:before-head", sid, rendered });
    await new Promise((r) => setTimeout(r, 1500));
  }

  return route.fulfill({ response: resp, json: json ?? undefined });
});

write({ t: Date.now(), type: "meta", text: "goto", url });
await page.goto(url, { waitUntil: "domcontentloaded" });

// If we were given target ids, click tasks until the active tab matches.
if (targetTaskId || targetSessionId) {
  const short = (v) => String(v || "").slice(0, 8);
  const wantTask = short(targetTaskId);
  const wantSession = short(targetSessionId);

  await page.waitForSelector(".wb-task-row", { timeout: 20000 });
  const rows = await page.$$(".wb-task-row");
  for (let i = 0; i < rows.length; i += 1) {
    await rows[i].click();
    // Wait for the session slot input to appear (task focused).
    await page
      .waitForSelector('.wb-session-slot[aria-hidden="false"] textarea.wb-active-textarea', { timeout: 20000 })
      .catch(() => {});

    const label = await page
      .$eval(".wb-topbar-ids", (el) => (el ? el.textContent || "" : ""))
      .catch(() => "");
    if (!label) continue;

    const okTask = wantTask ? label.includes(`task:${wantTask}`) : true;
    const okSession = wantSession ? label.includes(`session:${wantSession}`) : true;
    write({ t: Date.now(), type: "meta", text: "taskFocus", i, label, okTask, okSession });
    if (okTask && okSession) break;
  }
}

// Best-effort: find the MessageList scroller element (Virtuoso owns scroll; classnames are not stable).
await page.waitForSelector(".wb-thread-stack", { timeout: 20000 });
const scrollerHandle = await page.evaluateHandle(() => {
  const root = document.querySelector(".wb-thread-stack") ?? document.body;
  const nodes = [root, ...Array.from(root.querySelectorAll("*"))];
  for (const node of nodes) {
    if (!(node instanceof HTMLElement)) continue;
    const style = window.getComputedStyle(node);
    if (style.overflowY !== "auto" && style.overflowY !== "scroll") continue;
    if ((node.scrollHeight ?? 0) - (node.clientHeight ?? 0) > 40) {
      return node;
    }
  }
  // Fall back to the document scroller if Virtuoso used it.
  return document.scrollingElement;
});
const scroller = scrollerHandle.asElement();
if (scroller) {
  const dims = await scroller.evaluate((el) => ({
    sh: el.scrollHeight ?? 0,
    ch: el.clientHeight ?? 0,
    oy: window.getComputedStyle(el).overflowY,
  }));
  write({ t: Date.now(), type: "meta", text: "foundScroller", dims });

  // Ensure we start at bottom.
  await scroller.evaluate((el) => {
    el.scrollTop = Math.max(0, (el.scrollHeight ?? 0) - (el.clientHeight ?? 0));
    el.dispatchEvent(new Event("scroll"));
  });
  await page.waitForTimeout(250);

  await scroller.hover().catch(() => {});
  // Wheel up a bunch to trigger history.
  for (let i = 0; i < 120; i += 1) {
    await page.mouse.wheel(0, -240);
    await page.waitForTimeout(50);
    const top = await scroller.evaluate((el) => el.scrollTop);
    if (top <= 1) break;
  }
} else {
  write({ t: Date.now(), type: "meta", text: "noScrollerFound" });
}

// Let any follow-up hydration settle.
await page.waitForTimeout(3000);

if (targetSid) {
  const e2e = await page
    .evaluate((sid) => {
      const w = window;
      const api = w.__ctxE2E;
      if (!api) return { ok: false };
      const msgs = typeof api.getSessionHeadMessages === "function" ? api.getSessionHeadMessages(sid) : null;
      const lastSeq = typeof api.getSessionLastEventSeq === "function" ? api.getSessionLastEventSeq(sid) : null;
      return { ok: true, headMessagesLen: Array.isArray(msgs) ? msgs.length : null, lastEventSeq: lastSeq ?? null };
    }, targetSid)
    .catch(() => null);
  write({ t: Date.now(), type: "inspect:e2e", sid: targetSid, e2e });
  const renderedAfter = await page.evaluate(() => {
    const ids = Array.from(document.querySelectorAll("[data-thread-item-id]"))
      .map((el) => el.getAttribute("data-thread-item-id"))
      .filter(Boolean);
    return { count: ids.length, sample: ids.slice(0, 25) };
  }).catch(() => null);
  write({ t: Date.now(), type: "inspect:after", sid: targetSid, rendered: renderedAfter });
}

write({ t: Date.now(), type: "meta", text: "done", outPath });
out.end();
await browser.close();

// eslint-disable-next-line no-console
console.log(outPath);
