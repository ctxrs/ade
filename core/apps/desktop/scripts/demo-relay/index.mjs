import { createServer } from "node:http";
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

function parseArgs(argv) {
  const out = {
    mode: "proxy",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: null,
    scenarioPath: null,
    upstreamBaseUrl: "https://openrouter.ai/api/v1",
    upstreamApiKey: process.env.OPENROUTER_API_KEY || "",
    rewriteFrom: "",
    rewriteTo: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--mode") {
      out.mode = next;
      i += 1;
    } else if (arg === "--host") {
      out.listenHost = next;
      i += 1;
    } else if (arg === "--port") {
      out.listenPort = Number(next);
      i += 1;
    } else if (arg === "--artifacts") {
      out.artifactDir = resolve(next);
      i += 1;
    } else if (arg === "--scenario") {
      out.scenarioPath = resolve(next);
      i += 1;
    } else if (arg === "--upstream-base-url") {
      out.upstreamBaseUrl = next;
      i += 1;
    } else if (arg === "--rewrite-from") {
      out.rewriteFrom = next;
      i += 1;
    } else if (arg === "--rewrite-to") {
      out.rewriteTo = next;
      i += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo-relay

Usage:
  node index.mjs --mode proxy --artifacts <dir>
  node index.mjs --mode replay --artifacts <dir> --scenario <file>

Options:
  --mode <proxy|replay>
  --host <host>
  --port <port>
  --artifacts <dir>
  --scenario <file>
  --upstream-base-url <url>
  --rewrite-from <text>
  --rewrite-to <text>
`);
}

function ensureDir(path) {
  if (!existsSync(path)) {
    mkdirSync(path, { recursive: true });
  }
}

function appendJsonl(path, payload) {
  appendFileSync(path, `${JSON.stringify({ ts: new Date().toISOString(), ...payload })}\n`, "utf8");
}

function readBody(req) {
  return new Promise((resolveBody, rejectBody) => {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(Buffer.from(chunk)));
    req.on("end", () => resolveBody(Buffer.concat(chunks)));
    req.on("error", rejectBody);
  });
}

function buildUpstreamUrl(baseUrl, pathname, search) {
  const base = baseUrl.replace(/\/+$/, "");
  const normalizedPath = pathname.startsWith("/v1/") ? pathname.slice("/v1".length) : pathname;
  return `${base}${normalizedPath}${search || ""}`;
}

function redactHeaders(headers) {
  const out = {};
  for (const [key, value] of Object.entries(headers || {})) {
    out[key] = key.toLowerCase() === "authorization" ? "<redacted>" : value;
  }
  return out;
}

function rewriteText(text, rewriteRules) {
  let changed = false;
  let output = text;
  for (const rule of rewriteRules) {
    if (!rule.from) {
      continue;
    }
    if (output.includes(rule.from)) {
      output = output.split(rule.from).join(rule.to || "");
      changed = true;
    }
  }
  return { text: output, changed };
}

function extractWorkspacePathFromHeaders(headers) {
  const raw = headers["x-codex-turn-metadata"];
  if (typeof raw !== "string" || !raw.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw);
    const workspaces = parsed && typeof parsed === "object" ? parsed.workspaces : null;
    if (!workspaces || typeof workspaces !== "object") {
      return null;
    }
    const first = Object.keys(workspaces)[0];
    return first || null;
  } catch {
    return null;
  }
}

function extractWorkspacePathFromBody(bodyText) {
  if (!bodyText.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(bodyText);
    const input = Array.isArray(parsed.input) ? parsed.input : [];
    for (const item of input) {
      const content = Array.isArray(item?.content) ? item.content : [];
      for (const part of content) {
        const text = typeof part?.text === "string" ? part.text : "";
        const match = text.match(/<cwd>([^<]+)<\/cwd>/u);
        if (match?.[1]) {
          return match[1];
        }
      }
    }
  } catch {
    return null;
  }
  return null;
}

function buildDynamicRewriteRules(step, requestHeaders, bodyText) {
  const capturedWorkspacePath = step.requestContext?.capturedWorkspacePath || null;
  const liveWorkspacePath = extractWorkspacePathFromHeaders(requestHeaders) || extractWorkspacePathFromBody(bodyText);
  if (!capturedWorkspacePath || !liveWorkspacePath || capturedWorkspacePath === liveWorkspacePath) {
    return [];
  }
  return [{ from: capturedWorkspacePath, to: liveWorkspacePath }];
}

function loadScenario(scenarioPath) {
  const raw = JSON.parse(readFileSync(scenarioPath, "utf8"));
  const baseDir = dirname(scenarioPath);
  const loadStep = (stepRaw) => {
    const sseEventsPath = stepRaw.responses?.sse_events_path
      ? resolve(baseDir, stepRaw.responses.sse_events_path)
      : null;
    let sseEvents = [];
    if (sseEventsPath) {
      sseEvents = JSON.parse(readFileSync(sseEventsPath, "utf8"));
    } else if (Array.isArray(stepRaw.responses?.sse_events)) {
      sseEvents = stepRaw.responses.sse_events;
    }
    return {
      requestMatch: stepRaw.request_match || {},
      requestContext: {
        capturedWorkspacePath: stepRaw.request_context?.captured_workspace_path || null,
      },
      rewriteRules: Array.isArray(stepRaw.rewrite_rules) ? stepRaw.rewrite_rules : [],
      sseEvents,
    };
  };
  const steps = Array.isArray(raw.steps)
    ? raw.steps.map(loadStep)
    : [loadStep(raw)];
  return {
    id: raw.id,
    provider: raw.provider,
    rewriteRules: Array.isArray(raw.rewrite_rules) ? raw.rewrite_rules : [],
    responseDelayMs: Number(raw.response_delay_ms || 0),
    steps,
  };
}

function requestMatchesScenario(requestMatch, pathname, bodyText) {
  if (requestMatch.path && requestMatch.path !== pathname) {
    return false;
  }
  if (requestMatch.body_includes) {
    const required = Array.isArray(requestMatch.body_includes)
      ? requestMatch.body_includes
      : [requestMatch.body_includes];
    for (const needle of required) {
      if (!bodyText.includes(needle)) {
        return false;
      }
    }
  }
  return true;
}

async function proxyRequest({ options, bodyBuffer, pathname, search, requestHeaders, requestsLog, responsesLog, res }) {
  if (!options.upstreamApiKey) {
    res.writeHead(500, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: "OPENROUTER_API_KEY missing" }));
    return;
  }

  const upstreamUrl = buildUpstreamUrl(options.upstreamBaseUrl, pathname, search);
  const upstreamHeaders = {
    "content-type": requestHeaders["content-type"] || "application/json",
    authorization: `Bearer ${options.upstreamApiKey}`,
    accept: requestHeaders.accept || "*/*",
    "user-agent": requestHeaders["user-agent"] || "ctx-demo-relay",
    "http-referer": "https://ctx.local/demo-relay",
    "x-title": "ctx demo relay",
  };

  const upstreamResp = await fetch(upstreamUrl, {
    method: "POST",
    headers: upstreamHeaders,
    body: bodyBuffer.length > 0 ? bodyBuffer : undefined,
  });

  res.writeHead(upstreamResp.status, Object.fromEntries(upstreamResp.headers.entries()));

  if (!upstreamResp.body) {
    appendJsonl(responsesLog, { kind: "sse-missing-body", path: pathname, status: upstreamResp.status });
    res.end();
    return;
  }

  const decoder = new TextDecoder();
  for await (const chunk of upstreamResp.body) {
    const text = decoder.decode(chunk, { stream: true });
    appendJsonl(responsesLog, {
      kind: "sse-chunk",
      path: pathname,
      status: upstreamResp.status,
      chunk: text,
    });
    res.write(chunk);
  }
  res.end();
}

async function replayScenario({ scenario, state, pathname, bodyText, requestHeaders, requestsLog, responsesLog, res }) {
  const currentStepIndex = state.nextStepIndex ?? 0;
  const step = scenario.steps[currentStepIndex];
  if (!step) {
    res.writeHead(409, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: `scenario ${scenario.id} exhausted` }));
    return;
  }
  if (!requestMatchesScenario(step.requestMatch, pathname, bodyText)) {
    res.writeHead(404, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: `no scenario match for ${pathname} at step ${currentStepIndex + 1}` }));
    return;
  }

  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const rewriteRules = [
    ...scenario.rewriteRules,
    ...step.rewriteRules,
    ...buildDynamicRewriteRules(step, requestHeaders, bodyText),
  ];
  for (const event of step.sseEvents) {
    const baseChunk = typeof event === "string" ? event : event.chunk;
    const delayMs = typeof event === "string" ? scenario.responseDelayMs : Number(event.delay_ms ?? scenario.responseDelayMs);
    const rewritten = rewriteText(baseChunk, rewriteRules);
    appendJsonl(responsesLog, {
      kind: "sse-chunk",
      path: pathname,
      status: 200,
      changed: rewritten.changed,
      chunk: rewritten.text,
    });
    if (delayMs > 0) {
      await new Promise((resolveDelay) => setTimeout(resolveDelay, delayMs));
    }
    res.write(rewritten.text);
  }
  state.nextStepIndex = currentStepIndex + 1;
  res.end();
}

export async function startDemoRelay(cliOptions) {
  const options = cliOptions;
  if (!options.artifactDir) {
    throw new Error("--artifacts is required");
  }
  ensureDir(options.artifactDir);
  const requestsLog = join(options.artifactDir, "requests.jsonl");
  const responsesLog = join(options.artifactDir, "responses.jsonl");
  const serverLog = join(options.artifactDir, "server.log");
  const scenario = options.scenarioPath ? loadScenario(options.scenarioPath) : null;
  const replayState = { nextStepIndex: 0 };
  const directRewriteRules = options.rewriteFrom
    ? [{ from: options.rewriteFrom, to: options.rewriteTo }]
    : [];
  if (scenario && directRewriteRules.length > 0) {
    scenario.rewriteRules = [...scenario.rewriteRules, ...directRewriteRules];
  }

  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url || "/", `http://${req.headers.host || "127.0.0.1"}`);
      const bodyBuffer = await readBody(req);
      const bodyText = bodyBuffer.toString("utf8");
      appendJsonl(requestsLog, {
        method: req.method,
        path: url.pathname,
        headers: redactHeaders(req.headers),
        body: bodyText,
      });

      if (url.pathname === "/v1/models") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ object: "list", data: [{ id: "openai/gpt-5.4", object: "model" }] }));
        return;
      }

      if (url.pathname !== "/v1/responses") {
        res.writeHead(404, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: `unsupported path ${url.pathname}` }));
        return;
      }

      if (options.mode === "replay") {
        await replayScenario({
          scenario,
          state: replayState,
          pathname: url.pathname,
          bodyText,
          requestHeaders: req.headers,
          requestsLog,
          responsesLog,
          res,
        });
        return;
      }

      await proxyRequest({
        options,
        bodyBuffer,
        pathname: url.pathname,
        search: url.search,
        requestHeaders: req.headers,
        requestsLog,
        responsesLog,
        res,
      });
    } catch (error) {
      appendFileSync(serverLog, `${new Date().toISOString()} ${String(error?.stack || error)}\n`, "utf8");
      res.writeHead(500, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "demo relay failure", detail: String(error) }));
    }
  });

  await new Promise((resolveListen) => {
    server.listen(options.listenPort, options.listenHost, resolveListen);
  });

  const address = server.address();
  const resolvedPort = typeof address === "object" && address ? address.port : options.listenPort;
  writeFileSync(join(options.artifactDir, "relay-port.txt"), String(resolvedPort), "utf8");
  appendFileSync(
    serverLog,
    `${new Date().toISOString()} listening http://${options.listenHost}:${resolvedPort} mode=${options.mode} scenario=${options.scenarioPath || ""}\n`,
    "utf8",
  );
  return { server, port: resolvedPort };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const options = parseArgs(process.argv.slice(2));
  startDemoRelay(options).catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
