const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const toolsBuild = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "BUILD.bazel"), "utf8");
const helperScript = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "provider_claude_crp_archive.sh"), "utf8");
const claudeBuild = fs.readFileSync(path.join(repoRoot, "external-harnesses", "claude-crp", "BUILD.bazel"), "utf8");
const stageScript = fs.readFileSync(path.join(repoRoot, "scripts", "claude_crp_stage_archives.sh"), "utf8");

test("claude-crp provider archive helper is exported for Bazel-run staging", () => {
  assert.match(toolsBuild, /"provider_claude_crp_archive\.sh"/);
  assert.match(helperScript, /CLAUDE_CRP_TARGETS="\$TARGET_KEY"/);
  assert.match(helperScript, /scripts\/claude_crp_stage_archives\.sh/);
  assert.match(helperScript, /claude-crp-index\.json/);
});

test("claude-crp exposes a Bazel-run provider archive target", () => {
  assert.match(claudeBuild, /name = "provider-stage-archive"/);
  assert.match(claudeBuild, /"\/\/tools\/bazel:provider_claude_crp_archive\.sh"/);
});

test("claude-crp archive staging uses TMPDIR-aware temp roots", () => {
  assert.match(stageScript, /mktemp -d "\$\{TMPDIR:-\/tmp\}\/ctx-claude-crp-stage\.XXXXXX"/);
  assert.match(stageScript, /claude-crp staging requires one npm target tuple per invocation/);
  assert.match(stageScript, /npm_config_platform="\$npm_target_platform"/);
  assert.match(stageScript, /npm_config_arch="\$npm_target_arch"/);
});
