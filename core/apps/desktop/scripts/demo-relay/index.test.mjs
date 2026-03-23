import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { performance } from "node:perf_hooks";

import { startDemoRelay } from "./index.mjs";

async function post(port, path, body) {
  const resp = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "text/event-stream" },
    body: JSON.stringify(body),
  });
  return resp.text();
}

test("replay mode serves scenario chunks and applies rewrite rules", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    scenarioPath,
    JSON.stringify({
      id: "hello",
      provider: "codex",
      request_match: { path: "/v1/responses", body_includes: "ALPHA" },
      rewrite_rules: [{ from: "ALPHA", to: "BETA" }],
      responses: {
        sse_events: [
          "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"ALPHA\"}]}}\n\n",
          "data: [DONE]\n\n",
        ],
      },
    }),
    "utf8",
  );

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const body = await post(relay.port, "/v1/responses", { prompt: "say ALPHA" });
    assert.match(body, /BETA/);
    assert.doesNotMatch(body, /ALPHA/);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("replay mode serves step sequences in order", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    scenarioPath,
    JSON.stringify({
      id: "seq",
      provider: "codex",
      steps: [
        {
          request_match: { path: "/v1/responses", body_includes: "ping pong" },
          responses: {
            sse_events: [
              "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"echo pong\\\"}\"}}\n\n",
              "data: [DONE]\n\n",
            ],
          },
        },
        {
          request_match: { path: "/v1/responses" },
          responses: {
            sse_events: [
              "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}}\n\n",
              "data: [DONE]\n\n",
            ],
          },
        },
      ],
    }),
    "utf8",
  );

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const first = await post(relay.port, "/v1/responses", { prompt: "make a ping pong game" });
    const second = await post(relay.port, "/v1/responses", { tool_results: [{ ok: true }] });
    assert.match(first, /function_call/);
    assert.match(second, /done/);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("replay mode rebinds captured workspace paths to the live request workspace", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    scenarioPath,
    JSON.stringify({
      id: "workspace-rebind",
      provider: "codex",
      steps: [
        {
          request_match: { path: "/v1/responses", body_includes: "ping pong" },
          request_context: { captured_workspace_path: "/tmp/captured-worktree" },
          responses: {
            sse_events: [
              "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"pwd\\\",\\\"workdir\\\":\\\"/tmp/captured-worktree\\\"}\"}}\n\n",
              "data: [DONE]\n\n",
            ],
          },
        },
      ],
    }),
    "utf8",
  );

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const resp = await fetch(`http://127.0.0.1:${relay.port}/v1/responses`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-codex-turn-metadata": JSON.stringify({
          workspaces: {
            "/tmp/live-worktree": { has_changes: false },
          },
        }),
      },
      body: JSON.stringify({ prompt: "make a ping pong game" }),
    });
    const body = await resp.text();
    assert.match(body, /\/tmp\/live-worktree/);
    assert.doesNotMatch(body, /\/tmp\/captured-worktree/);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("replay mode returns 404 when no scenario matches", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    scenarioPath,
    JSON.stringify({
      id: "hello",
      provider: "codex",
      request_match: { path: "/v1/responses", body_includes: "expected token" },
      responses: { sse_events: ["data: [DONE]\n\n"] },
    }),
    "utf8",
  );

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const resp = await fetch(`http://127.0.0.1:${relay.port}/v1/responses`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt: "different" }),
    });
    assert.equal(resp.status, 404);
    const text = await resp.text();
    assert.match(text, /no scenario match/);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("models endpoint exposes GPT-5.4 effort variants for demo selection", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath: null,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const resp = await fetch(`http://127.0.0.1:${relay.port}/v1/models`, {
      method: "POST",
      headers: { "content-type": "application/json" },
    });
    assert.equal(resp.status, 200);
    const body = await resp.json();
    assert.deepEqual(body.data.map((entry) => entry.id), [
      "gpt-5.4/low",
      "gpt-5.4/medium",
      "gpt-5.4/high",
      "gpt-5.4/xhigh",
    ]);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("replay mode can force a fixed delay instead of captured per-event delays", async () => {
  const dir = mkdtempSync(join(tmpdir(), "demo-relay-test-"));
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    scenarioPath,
    JSON.stringify({
      id: "fixed-delay",
      provider: "codex",
      response_delay_ms: 0,
      force_response_delay_ms: true,
      steps: [
        {
          request_match: { path: "/v1/responses", body_includes: "ping" },
          responses: {
            sse_events: [
              {
                delay_ms: 250,
                chunk: "data: {\"type\":\"response.created\"}\n\n",
              },
              {
                delay_ms: 250,
                chunk: "data: [DONE]\n\n",
              },
            ],
          },
        },
      ],
    }),
    "utf8",
  );

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: dir,
    scenarioPath,
    upstreamBaseUrl: "https://example.invalid",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  try {
    const startedAt = performance.now();
    const body = await post(relay.port, "/v1/responses", { prompt: "ping" });
    const elapsedMs = performance.now() - startedAt;
    assert.match(body, /response\.created/);
    assert.ok(elapsedMs < 200, `expected forced delay replay to finish quickly, saw ${elapsedMs}ms`);
  } finally {
    relay.server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});
