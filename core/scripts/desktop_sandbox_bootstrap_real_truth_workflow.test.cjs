#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "desktop-sandbox-bootstrap-real-truth.yml");

test("desktop sandbox bootstrap real-truth workflow stays manual/scheduled and follows release runs", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  assert.match(workflow, /name:\s+Desktop sandbox bootstrap release truth/);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /workflow_run:\s*\n\s+workflows:\s*\n\s+-\s+"Release \(Supabase Storage\)"/);
  assert.match(workflow, /schedule:\s*\n\s+-\s+cron:/);
  assert.match(workflow, /cloud_provider:[\s\S]*options:[\s\S]*-\s+aws[\s\S]*-\s+hetzner/);
  assert.match(workflow, /run_local_ubuntu:[\s\S]*type:\s+boolean/);
  assert.match(workflow, /run_remote_macos:[\s\S]*type:\s+boolean/);
  assert.match(workflow, /run_remote_container:[\s\S]*type:\s+boolean/);
});

test("desktop sandbox bootstrap real-truth workflow covers fresh Ubuntu local install and macOS remote truth lanes", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  assert.match(workflow, /ubuntu-local-install-truth:/);
  assert.match(workflow, /runs-on:\s+ubuntu-24\.04/);
  assert.match(workflow, /scripts\/tests\/linux_local_install_sandbox_release_truth\.sh/);
  assert.match(workflow, /Upload Ubuntu local-install truth artifacts/);
  assert.match(workflow, /macos-remote-fresh-ubuntu-truth:/);
  assert.match(workflow, /runs-on:\s+\$\{\{\s*vars\.RELEASE_RUNNER_MACOS_ARM\s*\|\|\s*'macos-15'\s*\}\}/);
  assert.match(workflow, /scripts\/tests\/macos_remote_ubuntu_sandbox_release_truth\.sh/);
  assert.match(workflow, /CTX_UPDATER_E2E_BOOTSTRAP_REMOTE_CTX:\s+"1"/);
  assert.match(workflow, /CTX_UPDATER_E2E_ARCHES:\s+linux-x64/);
  assert.match(workflow, /Upload macOS remote truth artifacts/);
});
