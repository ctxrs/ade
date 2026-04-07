import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadRuntimeModule() {
  return import(pathToFileURL(path.join(rootDir, "dist", "runtime.js")).href);
}

function projectKeyForCwd(cwd) {
  return cwd.replace(/[^a-zA-Z0-9]/g, "-");
}

function withClaudeConfigDir(configDir, fn) {
  const previous = process.env.CLAUDE_CONFIG_DIR;
  process.env.CLAUDE_CONFIG_DIR = configDir;
  return Promise.resolve()
    .then(fn)
    .finally(() => {
      if (previous === undefined) {
        delete process.env.CLAUDE_CONFIG_DIR;
      } else {
        process.env.CLAUDE_CONFIG_DIR = previous;
      }
    });
}

function buildTurn(overrides = {}) {
  return {
    sessionId: "ctx-session-1",
    providerSessionId: "provider-session-1",
    turnId: "turn-1",
    runId: "run-1",
    cwd: rootDir,
    permissionMode: "default",
    allowDangerouslySkipPermissions: false,
    records: [],
    emittedCount: 0,
    interrupted: false,
    endRecordAdded: false,
    abortController: new AbortController(),
    ...overrides,
  };
}

test("buildQueryOptions resumes the provider session when provider_session_id differs", async () => {
  const { buildQueryOptions } = await loadRuntimeModule();

  const tempConfigDir = await fs.mkdtemp(path.join(os.tmpdir(), "claude-crp-resume-"));
  const projectDir = path.join(
    tempConfigDir,
    "projects",
    projectKeyForCwd(rootDir),
  );
  await fs.mkdir(projectDir, { recursive: true });
  await fs.writeFile(path.join(projectDir, "provider-session-1.jsonl"), "");

  await withClaudeConfigDir(tempConfigDir, async () => {
    const options = buildQueryOptions(buildTurn());
    assert.equal(options.resume, "provider-session-1");
    assert.equal(options.extraArgs, undefined);
  });
});

test("buildQueryOptions creates new Claude sessions with provider_session_id", async () => {
  const { buildQueryOptions } = await loadRuntimeModule();

  const tempConfigDir = await fs.mkdtemp(path.join(os.tmpdir(), "claude-crp-session-id-"));

  await withClaudeConfigDir(tempConfigDir, async () => {
    const options = buildQueryOptions(
      buildTurn({
        sessionId: "ctx-session-2",
        providerSessionId: "provider-session-2",
      }),
    );
    assert.deepEqual(options.extraArgs, { "session-id": "provider-session-2" });
    assert.equal(options.resume, undefined);
  });
});
