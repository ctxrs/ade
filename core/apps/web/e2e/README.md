# Web E2E Suites

The source tree keeps browser e2e tests focused on local workbench
behavior. Tests should run against a local daemon or mocked browser fixtures and
must not require external CI services, hosted credentials, or organization-only infrastructure.

Run local browser contracts from `core/`:

```bash
pnpm -C apps/web exec playwright test -c playwright.premerge.config.ts
```

Visual tests may write local screenshots under the test output directory. Do
not configure this source tree to upload screenshots or diagnostics to hosted
services by default.

Template screenshots for Classic, Kanban, Multipane, and Review are captured by:

```bash
pnpm -C core/apps/web exec playwright test -c playwright.visual.config.ts e2e/visual-workbench-templates.spec.ts
```

The command starts the local E2E web server through Playwright on an ephemeral
port by default. Set `CTX_E2E_REUSE_SERVER=1 CTX_E2E_PORT=4401` to reuse a
server at `http://127.0.0.1:4401`. Local screenshots are written under
`core/apps/web/e2e/test-results/visual` and Argos-compatible PNG artifacts under
`$CTX_E2E_DATA_DIR/argos-screenshots` when that env var is set.

Provider tests that require real third-party credentials are opt-in. Keep those
credentials in your local environment and avoid committing provider tokens,
service-account JSON, browser profiles, or generated diagnostic bundles.
