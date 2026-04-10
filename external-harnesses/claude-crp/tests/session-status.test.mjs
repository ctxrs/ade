import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import * as os from "node:os";
import * as path from "node:path";

import { buildSessionStatusNotice } from "../dist/runtime.js";

const rootDir = path.resolve(import.meta.dirname, "..");
const runtimeBin = path.join(rootDir, "bin", "claude-crp");

function runIdleSessionStatusProbe() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [runtimeBin], {
      cwd: rootDir,
      env: {
        ...process.env,
        CLAUDE_CODE_OAUTH_TOKEN: "test-oauth-token",
        CLAUDE_CONFIG_DIR: os.tmpdir()
      },
      stdio: ["pipe", "pipe", "pipe"]
    });

    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`claude-crp exited ${code}: ${stderr.trim() || "no stderr"}`));
        return;
      }
      resolve({ stdout, stderr });
    });

    child.stdin.write(
      `${JSON.stringify({
        v: 1,
        command: {
          type: "session.open",
          session_id: "session-status",
          config: { model: "default", cwd: rootDir }
        }
      })}\n`
    );
    child.stdin.write(
      `${JSON.stringify({
        v: 1,
        command: {
          type: "session.status",
          session_id: "session-status"
        }
      })}\n`
    );
    child.stdin.end();
  });
}

test("session.status reports quiescent for an idle session", async () => {
  const { stdout } = await runIdleSessionStatusProbe();
  const payloads = stdout
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line));

  const statusNotice = payloads.find(
    (payload) => payload.type === "session.notice" && payload.code === "session_status"
  );
  assert.ok(statusNotice, "expected session_status notice");
  assert.equal(statusNotice.details?.quiescent, true);
  assert.equal(statusNotice.details?.active_turn_id, null);
  assert.deepEqual(statusNotice.details?.busy_reasons, []);
});

test("buildSessionStatusNotice reports busy when an active turn exists", () => {
  const notice = buildSessionStatusNotice({
    sessionId: "session-status",
    activeTurnId: "turn-123"
  });

  assert.equal(notice.code, "session_status");
  assert.equal(notice.details?.quiescent, false);
  assert.equal(notice.details?.active_turn_id, "turn-123");
  assert.deepEqual(notice.details?.busy_reasons, ["active_turn"]);
});
