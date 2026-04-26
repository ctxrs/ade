const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  isAccountBackedImport,
  stageProviderAuthImportFixture,
} = require("./provider_auth_import_runtime.cjs");

const withEnv = (updates, run) => {
  const previous = new Map();
  for (const [key, value] of Object.entries(updates)) {
    previous.set(key, process.env[key]);
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
  try {
    return run();
  } finally {
    for (const [key, value] of previous.entries()) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
};

test("codex-crp auth import stages the canonical codex auth fixture", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-codex-crp-auth-import-"));
  const authJson = JSON.stringify({ OPENAI_API_KEY: "test-key" });
  const staged = withEnv(
    {
      CTX_E2E_AUTH_IMPORT_CODEX_AUTH_JSON_B64: Buffer.from(authJson, "utf8").toString("base64"),
      CTX_E2E_AUTH_IMPORT_CODEX_AUTH_JSON: undefined,
      CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME: tempDir,
    },
    () => stageProviderAuthImportFixture("codex-crp"),
  );

  assert.equal(staged.providerId, "codex");
  assert.equal(staged.kind, "auth_file");
  assert.equal(staged.expectedPath, path.join(tempDir, "auth.json"));
  assert.equal(fs.readFileSync(staged.expectedPath, "utf8"), authJson);
});

test("codex-crp auth import is account backed like codex", () => {
  assert.equal(isAccountBackedImport("codex", "auth_file"), true);
  assert.equal(isAccountBackedImport("codex-crp", "auth_file"), true);
});
