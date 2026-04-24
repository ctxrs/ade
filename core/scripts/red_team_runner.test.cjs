const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const {
  defaultOutRoot,
  getLaneDefinitions,
  getPresetLaneIds,
  parseArgs,
  resolveLaneSkipReason,
  resolveSelectedLanes,
} = require("./red_team_runner.cjs");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));

function laneMap() {
  return new Map(getLaneDefinitions().map((lane) => [lane.id, lane]));
}

test("red-team presets are additive and keep opt-in lanes out of the default preset", () => {
  assert.deepEqual(getPresetLaneIds("core"), [
    "anomaly",
    "fuzz-regression",
    "mutation-critical",
    "provider-offline",
    "http-cross-platform",
    "updater-failure-safety",
    "release-manifest-race",
    "release-promote-smoke",
    "updater-contract-smoke",
  ]);

  assert.deepEqual(getPresetLaneIds("extended"), [
    "anomaly",
    "fuzz-regression",
    "mutation-critical",
    "provider-offline",
    "http-cross-platform",
    "updater-failure-safety",
    "release-manifest-race",
    "release-promote-smoke",
    "updater-contract-smoke",
    "web-pretext-fuzz",
    "load-smoke",
    "soak",
    "perf-daemon-cpu",
  ]);

  assert.deepEqual(getPresetLaneIds("full"), [
    "anomaly",
    "fuzz-regression",
    "mutation-critical",
    "provider-offline",
    "http-cross-platform",
    "updater-failure-safety",
    "release-manifest-race",
    "release-promote-smoke",
    "updater-contract-smoke",
    "web-pretext-fuzz",
    "load-smoke",
    "soak",
    "perf-daemon-cpu",
    "sandbox-egress-guard",
    "desktop-break-matrix",
    "provider-live-canary",
  ]);
});

test("explicit lane selection overrides preset and skip-lane removes entries", () => {
  const selected = resolveSelectedLanes({
    laneIds: ["load-smoke", "anomaly", "load-smoke"],
    preset: "full",
    skipLaneIds: ["anomaly"],
  });

  assert.deepEqual(
    selected.map((lane) => lane.id),
    ["load-smoke"],
  );
});

test("opt-in lanes explain missing prerequisites", () => {
  const lanes = laneMap();

  assert.equal(
    resolveLaneSkipReason(lanes.get("sandbox-egress-guard"), {
      availableCommands: new Set(),
      env: {},
      platform: "linux",
    }),
    "requires container runtime 'nerdctl'",
  );

  assert.equal(
    resolveLaneSkipReason(lanes.get("desktop-break-matrix"), {
      availableCommands: new Set(),
      env: {},
      platform: "linux",
    }),
    "requires macOS",
  );

  assert.equal(
    resolveLaneSkipReason(lanes.get("provider-live-canary"), {
      availableCommands: new Set(),
      env: {},
      platform: "darwin",
    }),
    "requires CTX_LIVE_PROVIDER_ID",
  );
});

test("default out root prefers CTX_VOLATILE_TMPDIR", () => {
  const outDir = defaultOutRoot({ CTX_VOLATILE_TMPDIR: "/tmp/ctx-volatile" }, new Date("2026-04-23T12:34:56.789Z"));
  assert.equal(outDir, "/tmp/ctx-volatile/ctx-redteam-2026-04-23T12-34-56-789Z");
});

test("parser tolerates pnpm's bare double-dash separator", () => {
  const args = parseArgs(["--preset", "core", "--", "--lane", "anomaly"]);
  assert.deepEqual(args.laneIds, ["anomaly"]);
});

test("package scripts expose stable red-team entrypoints", () => {
  assert.equal(packageJson.scripts["verify:redteam:list"], "node scripts/red_team_runner.cjs --list");
  assert.equal(packageJson.scripts["verify:redteam"], "node scripts/red_team_runner.cjs --preset core");
  assert.equal(packageJson.scripts["verify:redteam:extended"], "node scripts/red_team_runner.cjs --preset extended");
  assert.equal(packageJson.scripts["verify:redteam:full"], "node scripts/red_team_runner.cjs --preset full");
});
