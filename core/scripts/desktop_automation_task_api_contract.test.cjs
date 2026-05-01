const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const automationSpecsDir = path.join(repoRoot, "core", "apps", "desktop", "automation", "specs");

function collectFiles(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...collectFiles(entryPath));
      continue;
    }
    if (/\.(?:cjs|mjs|js|ts|tsx)$/.test(entry.name)) {
      out.push(entryPath);
    }
  }
  return out;
}

test("desktop automation uses the current task default_session API", () => {
  const offenders = collectFiles(automationSpecsDir)
    .filter((filePath) => fs.readFileSync(filePath, "utf8").includes("create_default_session"))
    .map((filePath) => path.relative(repoRoot, filePath))
    .sort();

  assert.deepEqual(
    offenders,
    [],
    `legacy create_default_session is rejected by the daemon task API; update these automation files to use default_session: ${offenders.join(", ")}`,
  );
});
