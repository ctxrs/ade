const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const buildScriptPath = path.join(__dirname, "..", "crates", "ctx-http", "build.rs");
const buildScript = fs.readFileSync(buildScriptPath, "utf8");

test("ctx-http build script only watches git metadata files that actually exist", () => {
  assert.match(buildScript, /let head_path = git_dir\.join\("HEAD"\);/);
  assert.match(
    buildScript,
    /if head_path\.exists\(\) \{\s*println!\("cargo:rerun-if-changed=\{\}", head_path\.display\(\)\);\s*\}/s,
  );
  assert.match(buildScript, /let packed_refs_path = git_dir\.join\("packed-refs"\);/);
  assert.match(
    buildScript,
    /if packed_refs_path\.exists\(\) \{\s*println!\("cargo:rerun-if-changed=\{\}", packed_refs_path\.display\(\)\);\s*\}/s,
  );
});
