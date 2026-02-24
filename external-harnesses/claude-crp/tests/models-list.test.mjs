import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import * as os from "node:os";
import * as path from "node:path";

const rootDir = path.resolve(import.meta.dirname, "..");
const runtimeBin = path.join(rootDir, "bin", "claude-crp");

function runModelsListProbe() {
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

    const line = JSON.stringify({
      v: 1,
      command: {
        type: "models.list",
        config: {
          model: "default",
          cwd: rootDir
        }
      }
    });
    child.stdin.write(`${line}\n`);
    child.stdin.end();
  });
}

test("models.list emits default OAuth model catalog", async () => {
  const { stdout } = await runModelsListProbe();
  const lines = stdout
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  assert.ok(lines.length > 0, "expected at least one JSONL output line");

  const payload = JSON.parse(lines[0]);
  assert.equal(payload.type, "models.list");

  const modelIds = Array.isArray(payload.models)
    ? payload.models.map((entry) => String(entry?.id ?? ""))
    : [];
  assert.ok(modelIds.includes("default"), "expected default model");
  assert.ok(modelIds.includes("sonnet"), "expected sonnet model");
  assert.ok(modelIds.includes("opus"), "expected opus model");
  assert.equal(payload.current_model_id, "default");
});
