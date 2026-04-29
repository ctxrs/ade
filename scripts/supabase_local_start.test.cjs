const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..");

function makeTempBin(t, dockerOutput = "") {
  const dir = fs.mkdtempSync(path.join("/tmp", "ctx-supabase-local-test-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));

  const callsPath = path.join(dir, "calls.log");
  const supabasePath = path.join(dir, "supabase");
  const dockerPath = path.join(dir, "docker");

  fs.writeFileSync(
    supabasePath,
    `#!/bin/bash
set -euo pipefail
printf '%s|supabase %s\\n' "$PWD" "$*" >> "$SUPABASE_CALLS"
if [ "$1" = "status" ]; then
  exit 1
fi
exit 0
`,
  );
  fs.chmodSync(supabasePath, 0o755);

  fs.writeFileSync(
    dockerPath,
    `#!/bin/bash
set -euo pipefail
printf '%s|docker %s\\n' "$PWD" "$*" >> "$SUPABASE_CALLS"
cat <<'EOF'
${dockerOutput}
EOF
`,
  );
  fs.chmodSync(dockerPath, 0o755);

  return { dir, callsPath };
}

function runScript(script, tempBin, extraEnv = {}) {
  return spawnSync("bash", [path.join(repoRoot, script)], {
    cwd: path.join(repoRoot, "core"),
    env: {
      ...process.env,
      PATH: `${tempBin.dir}${path.delimiter}${process.env.PATH || ""}`,
      SUPABASE_CALLS: tempBin.callsPath,
      ...extraEnv,
    },
    encoding: "utf8",
  });
}

function readCalls(callsPath) {
  return fs.existsSync(callsPath)
    ? fs.readFileSync(callsPath, "utf8").trim().split("\n").filter(Boolean)
    : [];
}

test("local Supabase scripts do not use the wrong --workdir supabase project", () => {
  for (const script of [
    "scripts/supabase_local_start.sh",
    "scripts/supabase_local_ensure.sh",
  ]) {
    const text = fs.readFileSync(path.join(repoRoot, script), "utf8");
    assert.doesNotMatch(text, /--workdir\s+supabase/);
  }
});

test("supabase_local_start runs start and reset from the repository root", (t) => {
  const tempBin = makeTempBin(t);
  const result = runScript("scripts/supabase_local_start.sh", tempBin);

  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readCalls(tempBin.callsPath), [
    `${repoRoot}|docker ps --format {{.Names}}`,
    `${repoRoot}|supabase start`,
    `${repoRoot}|supabase db reset --no-seed`,
  ]);
});

test("supabase_local_ensure starts from the repository root when status is absent", (t) => {
  const tempBin = makeTempBin(t);
  const result = runScript("scripts/supabase_local_ensure.sh", tempBin);

  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readCalls(tempBin.callsPath), [
    `${repoRoot}|docker ps --format {{.Names}}`,
    `${repoRoot}|supabase status --output json`,
    `${repoRoot}|supabase start`,
  ]);
});

test("local scripts reject the accidental --workdir supabase stack", (t) => {
  const tempBin = makeTempBin(t, "supabase_db_supabase");
  const result = runScript("scripts/supabase_local_start.sh", tempBin);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /project 'supabase'/);
  assert.deepEqual(readCalls(tempBin.callsPath), [
    `${repoRoot}|docker ps --format {{.Names}}`,
  ]);
});
