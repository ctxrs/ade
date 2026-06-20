# SDLC Reviews

Record process, worktree, validation, and agent-workflow reviews.

## Pending

- Initial SDLC review after Phase 0 commit hygiene.
- Final SDLC review before full local validation.

## Plan Review Baseline

- The execution plan now requires manager-owned contract commits before broad
  parallel worker fan-out.
- Worker handoffs must include base commit, diff stat, invariants changed, tests
  run, residual risks, expected conflicts, and integration notes.
- Heavy validation remains serialized and memory-capped.

## Local Cargo Pressure Review

- Finding: multiple agents running independent local Cargo commands can freeze
  this host even when each command uses `-j 1`, because linking large Rust test
  binaries still creates root-drive write pressure and `-j 1` is only
  per-process.
- Process rule: broad local Rust validation must go through
  `core/scripts/dev/cargo-safe.sh`, which now owns a host-level Cargo lock and
  low-I/O priority policy. Agents must not run direct concurrent Cargo commands
  for broad verification.
- Host-specific mitigations such as APST, scheduler, dirty writeback, and
  `cargo-lowio` are useful on this machine, but the repo process must not depend
  on that host setup existing.
- Reviewer Lagrange (`019ee177-56e3-7520-b1a8-3304e456f331`) found no blockers
  in the wrapper change. The manager addressed the non-blocking concerns by
  probing `ionice` before using it and documenting that the lock is cooperative,
  so direct Cargo/Bazel can still bypass it.
- Follow-up finding during Workbench visual validation: managed Playwright
  launched `cargo run -p ctx-http --bin ctx` directly through
  `apps/web/scripts/start-e2e-server.mjs`, bypassing the cooperative host lock.
  The manager stopped that run and changed local-build E2E server launches to
  resolve `scripts/dev/cargo-safe.sh` by default on Unix when present, while
  preserving Bazel-runfiles mode and adding explicit override/opt-out env vars.
- Follow-up process rule: before any Rust validation, the manager checks for
  active `cargo`, `rustc`, `rust-lld`, or `/tmp/ctx-cargo.lock` processes and
  runs local Rust gates with `CTX_CARGO_MEMORY_MAX_GIB=24`, `CTX_CARGO_JOBS=1`,
  and `CTX_RUST_TEST_THREADS=1`.

## Shift-Left Gate Review

- The branch now has a `check-local.sh quick` mode for schema compilation,
  `@ctx/types` typecheck, and web typecheck. This gives workers a cheap
  contract gate before they attempt heavier Rust, web, or visual runs.
- Bazel coverage now includes schema compile tests and `@ctx/types` typecheck
  in addition to JSON syntax checks.
- Local dependency cleanup must be followed by the quick gate and, where Bazel
  data labels change, a targeted Bazel check before final done-ness review.

## Contract Base

- Broad implementation workers must base on `8123c74` unless the manager records
  a later contract base.

## Final Local Process Review

- The manager kept broad Rust validation serialized through
  `scripts/dev/cargo-safe.sh` with `CTX_CARGO_MEMORY_MAX_GIB=24`,
  `CTX_CARGO_JOBS=1`, and `CTX_RUST_TEST_THREADS=1`, checking for active
  Cargo/rustc/linker processes before each Rust-capable gate.
- Managed Playwright visual runs now inherit the same safe wrapper path for
  local-build server launches, so visual E2E no longer bypasses the host Cargo
  lock.
- The final local validation set intentionally favors affected-crate Rust gates,
  full web gates, source-boundary scans, and targeted visual E2E over broad
  concurrent workspace Cargo. This matches the user-provided host mitigation:
  avoid multiple local Cargo invocations and avoid broad workspace tests on this
  machine unless explicitly needed.
- Buildkite/remote, pushing, PRs, release artifacts, hosted/team services, and
  production promotion remain out of local scope for this branch.
