# Validation Log

Record commands, timestamps, exit status, resource caps, and notable warnings.

## Baseline Already Observed Before This Plan

- Web typecheck, lint, test, and build passed on the current branch before this
  expanded plan was written.
- Buildkite/Bazel shifted-left schema/config tests passed on the current branch
  before this expanded plan was written.
- Full Rust workspace tests passed through `core/scripts/dev/cargo-safe.sh` with
  memory/job/thread caps before this expanded plan was written.

These baseline results must be rerun after subsequent implementation phases.

## Phase 0 Focused Validation

- After `5dc809d`:
  - `pnpm -C core/apps/web test -- src/pages/workbenchShell/WorkbenchPageShellView.test.tsx src/pages/workbenchShell/WorkbenchTemplates.test.tsx src/workbench/persistence.test.ts src/workbench/store.template.test.ts src/utils/workbenchStoreLayout.test.ts src/pages/workbenchShell/agentWorkProjection.test.ts`
  - Result: passed, 6 files / 32 tests.
- After `729d953`:
  - `scripts/dev/cargo-safe.sh test --manifest-path Cargo.toml --locked -p ctx-store agent_work`
  - Result: passed, 11 tests.
- After `399b29e`:
  - `scripts/dev/cargo-safe.sh test --manifest-path Cargo.toml --locked -p ctx-daemon duplicate_plugin_ids_are_load_errors_and_not_registered`
  - Result: passed, 1 test. Existing daemon warnings remained warnings only.
- After `6a36194`:
  - `bash -n core/scripts/dev/cargo-safe.sh core/scripts/dev/check-local.sh`
  - Result: passed.

## Workbench Template Visual Slice

- Before commit:
  - `CTX_E2E_BROWSER=chromium CTX_E2E_BROWSER_CHANNEL=chrome CTX_E2E_DISABLE_VIDEO=1 pnpm -C core/apps/web exec playwright test -c playwright.visual.config.ts e2e/visual-workbench-templates.spec.ts --grep "Classic template desktop-wide"`
  - Result: passed, 1 test. Used system Chrome because this host lacks the
    Playwright-managed WebKit browser for this Ubuntu image; disabled video
    because the matching Playwright-managed ffmpeg binary is also absent.
- Before commit:
  - `CTX_E2E_BROWSER=chromium CTX_E2E_BROWSER_CHANNEL=chrome CTX_E2E_DISABLE_VIDEO=1 pnpm -C core/apps/web exec playwright test -c playwright.visual.config.ts e2e/visual-workbench-templates.spec.ts`
  - Result: passed, 14 tests / 39.8s, one Playwright worker.
- Before commit:
  - `pnpm -C core/apps/web test -- src/pages/workbenchShell/WorkbenchPageShellView.test.tsx src/pages/workbenchShell/WorkbenchTemplates.test.tsx src/workbench/persistence.test.ts src/workbench/store.template.test.ts src/utils/workbenchStoreLayout.test.ts src/pages/workbenchShell/agentWorkProjection.test.ts`
  - Result: passed, 6 files / 32 tests.
- Before commit:
  - `pnpm -C core/apps/web typecheck`
  - Result: passed.
