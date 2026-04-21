import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import * as os from "node:os";
import * as path from "node:path";

const rootDir = path.resolve(import.meta.dirname, "..");
const runtimeBin = path.join(rootDir, "bin", "claude-crp");

function runSessionSetModelProbe() {
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
          session_id: "session-1",
          config: { model: "default", cwd: rootDir }
        }
      })}\n`
    );
    child.stdin.write(
      `${JSON.stringify({
        v: 1,
        command: {
          type: "session.set_model",
          session_id: "session-1",
          model_id: "sonnet"
        }
      })}\n`
    );
    child.stdin.end();
  });
}

test("session.set_model updates the active session default model", async () => {
  const { stdout } = await runSessionSetModelProbe();
  const lines = stdout
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  assert.ok(lines.length >= 2, "expected session.opened and session.notice");
  const payloads = lines.map((line) => JSON.parse(line));

  const notice = payloads.find((payload) => payload.type === "session.notice");
  assert.ok(notice, "expected session.notice payload");
  assert.equal(notice.code, "session_model_updated");
  assert.equal(notice.details?.model_id, "sonnet");
});
