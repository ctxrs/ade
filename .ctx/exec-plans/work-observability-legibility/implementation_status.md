# Work Observability And Legibility Implementation Status

Task: `feb64c1c-e58c-40f8-b1e9-1094dca0646e`
Branch: `ctx/agent-work-semantics-primary`
Worktree: `/home/daddy/code/ctx-multi-repo-workspace/worktrees/ctx/agent-work-semantics-primary`
Starting head: `bf11a2c Record Work productization follow-up`

## Manager plan

1. Commit this reviewed implementation plan and status baseline.
2. Map existing storage, CLI, daemon, web, redaction, and artifact surfaces.
3. Implement the P0 local Work observability slice in narrow, reviewable commits:
   store/model, capture/indexing, CLI/API contract, Work Report UI, docs.
4. Keep hosted/team/enterprise, remote push, PR, release, and broad remote CI out
   of scope.
5. Use resource-safe Rust commands and avoid concurrent broad Cargo runs on this
   host.
6. Run focused validation after each slice, then adversarial review subagents.
7. Record final validations, accepted deferrals, and done-ness review result in
   this plan directory before final response.

## Subagents

Read-only exploration started:

- Data/store explorer: `019ee7e1-13d2-7bc3-9dca-bff96d91e067`
- CLI/API explorer: `019ee7e1-31b8-7ab1-b1bf-456f5f725fe4`
- Web/ADE explorer: `019ee7e1-46c2-7b83-9cd9-1c7a197d6bbd`
- Redaction/artifact explorer: `019ee7e1-7449-7951-ad5d-a3c5af54db75`

Planned implementation/review teams:

- Data model/store worker
- Capture/indexing worker
- CLI/agent-contract worker
- Daemon/API worker
- Work Report UI worker
- Docs/product worker
- Architecture/data-model reviewer
- Product/UX reviewer
- Agent-contract reviewer
- Security/privacy reviewer
- Test-coverage reviewer
- SDLC/resource-safety reviewer
- Final done-ness reviewer

## Status log

- Baseline plan read. Canonical branch/worktree verified.
- Repo-local `AGENTS.md` was not present in this worktree; parent workspace
  instructions apply.
- Only uncommitted state at start was this plan directory.
- Added first implementation slice:
  - Work observability IDs and typed records;
  - `0076_work_observability.sql` tables and FTS5 search table;
  - `ctx-store` Work CRUD/search APIs;
  - `ctx work` commands for search, context, report, timeline, evidence,
    summarize, link-commit, and index rebuild;
  - capture/link-pr/note now materialize durable Work records and redacted
    timeline/search docs.
- Validation: `scripts/dev/cargo-safe.sh check --manifest-path Cargo.toml -p
  ctx-http --locked` passed under the global Cargo lock and capped memory.

## Final local implementation status

Local code head before this status note: `b3c6126 Test Work observability
safety states`.

Landed commits for this pass:

- `538f564 Add Work observability implementation plan`
- `23077da Add local Work observability records`
- `297e667 Expose safe Work observability routes`
- `f79d2d8 Add Work Report web surface`
- `b3c6126 Test Work observability safety states`

The P0 local Work observability slice is implemented in the public `ctx`
worktree, without hosted/team sync, remote push, PR creation, or release work.

What landed:

- Durable Work Record model:
  - `work_records`, `work_record_links`, `work_events`, `work_evidence`,
    `work_summaries`, `work_summary_claims`, and redacted FTS search docs in
    migration `0076_work_observability.sql`.
  - Typed IDs and Rust models for Work records, links, events, evidence,
    summaries, claims, search documents, trust, freshness, fidelity, source,
    lifecycle, and redaction classes.
  - `ctx-store` APIs for creating, linking, indexing, searching, reporting, and
    refreshing Work evidence/search material.
- Local capture and agent contract:
  - `ctx work search`, `context`, `report`, `timeline`, `evidence`,
    `summarize`, `link-commit`, and `index rebuild` surfaces.
  - `capture`, `link-pr`, and `note` now materialize Work records and redacted
    Work events/search documents while preserving existing change-set and
    contribution compatibility.
  - CLI JSON output is the stable local agent contract for P0.
- Daemon API:
  - Workspace Work route handles and HTTP routes for listing/fetching Work,
    context packs, reports, timeline, evidence, evidence creation, and
    summaries.
  - Route-contract DTOs intentionally avoid raw core payload exposure for
    events/evidence/report material.
  - Context and route text are bounded and redacted by default.
