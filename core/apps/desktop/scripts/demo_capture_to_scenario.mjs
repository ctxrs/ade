#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";

function parseArgs(argv) {
  const out = {
    requestsLogPath: null,
    responsesLogPath: null,
    scenarioPath: null,
    scenarioId: "demo-scenario",
    provider: "codex",
    matchPath: "/v1/responses",
    matchText: "",
    responseDelayMs: 0,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--responses-log") {
      out.responsesLogPath = resolve(next);
      i += 1;
    } else if (arg === "--requests-log") {
      out.requestsLogPath = resolve(next);
      i += 1;
    } else if (arg === "--scenario") {
      out.scenarioPath = resolve(next);
      i += 1;
    } else if (arg === "--scenario-id") {
      out.scenarioId = next;
      i += 1;
    } else if (arg === "--provider") {
      out.provider = next;
      i += 1;
    } else if (arg === "--match-text") {
      out.matchText = next;
      i += 1;
    } else if (arg === "--match-path") {
      out.matchPath = next;
      i += 1;
    } else if (arg === "--response-delay-ms") {
      out.responseDelayMs = Number(next);
      i += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  if (!out.responsesLogPath) {
    throw new Error("--responses-log is required");
  }
  if (!out.scenarioPath) {
    throw new Error("--scenario is required");
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo_capture_to_scenario

Usage:
  node core/apps/desktop/scripts/demo_capture_to_scenario.mjs \\
    --requests-log path/to/requests.jsonl \\
    --responses-log path/to/responses.jsonl \\
    --scenario /tmp/codex-ping-pong.replay.json \\
    --scenario-id codex-ping-pong \\
    --match-text "Make a ping pong game."
`);
}

function parseJsonl(path) {
  return readFileSync(path, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function normalizePath(pathname) {
  if (pathname === "/api/v1/responses") {
    return "/v1/responses";
  }
  return pathname;
}

function computeDelayMs(prevTs, nextTs, fallbackMs) {
  if (!prevTs || !nextTs) {
    return fallbackMs;
  }
  const delta = Math.max(0, Math.round(new Date(nextTs).getTime() - new Date(prevTs).getTime()));
  return Math.min(delta, 4_000);
}

function parseSseEvent(raw) {
  const normalized = raw.endsWith("\n\n") ? raw : `${raw}\n\n`;
  const lines = normalized.trimEnd().split("\n");
  const dataLines = lines.filter((line) => line.startsWith("data: "));
  const commentLines = lines.filter((line) => line.startsWith(":"));
  let type = null;
  if (dataLines.length > 0) {
    const payload = dataLines.map((line) => line.slice(6)).join("\n");
    if (payload === "[DONE]") {
      type = "[DONE]";
    } else {
      try {
        type = JSON.parse(payload).type || null;
      } catch {
        type = null;
      }
    }
  } else if (commentLines.length > 0) {
    type = "comment";
  }
  return { raw: normalized, type };
}

function stripSensitiveFields(value) {
  if (Array.isArray(value)) {
    return value.map(stripSensitiveFields);
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value)
      .filter(([key]) => key !== "encrypted_content")
      .map(([key, nestedValue]) => [key, stripSensitiveFields(nestedValue)]),
  );
}

function sanitizeResponseCreated(response) {
  if (!response || typeof response !== "object") {
    return response;
  }
  const sanitized = {};
  for (const key of [
    "id",
    "object",
    "created_at",
    "model",
    "status",
    "completed_at",
    "output",
    "error",
    "incomplete_details",
  ]) {
    if (key in response) {
      sanitized[key] = stripSensitiveFields(response[key]);
    }
  }
  return sanitized;
}

function sanitizeSseChunk(raw) {
  const normalized = raw.endsWith("\n\n") ? raw : `${raw}\n\n`;
  const trimmed = normalized.trimEnd();
  if (!trimmed) {
    return null;
  }
  if (trimmed.split("\n").every((line) => line.startsWith(":"))) {
    return null;
  }
  const dataLines = trimmed
    .split("\n")
    .filter((line) => line.startsWith("data: "))
    .map((line) => line.slice(6));
  if (dataLines.length === 0) {
    return normalized;
  }
  const payload = dataLines.join("\n");
  if (payload === "[DONE]") {
    return "data: [DONE]\n\n";
  }
  let parsed;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return normalized;
  }
  const sanitized = stripSensitiveFields(parsed);
  if (sanitized?.response) {
    sanitized.response = sanitizeResponseCreated(sanitized.response);
  }
  return `data: ${JSON.stringify(sanitized)}\n\n`;
}

export function sanitizeSseEvents(entries) {
  const sanitized = [];
  let carryDelayMs = 0;
  for (const entry of entries) {
    const chunk = sanitizeSseChunk(String(entry.chunk || ""));
    if (!chunk) {
      carryDelayMs += Number(entry.delay_ms || 0);
      continue;
    }
    sanitized.push({
      delay_ms: Number(entry.delay_ms || 0) + carryDelayMs,
      chunk,
    });
    carryDelayMs = 0;
  }
  return sanitized;
}

function collectResponseStreams(entries) {
  const streams = [];
  let current = [];
  let currentStartTs = null;
  let buffer = "";
  for (const entry of entries) {
    const chunk = typeof entry.chunk === "string" ? entry.chunk : "";
    if (!chunk) {
      continue;
    }
    if (normalizePath(String(entry.path || "")) !== "/v1/responses") {
      continue;
    }
    buffer += chunk;
    while (buffer.includes("\n\n")) {
      const boundary = buffer.indexOf("\n\n");
      const rawEvent = buffer.slice(0, boundary + 2);
      buffer = buffer.slice(boundary + 2);
      const parsed = parseSseEvent(rawEvent);
      if (!parsed.type) {
        continue;
      }
      if (current.length === 0) {
        if (parsed.type !== "response.created" && parsed.type !== "comment") {
          continue;
        }
        currentStartTs = entry.ts;
      }
      current.push({
        ts: entry.ts,
        chunk: parsed.raw,
        type: parsed.type,
      });
      if (parsed.type === "[DONE]") {
        streams.push({
          startTs: currentStartTs || entry.ts,
          events: current,
        });
        current = [];
        currentStartTs = null;
      }
    }
  }
  return streams;
}

function buildSseEvents(entries, fallbackDelayMs) {
  let previousTs = null;
  return entries.map((entry) => {
    const delayMs = computeDelayMs(previousTs, entry.ts, fallbackDelayMs);
    previousTs = entry.ts;
    return {
      delay_ms: delayMs,
      chunk: entry.chunk,
    };
  });
}

function streamHasFunctionCall(events) {
  return events.some((entry) => String(entry.chunk || "").includes("\"function_call\""));
}

function extractWorkspacePathFromRequest(requestEntry) {
  const headers = requestEntry && typeof requestEntry.headers === "object" ? requestEntry.headers : {};
  const turnMetadata = headers["x-codex-turn-metadata"];
  if (typeof turnMetadata === "string" && turnMetadata.trim()) {
    try {
      const parsed = JSON.parse(turnMetadata);
      const workspaces = parsed && typeof parsed === "object" ? parsed.workspaces : null;
      if (workspaces && typeof workspaces === "object") {
        const first = Object.keys(workspaces)[0];
        if (first) {
          return first;
        }
      }
    } catch {
      // Ignore malformed captured metadata and fall back to body parsing.
    }
  }

  if (typeof requestEntry?.body !== "string" || !requestEntry.body.trim()) {
    return null;
  }

  try {
    const parsed = JSON.parse(requestEntry.body);
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
    // Ignore malformed captured request bodies.
  }

  return null;
}

export function buildScenarioFromCapture({
  requestsLogPath,
  responsesLogPath,
  scenarioPath,
  scenarioId,
  provider,
  matchPath,
  matchText,
  responseDelayMs,
}) {
  const requestEntries = (requestsLogPath ? parseJsonl(requestsLogPath) : [])
    .filter((entry) => normalizePath(String(entry.path || "")) === "/v1/responses");
  const responseEntries = parseJsonl(responsesLogPath);
  const responseStreams = collectResponseStreams(responseEntries);

  const pairedSteps = [];
  let requestIndex = 0;
  for (const stream of responseStreams) {
    const groupStartTs = String(stream.startTs || "");
    while (
      requestIndex + 1 < requestEntries.length
      && String(requestEntries[requestIndex + 1]?.ts || "") <= groupStartTs
    ) {
      requestIndex += 1;
    }
    const requestEntry = requestEntries[requestIndex];
    if (!requestEntry) {
      continue;
    }
    const firstType = String(stream.events[0]?.type || "");
    if (firstType !== "response.created" && firstType !== "comment") {
      continue;
    }
    pairedSteps.push({
      request: requestEntry,
      events: buildSseEvents(stream.events, responseDelayMs),
    });
    requestIndex += 1;
  }

  const selectedRequestIndex = matchText
    ? pairedSteps.findIndex((step) => String(step.request.body || "").toLowerCase().includes(matchText.toLowerCase()))
    : 0;
  if (selectedRequestIndex < 0) {
    throw new Error(`No matching Responses request found in ${requestsLogPath || "<none>"}`);
  }

  const selectedSteps = [];
  for (let i = selectedRequestIndex; i < pairedSteps.length; i += 1) {
    const step = pairedSteps[i];
    selectedSteps.push(step);
    if (!streamHasFunctionCall(step.events)) {
      break;
    }
  }

  if (selectedSteps.length === 0) {
    throw new Error(`No complete Responses SSE stream found in ${responsesLogPath}`);
  }

  mkdirSync(dirname(scenarioPath), { recursive: true });
  const scenarioDir = dirname(scenarioPath);
  const steps = selectedSteps.map((step, index) => {
    const ssePath = scenarioPath.replace(/\.json$/u, `.step-${index + 1}.sse.json`);
    writeFileSync(ssePath, JSON.stringify(sanitizeSseEvents(step.events), null, 2), "utf8");
    return {
      request_match: {
        path: matchPath,
        body_includes: index === 0 && matchText ? [matchText] : [],
      },
      request_context: {
        captured_workspace_path: extractWorkspacePathFromRequest(step.request),
      },
      responses: {
        sse_events_path: relative(scenarioDir, ssePath),
      },
    };
  });

  const scenario = {
    id: scenarioId,
    provider,
    response_delay_ms: responseDelayMs,
    rewrite_rules: [],
    steps,
  };
  writeFileSync(scenarioPath, JSON.stringify(scenario, null, 2), "utf8");
  return {
    scenarioPath,
    ssePath: steps[0]?.responses?.sse_events_path ? resolve(scenarioDir, steps[0].responses.sse_events_path) : null,
    eventCount: selectedSteps.reduce((sum, step) => sum + step.events.length, 0),
    stepCount: selectedSteps.length,
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = buildScenarioFromCapture({
    responsesLogPath: options.responsesLogPath,
    requestsLogPath: options.requestsLogPath,
    scenarioPath: options.scenarioPath,
    scenarioId: options.scenarioId,
    provider: options.provider,
    matchPath: options.matchPath,
    matchText: options.matchText,
    responseDelayMs: options.responseDelayMs,
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
