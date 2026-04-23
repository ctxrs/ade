const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const buildFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "BUILD.bazel"), "utf8");
const verifyAgentRemote = fs.readFileSync(path.join(repoRoot, "core", "scripts", "verify_agent_remote.cjs"), "utf8");

function targetBlock(name) {
  const lines = buildFile.split(/\r?\n/u);
  const nameLineIndex = lines.findIndex((line) => line.trim() === `name = "${name}",`);
  assert.notEqual(nameLineIndex, -1, `expected target ${name} in core/apps/web/BUILD.bazel`);

  let startIndex = nameLineIndex;
  while (startIndex > 0 && !/^[A-Za-z0-9_.]+\($/u.test(lines[startIndex].trim())) {
    startIndex -= 1;
  }
  assert.match(lines[startIndex].trim(), /^[A-Za-z0-9_.]+\($/u, `expected start of target ${name}`);

  let endIndex = nameLineIndex;
  while (endIndex < lines.length && lines[endIndex].trim() !== ")") {
    endIndex += 1;
  }
  assert.notEqual(endIndex, lines.length, `expected end of target ${name}`);

  return lines.slice(startIndex, endIndex + 1).join("\n");
}

test("supported web validation targets do not use the non-hermetic workspace wrapper", () => {
  for (const targetName of ["unit_smoke", "unit_tests", "lint", "typecheck", "build", "desktop_ipc_corpus"]) {
    const block = targetBlock(targetName);
    assert.doesNotMatch(block, /run_workspace_task\.sh/u, `${targetName} must not use workspace node_modules`);
    assert.doesNotMatch(block, /args = \[[\s\S]*?"pnpm"/u, `${targetName} must not shell out to pnpm`);
    assert.match(block, /HERMETIC_WEB_(?:SMOKE|CHECK)_DATA/u, `${targetName} should use Bazel-managed web inputs`);
  }
});

test("browser e2e targets remain explicit non-agent wrapper targets", () => {
  for (const targetName of ["e2e_premerge", "e2e_release", "e2e_cross_platform", "e2e_visual", "e2e_soak", "e2e_load"]) {
    const block = targetBlock(targetName);
    assert.match(block, /run_workspace_task\.sh/u, `${targetName} should be visibly outside hermetic web validation`);
  }
});

test("verify:agent-remote web profiles run Bazel web targets without dependency hydration", () => {
  assert.match(verifyAgentRemote, /"web-smoke"[\s\S]*?\/\/core\/apps\/web:unit_smoke/u);
  assert.match(verifyAgentRemote, /"web-unit"[\s\S]*?\/\/core\/apps\/web:unit_tests/u);
  assert.match(verifyAgentRemote, /function requiresWebHydration\(step\) \{\n  return false;\n\}/u);
  assert.doesNotMatch(verifyAgentRemote, /pnpm[^\n]+install[^\n]+frozen-lockfile/u);
});