- Legibility:
  - Trust and evidence freshness are recomputed from current evidence state
    where reports/context need them.
  - Summaries have source revision keys and become stale when source material
    changes.
  - Duplicate strong links for PR/commit targets are detectable and reportable
    instead of silently forking reviewer truth.
- Work Report web surface:
  - `/workspaces/:workspaceId/work/:workId` route.
  - Work Report page with header, trust verdict, evidence counts, linked targets,
    change summary, timeline/details, and reviewer next-action copy.
  - Shared `ctx-types` Work DTOs and web API client helpers.
- Documentation:
  - README/docs now position ctx as Work-first with the ADE as optional.
  - Work Records, source-of-truth, compatibility, privacy, setup, and ADE-vs-CLI
    docs describe the local Work contract and current limits.
- Resource safety:
  - Validation used the repo-local `scripts/dev/cargo-safe.sh` wrapper, global
    Cargo lock, low I/O priority, capped jobs/test threads, and memory cgroup
    where available.

## Review status

Adversarial review findings addressed during this pass:

- Architecture/data:
  - Added daemon/API exposure instead of leaving the model CLI-only.
  - Added duplicate detection for strong PR/commit Work links.
  - Recomputed report/context trust from evidence instead of trusting stale
    cached record fields.
  - Reindexed evidence/search material after freshness changes.
  - Made summary freshness depend on material revision keys.
- Product/UX:
  - Added a dedicated Work Report route/page instead of only raw CLI output.
  - Kept first-viewport focus on title, linked targets, trust, evidence,
    change summary, and reviewer next action.
- Agent contract:
  - Kept `ctx work ... --json` as the P0 stable agent surface.
  - Added daemon JSON routes over the same local model for ADE/web clients.
- Security/privacy:
  - Redacted search output and indexed titles/paths.
  - Made FTS title unindexed and avoided raw repo-root leakage in default search
    output.
  - Added route DTOs that omit raw `payload_json` and `artifact_ref` by
    default.
  - Bounded context/report text and kept raw transcript inclusion false by
    default.
- Test coverage:
  - Added focused tests for trust aggregation, summary freshness, route event
    redaction, CLI parsing/round-trips, Work client URLs, and Work Report
    rendering.

Final done-ness review is still required after this status note is committed.

## Validation results

Passed:

- `scripts/dev/cargo-safe.sh check --manifest-path Cargo.toml -p ctx-http
  --locked`
- `scripts/dev/cargo-safe.sh test --manifest-path Cargo.toml -p ctx-http --bin
  ctx agent_work_cli::tests --locked`
  - 35 passed; 0 failed; 16 filtered.
- `scripts/dev/cargo-safe.sh test --manifest-path Cargo.toml -p ctx-daemon
  --lib workspaces::route_contract::work::tests --locked`
  - 1 passed; 0 failed; 573 filtered.
  - Existing daemon unused-import/dead-code warnings were observed and left
    unchanged.
- `pnpm -C core/apps/web exec vitest run
  src/api/clientWorkspaces.work.test.ts
  src/pages/workReport/WorkReportView.test.tsx`
  - 2 test files passed; 2 tests passed.
- `pnpm -C core/apps/web typecheck`
- `pnpm -C core/apps/web lint`
- `pnpm -C core/apps/web build`
  - Existing Browserslist, dynamic/static import, and large chunk warnings were
    observed and left unchanged.
- `cargo fmt --manifest-path core/Cargo.toml --all -- --check`
- `git diff --check`

Not run by design:

- Broad Rust workspace test suite, broad Bazel/Buildkite sweeps, desktop package
  builds, remote CI, release, hosted/team validation, and PR/push flows. These
  are Tier 3/out-of-scope for this local pass and were avoided to keep this host
  stable.

## Accepted deferrals

- Hosted/team/enterprise Work sync and transcript data lake.
- Provider-backed LLM summaries.
- MCP tools/resources over the Work daemon contract.
- Git notes, commit trailers, PR comments, or any mutation of user commits.
- Arbitrary executable plugin/UI runtime work.
- Semantic/vector search.
- Full raw transcript expansion in default reports/context packs.
- Broad Buildkite/remote CI and release validation.
