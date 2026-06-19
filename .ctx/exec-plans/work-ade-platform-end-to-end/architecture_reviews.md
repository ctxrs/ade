# Architecture Reviews

Record architecture review checkpoints and sign-offs.

## Pending Checkpoints

- After Work CLI/import/export design.
- After plugin SDK/contribution schema implementation.
- After hot reload implementation.
- Before final done-ness review.

## Plan Review Baseline

- Reviewer Ohm inspected the branch direction and first draft. Findings around
  Work source-of-truth, CLI/import/export, plugin security, hot reload, ACP
  provider contract, UX artifacts, review gates, and subagent workflow were
  incorporated into `exec_plan.md`.
- Reviewer Locke inspected the updated plan. Findings around SDK scope,
  storage semantics, ACP conformance, bundle safety, declarative data/action
  contracts, network-adjacent boundaries, and worker base-commit rules were
  incorporated into `exec_plan.md`.

Reviewer agents:

- Ohm: `019ee0b6-defd-7c51-be16-514e06259ca5`
- Locke: `019ee0bb-d702-7541-811a-585a218a38d1`

## Blocking Contract Base

- `3d1b60a` documents the Work namespace, Work source-of-truth/storage,
  ACP provider plugin, and plugin contribution contracts. Broad implementation
  workers must base on this commit or a later manager-owned contract commit.

## Contract Gap Review

- Reviewer Boyle (`019ee0c6-f450-7f50-bf4f-e48fa2bad5ee`) found six contract
  gaps after `3d1b60a`: diagnostics durability, importer write boundaries, ID
  collision policy, ACP target drift, old control-plane import semantics, and
  concrete worker write ownership.
- The manager resolved the first five by adding durable diagnostics, approved
  import/capture actions, ID-class collision rules, a local ACP v1 conformance
  target, and old control-plane historical import boundaries.
- Worker write ownership remains manager-enforced per spawned worker; no broad
  overlapping plugin/provider/runtime workers should start until each write set
  is assigned explicitly.
- Current contract base after this review is `8123c74`.
