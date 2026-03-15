import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

import { buildScenarioFromCapture, sanitizeSseEvents } from "./demo_capture_to_scenario.mjs";

test("buildScenarioFromCapture extracts a multi-step responses sequence", () => {
  const dir = mkdtempSync(join(tmpdir(), "ctx-demo-capture-"));
  const requestsLogPath = join(dir, "requests.jsonl");
  const responsesLogPath = join(dir, "responses.jsonl");
  const scenarioPath = join(dir, "scenario.json");
  writeFileSync(
    requestsLogPath,
    [
      JSON.stringify({ ts: "2026-03-13T00:00:00.500Z", path: "/v1/responses", body: "reply with pong" }),
      JSON.stringify({
        ts: "2026-03-13T00:00:01.500Z",
        path: "/v1/responses",
        headers: {
          "x-codex-turn-metadata": JSON.stringify({
            workspaces: {
              "/tmp/captured-demo-worktree": { has_changes: false },
            },
          }),
        },
        body: JSON.stringify({
          input: [
            {
              content: [
                { text: "<environment_context>\n  <cwd>/tmp/captured-demo-worktree</cwd>\n</environment_context>" },
              ],
            },
            {
              content: [{ text: "make a ping pong game" }],
            },
          ],
        }),
      }),
      JSON.stringify({ ts: "2026-03-13T00:00:04.500Z", path: "/v1/responses", body: "tool output for ping pong game" }),
    ].join("\n"),
    "utf8",
  );
  writeFileSync(
    responsesLogPath,
    [
      JSON.stringify({ ts: "2026-03-13T00:00:00.000Z", path: "/v1/responses", chunk: "<!DOCTYPE html>" }),
      JSON.stringify({ ts: "2026-03-13T00:00:01.000Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.created\"}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:02.250Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.output_item.done\"}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:03.000Z", path: "/api/v1/responses", chunk: "data: [DONE]\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:04.000Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.created\"}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:05.250Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"exec_command\"}}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:06.000Z", path: "/api/v1/responses", chunk: "data: [DONE]\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:07.000Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.created\"}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:08.250Z", path: "/api/v1/responses", chunk: "data: {\"type\":\"response.output_item.done\",\"item\":{\"content\":[{\"text\":\"ping pong\"}]}}\n\n" }),
      JSON.stringify({ ts: "2026-03-13T00:00:09.000Z", path: "/api/v1/responses", chunk: "data: [DONE]\n\n" }),
    ].join("\n"),
    "utf8",
  );

  const result = buildScenarioFromCapture({
    requestsLogPath,
    responsesLogPath,
    scenarioPath,
    scenarioId: "codex-ping-pong",
    provider: "codex",
    matchPath: "/v1/responses",
    matchText: "ping pong game",
    responseDelayMs: 150,
  });

  assert.equal(result.stepCount, 2);
  assert.equal(result.eventCount, 6);
  const scenario = JSON.parse(readFileSync(scenarioPath, "utf8"));
  assert.equal(scenario.id, "codex-ping-pong");
  assert.equal(scenario.steps.length, 2);
  assert.deepEqual(scenario.steps[0].request_match.body_includes, ["ping pong game"]);
  assert.deepEqual(scenario.steps[1].request_match.body_includes, []);
  assert.equal(scenario.steps[0].request_context.captured_workspace_path, "/tmp/captured-demo-worktree");

  const firstStepEvents = JSON.parse(readFileSync(join(dir, "scenario.step-1.sse.json"), "utf8"));
  assert.deepEqual(firstStepEvents.map((event) => event.chunk), [
    "data: {\"type\":\"response.created\"}\n\n",
    "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"exec_command\"}}\n\n",
    "data: [DONE]\n\n",
  ]);
  assert.equal(firstStepEvents[0].delay_ms, 150);
  assert.equal(firstStepEvents[1].delay_ms, 1250);

  const secondStepEvents = JSON.parse(readFileSync(join(dir, "scenario.step-2.sse.json"), "utf8"));
  assert.deepEqual(secondStepEvents.map((event) => event.chunk), [
    "data: {\"type\":\"response.created\"}\n\n",
    "data: {\"type\":\"response.output_item.done\",\"item\":{\"content\":[{\"text\":\"ping pong\"}]}}\n\n",
    "data: [DONE]\n\n",
  ]);
});

test("sanitizeSseEvents strips provider internals from captured replay chunks", () => {
  const events = sanitizeSseEvents([
    {
      delay_ms: 120,
      chunk: ": OPENROUTER PROCESSING\n\n",
    },
    {
      delay_ms: 80,
      chunk: "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-1\",\"object\":\"response\",\"created_at\":1,\"model\":\"openai/gpt-5.4\",\"status\":\"in_progress\",\"completed_at\":null,\"output\":[],\"error\":null,\"incomplete_details\":null,\"tools\":[{\"name\":\"exec_command\"}],\"instructions\":\"secret prompt\"}}\n\n",
    },
    {
      delay_ms: 40,
      chunk: "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"secret\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"ok\"}]}}\n\n",
    },
    {
      delay_ms: 20,
      chunk: "data: [DONE]\n\n",
    },
  ]);

  assert.equal(events.length, 3);
  assert.equal(events[0].delay_ms, 200);
  assert.equal(events[0].chunk, "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-1\",\"object\":\"response\",\"created_at\":1,\"model\":\"openai/gpt-5.4\",\"status\":\"in_progress\",\"completed_at\":null,\"output\":[],\"error\":null,\"incomplete_details\":null}}\n\n");
  assert.equal(events[1].chunk, "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"ok\"}]}}\n\n");
  assert.equal(events[2].chunk, "data: [DONE]\n\n");
});
