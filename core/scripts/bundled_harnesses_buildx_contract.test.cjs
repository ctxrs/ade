#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const bundledImagesScript = fs.readFileSync(
  path.join(repoRoot, "scripts", "lib", "bundled_harnesses_images.sh"),
  "utf8",
);
const dockerOnlySmoke = fs.readFileSync(
  path.join(repoRoot, "scripts", "tests", "ensure_bundled_harnesses_docker_only.sh"),
  "utf8",
);

test("bundled harness image export provisions an explicit docker-container builder", () => {
  assert.match(
    bundledImagesScript,
    /CTX_BUNDLE_BUILDX_BUILDER:-ctx-bundle-export/,
    "shared bundler should expose an overridable builder name",
  );
  assert.match(
    bundledImagesScript,
    /docker buildx create --name "\$builder_name" --driver docker-container --use/,
    "shared bundler should provision a docker-container builder when needed",
  );
  assert.match(
    bundledImagesScript,
    /docker buildx build \\\n\s+--builder "\$builder_name"/,
    "shared bundler should pin buildx export to the explicit builder",
  );
});

test("docker-only smoke injects an isolated buildx builder override", () => {
  assert.match(
    dockerOnlySmoke,
    /CTX_BUNDLE_BUILDX_BUILDER="\$temp_builder"/,
    "docker-only smoke should isolate buildx state with a temp builder",
  );
});
