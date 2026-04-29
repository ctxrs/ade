#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const {
  REQUIRED_PLATFORM_TARGETS,
  buildDefaultManifest,
  buildReport,
  flattenCells,
  validateManifest,
} = require("./validate_harness_install_matrix.cjs");

const providerMatrixFixture = () => ({
  providers: [
    {
      id: "codex",
      display_name: "Codex",
      command: { command: "codex-crp" },
      managed_install: { kind: "archive", version: "1.0.0", targets: {} },
    },
    {
      id: "gemini",
      display_name: "Gemini",
      command: { command: "gemini" },
      managed_install: { kind: "npm", package: "@google/gemini-cli", version: "0.39.0" },
    },
    {
      id: "codex-cli",
      kind: "dependency",
      display_name: "Codex CLI",
      command: { command: "codex-crp" },
      managed_install: { kind: "archive", version: "1.0.0", targets: {} },
    },
  ],
});

test("default harness install matrix covers every managed provider and platform target", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix, generatedAt: "2026-04-29" });
  const validation = validateManifest(manifest, providerMatrix);

  assert.deepEqual(validation.errors, []);
  assert.equal(validation.summary.providers, 2);
  assert.equal(validation.summary.cells, 2 * REQUIRED_PLATFORM_TARGETS.length);
  assert.equal(validation.summary.lane_counts.release, 8);
  assert.equal(validation.summary.lane_counts.nightly, 8);
  assert.equal(validation.summary.lane_counts.preview, 4);
});

test("flattenCells exposes runner-ready cell ids and install targets", () => {
  const manifest = buildDefaultManifest({ providerMatrix: providerMatrixFixture() });
  const cells = flattenCells(manifest);

  assert.ok(cells.some((cell) => cell.id === "codex.macos.host" && cell.install_target === "host"));
  assert.ok(cells.some((cell) => cell.id === "codex.macos.sandbox" && cell.install_target === "container"));
  assert.ok(cells.some((cell) => cell.id === "gemini.linux.sandbox" && cell.lanes.includes("release")));
});

test("validator rejects missing provider target cells", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix });
  delete manifest.providers.find((provider) => provider.id === "codex").cells["linux.sandbox"];

  const validation = validateManifest(manifest, providerMatrix);
  assert.ok(validation.errors.some((error) => error.includes("missing matrix cell: codex.linux.sandbox")));
});

test("validator rejects adapter ids as product providers", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix });
  manifest.providers.push({
    ...manifest.providers[0],
    id: "codex-crp",
  });

  const validation = validateManifest(manifest, providerMatrix);
  assert.ok(validation.errors.some((error) => error.includes("codex-crp") && error.includes("adapter/runtime id")));
});

test("validator rejects dependency providers as harness providers", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix });
  manifest.providers.push({
    ...manifest.providers[0],
    id: "codex-cli",
  });

  const validation = validateManifest(manifest, providerMatrix);
  assert.ok(validation.errors.some((error) => error.includes("codex-cli") && error.includes("dependency provider")));
});

test("validator rejects release-supported cells without live install runner", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix });
  manifest.providers[0].cells["macos.host"].runner = { kind: "none" };

  const validation = validateManifest(manifest, providerMatrix);
  assert.ok(validation.errors.some((error) => error.includes("codex.macos.host") && error.includes("release_runtime_install_smoke")));
});

test("report documents lane contract and canonical provider ids", () => {
  const providerMatrix = providerMatrixFixture();
  const manifest = buildDefaultManifest({ providerMatrix });
  const validation = validateManifest(manifest, providerMatrix);
  const report = buildReport(manifest, validation);

  assert.match(report, /Harness Install Matrix/);
  assert.match(report, /codex\.macos\.host/);
  assert.match(report, /adapter\/runtime ids such as `codex-crp`/);
});
