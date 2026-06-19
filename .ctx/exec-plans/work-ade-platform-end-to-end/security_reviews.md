# Security Reviews

Record plugin, import/export, path, redaction, and capability security reviews.

## Pending

- Initial plugin threat model review.
- Import/export/redaction review.
- Final security review before full local validation.

## Work CLI Review-Hardening Slice

- Finding: transcript-like event payloads could retain raw text in redaction
  previews when the record shape used event fields instead of message fields.
  Resolution: event-aware omission now treats transcript-like `event_type`
  values and nested payload keys such as `content`, `delta`, `message`, `text`,
  `thought`, and `transcript` as content-bearing fields to omit.
- Finding: plugin manifest validation accepted shallow manifests with unknown
  fields before the daemon/plugin runtime saw them. Resolution: the CLI now
  rejects unknown public v1 manifest fields and delegates structural validation
  to the Rust `PluginManifest` model.
- Finding: shifted-left CLI smoke coverage did not exercise `work-bundle`
  schema output or negative path traversal fixtures. Resolution: the Bazel bin
  smoke test now covers `work-bundle` and rejects `../` bundle object paths.
- Residual risk: local plugin manifests still represent trusted local code once
  installed. The final plugin threat model must explicitly review root escape,
  env leakage, command timeout/output caps, provider ID collisions, and
  diagnostics visibility.

## Store-Backed Work CLI Slice

- Local import/export is scoped to public Work records only: change sets and
  contributions. It does not import hosted, team, enterprise, policy, gate, or
  enforcement state.
- `ctx work import` rejects records whose embedded workspace id does not match
  the selected local workspace before writing.
- `ctx work export` defaults to `safe-summary` redaction and requires explicit
  `--redaction-profile full-local` for raw local records.
- JSON stdout modes suppress diagnostics on stdout so downstream tools do not
  accidentally parse mixed data and diagnostic text.
- Residual risk: import writes are sequential through existing store APIs, not
  yet a single explicit transaction. Existing store validation protects
  workspace relationships and endpoint references, but transactional all-or-none
  import should be added when the store API gains a dedicated import bundle
  method.

## Plugin Contribution Collision Slice

- Duplicate provider/runtime contribution IDs are treated as hard load errors
  because those IDs determine provider authority and adapter ownership.
- Duplicate command/UI contribution IDs are warning diagnostics rather than hard
  errors because current command execution requires both `plugin_id` and
  `command_id`, and registry entries carry plugin identity. Public surfaces must
  still show source labels when displaying these collisions.
- Collision diagnostics are attached to plugin inventory items, so invalid
  provider ownership does not progress into provider adapter sync.
- Residual risk: conflicts with pre-existing non-plugin provider adapters are
  still handled during provider sync by warning and skip behavior; a later
  diagnostics slice should make that visible through the same diagnostic
  surface as plugin inventory collisions.

## Plugin Last-Good Reload Slice

- Last-good preservation is limited to recoverable manifest read, parse, and
  validation failures.
- Plugin inventory finalization now runs after preservation, so restored
  last-good manifests are still subject to duplicate plugin ID and
  provider/runtime authority collision checks.
- Duplicate plugin/provider/runtime collisions remain hard load errors and are
  excluded from extension registry projections even when one side is a
  preserved last-good plugin.
- The explicit regression matrix covers plugin ID, provider ID, and runtime ID
  collisions after last-good preservation.
- Residual risk: active plugin command behavior during reload/remove still
  needs an explicit lifecycle slice before plugin dev-loop semantics are
  complete.

## Declarative Plugin Contribution Contract Slice

- Rust manifest parsing now uses strict `deny_unknown_fields` on public manifest
  structs, so daemon loading rejects stray runtime-shaped declarative fields,
  processor buckets, and unknown top-level keys instead of silently dropping
  them.
- The new declarative Workbench buckets are host-owned declarations only. They
  identify host-known templates, renderer IDs, sections, data sources, and
  approved action or declared command targets; they do not load arbitrary
  renderer code, JavaScript modules, webview URLs, or React component names.
- Toolbar command targets now validate against commands declared by the same
  plugin. This avoids a misleading manifest where a toolbar button points at a
  nonexistent plugin command.
- Redaction/export processors remain out of the manifest and are only SDK
  sidecar markers in this slice.
- Residual risk: once arbitrary UI/webview execution is introduced, it needs a
  separate capability, permission, sandbox, root-escape, env-leakage, and
  lifecycle security review before it is considered local-done.

## Declarative Registry Projection And Slash Source Labels

- Daemon extension-registry projection copies declarative Workbench
  contributions as source-labeled data. It does not execute plugin entrypoints,
  renderer code, React components, webviews, or module paths.
- Duplicate declarative Workbench contribution IDs remain warning diagnostics
  because rendered/useful surfaces are plugin-qualified and source-labeled.
  Provider/runtime IDs remain authority-bearing hard errors.
- Web Workbench projection treats template/renderer IDs as compatibility data
  over host-owned primitives. Unsupported IDs are surfaced as unsupported data
  rather than executed.
- Slash command source labels are presentation-only. Provider commands and
  plugin commands keep their existing routing contracts; plugin command
  invocation still requires the namespaced plugin slash token.
- Residual risk: future executable UI/webview contributions and richer
  diagnostics surfaces need separate security review before local-done.

## Local Plugin CLI Slice

- `ctx plugin validate` parses through the strict Rust manifest model and runs
  manifest validation. Unknown fields and invalid contribution targets are
  rejected by the shared model rather than accepted by ad hoc CLI parsing.
- `ctx plugin list --json` and `ctx plugin reload --json` emit inventory
  metadata only: plugin id, load status, manifest path, and diagnostics. They
  do not emit plugin manifests, entrypoint command args, cwd, or environment
  variables.
- `list` and `reload` are local scanner commands. They do not execute plugin
  entrypoints and do not mutate a live daemon, active sessions, provider
  adapter ownership, or Work records.
- Residual risk: daemon-connected plugin reload/apply, plugin log streaming,
  executable UI/webview contributions, and plugin dev-mode process management
  need separate permission, provenance, redaction, and lifecycle review before
  local-done.
