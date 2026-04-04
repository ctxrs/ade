#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "desktop-sandbox-bootstrap-real-truth.yml");
const releaseWorkflowPath = path.join(repoRoot, ".github", "workflows", "release-supabase.yml");

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
  assert.match(workflow, /github\.event\.workflow_run\.event == 'push' \|\| github\.event\.workflow_run\.event == 'workflow_dispatch'/);
  assert.match(workflow, /Resolve release channel for Ubuntu local truth lane/);
  assert.match(workflow, /Resolve release channel for macOS remote truth lane/);
  assert.match(workflow, /workflow_run\.display_title/);
});

test("desktop sandbox bootstrap real-truth workflow covers fresh Ubuntu local install and macOS remote truth lanes", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  assert.match(workflow, /ubuntu-local-install-truth:/);
  assert.match(workflow, /ubuntu-local-install-truth:[\s\S]*if:\s*\$\{\{\s*\(github\.event_name != 'workflow_dispatch' \|\| inputs\.run_local_ubuntu\)/);
  assert.match(workflow, /platform:\s+linux-x64[\s\S]*runner:\s+ubuntu-24\.04/);
  assert.match(workflow, /platform:\s+linux-arm64[\s\S]*runner:\s+\$\{\{\s*vars\.RELEASE_RUNNER_LINUX_ARM64 \|\| 'linux-arm-8core'\s*\}\}/);
  assert.match(workflow, /scripts\/tests\/linux_local_install_sandbox_release_truth\.sh/);
  assert.match(workflow, /linux-local-install-truth-\$\{\{\s*matrix\.platform\s*\}\}-\$\{\{\s*github\.run_id\s*\}\}/);
  assert.match(workflow, /CTX_LINUX_RELEASE_TRUTH_ARTIFACT_DIR:\s+\$\{\{\s*runner\.temp\s*\}\}\/linux-local-install-truth\/\$\{\{\s*matrix\.platform\s*\}\}/);
  assert.match(workflow, /CTX_LINUX_RELEASE_TRUTH_CHANNEL=/);
  assert.match(workflow, /macos-remote-fresh-ubuntu-truth:/);
  assert.match(workflow, /macos-remote-fresh-ubuntu-truth:[\s\S]*if:\s*\$\{\{\s*\(github\.event_name != 'workflow_dispatch' \|\| inputs\.run_remote_macos\)/);
  assert.match(workflow, /runs-on:\s+\$\{\{\s*vars\.RELEASE_RUNNER_MACOS_ARM\s*\|\|\s*'macos-15'\s*\}\}/);
  assert.match(workflow, /scripts\/tests\/macos_remote_ubuntu_sandbox_release_truth\.sh/);
  assert.match(workflow, /CTX_UPDATER_E2E_BOOTSTRAP_REMOTE_CTX:\s+"1"/);
  assert.match(workflow, /CTX_UPDATER_E2E_BOOTSTRAP_CHANNEL=/);
  assert.match(workflow, /CTX_UPDATER_E2E_ARCHES:\s+linux-x64,linux-arm64/);
  assert.match(workflow, /Upload macOS remote truth artifacts/);
});

test("desktop sandbox bootstrap real-truth auto-follow pins the upstream release run-name contract", () => {
  const releaseWorkflow = fs.readFileSync(releaseWorkflowPath, "utf8");

  assert.match(
    releaseWorkflow,
    /run-name:\s+Release \(\$\{\{\s*github\.event_name == 'workflow_dispatch' && inputs\.channel \|\| 'stable'\s*\}\}\)/,
  );
});
