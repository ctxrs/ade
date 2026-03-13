import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadTranslateModule() {
  const translatePath = pathToFileURL(path.join(rootDir, "src", "translate.ts"));
  const code = await fs.readFile(translatePath, "utf8");
  const dataUrl = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
  return import(dataUrl);
}

test("assistant tool_use messages preserve nested tool names for later tool results", async () => {
  const { translateClaudeEventsToCrp } = await loadTranslateModule();

  const records = [
    {
      record: "header",
      session_id: "session-1",
    },
    {
      record: "event",
      event: {
        type: "assistant",
        session_id: "session-1",
        message: {
          id: "msg-tool",
          model: "claude-haiku",
          content: [
            {
              type: "tool_use",
              id: "toolu_123",
              name: "Read",
              input: { file_path: "/tmp/specs" },
            },
          ],
        },
      },
    },
    {
      record: "event",
      event: {
        type: "user",
        session_id: "session-1",
        tool_use_result: "Error: EISDIR",
        message: {
          role: "user",
          content: [
            {
              type: "tool_result",
              tool_use_id: "toolu_123",
              content: "EISDIR",
              is_error: true,
            },
          ],
        },
      },
    },
  ];

  const events = translateClaudeEventsToCrp(records, {});
  const toolStarted = events.find((event) => event.type === "tool.started");
  const toolCompleted = events.find((event) => event.type === "tool.completed");

  assert.ok(toolStarted, "expected tool.started");
  assert.equal(toolStarted.tool_name, "Read");
  assert.deepEqual(toolStarted.input, { file_path: "/tmp/specs" });

  assert.ok(toolCompleted, "expected tool.completed");
  assert.equal(toolCompleted.tool_name, "Read");
  assert.equal(toolCompleted.status, "error");
});
