# Harness Install Matrix

Generated from `core/apps/desktop/automation/fixtures/harness_install_matrix.json`.

- Generated at: 2026-04-29
- Source provider matrix: `core/crates/ctx-provider-accounts/src/provider_matrix.json`
- Providers: 15/15
- Platform targets: 4/4
- Cells: 60/60

## Lane Counts

- preview: 30
- release: 60
- nightly: 60

## Support Counts

- supported: 60
- unsupported: 0

## Cells

| cell | provider | platform | execution target | install target | support | lanes | runner |
| --- | --- | --- | --- | --- | --- | --- | --- |
| auggie.linux.host | auggie | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| auggie.linux.sandbox | auggie | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| auggie.macos.host | auggie | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| auggie.macos.sandbox | auggie | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| claude-crp.linux.host | claude-crp | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| claude-crp.linux.sandbox | claude-crp | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| claude-crp.macos.host | claude-crp | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| claude-crp.macos.sandbox | claude-crp | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| cline.linux.host | cline | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| cline.linux.sandbox | cline | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| cline.macos.host | cline | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| cline.macos.sandbox | cline | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| codex.linux.host | codex | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| codex.linux.sandbox | codex | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| codex.macos.host | codex | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| codex.macos.sandbox | codex | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| copilot.linux.host | copilot | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| copilot.linux.sandbox | copilot | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| copilot.macos.host | copilot | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| copilot.macos.sandbox | copilot | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| cursor.linux.host | cursor | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| cursor.linux.sandbox | cursor | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| cursor.macos.host | cursor | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| cursor.macos.sandbox | cursor | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| droid.linux.host | droid | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| droid.linux.sandbox | droid | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| droid.macos.host | droid | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| droid.macos.sandbox | droid | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| gemini.linux.host | gemini | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| gemini.linux.sandbox | gemini | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| gemini.macos.host | gemini | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| gemini.macos.sandbox | gemini | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| goose.linux.host | goose | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| goose.linux.sandbox | goose | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| goose.macos.host | goose | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| goose.macos.sandbox | goose | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| kimi.linux.host | kimi | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| kimi.linux.sandbox | kimi | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| kimi.macos.host | kimi | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| kimi.macos.sandbox | kimi | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| mistral.linux.host | mistral | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| mistral.linux.sandbox | mistral | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| mistral.macos.host | mistral | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| mistral.macos.sandbox | mistral | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| opencode.linux.host | opencode | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| opencode.linux.sandbox | opencode | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| opencode.macos.host | opencode | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| opencode.macos.sandbox | opencode | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| openhands.linux.host | openhands | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| openhands.linux.sandbox | openhands | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| openhands.macos.host | openhands | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| openhands.macos.sandbox | openhands | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| pi.linux.host | pi | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| pi.linux.sandbox | pi | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| pi.macos.host | pi | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| pi.macos.sandbox | pi | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |
| qwen.linux.host | qwen | linux | host | host | supported | release, nightly | release_runtime_install_smoke |
| qwen.linux.sandbox | qwen | linux | sandbox | container | supported | release, nightly | release_runtime_install_smoke |
| qwen.macos.host | qwen | macos | host | host | supported | preview, release, nightly | release_runtime_install_smoke |
| qwen.macos.sandbox | qwen | macos | sandbox | container | supported | preview, release, nightly | release_runtime_install_smoke |

## Contract

- Provider ids are canonical product ids from `provider_matrix.json`; adapter/runtime ids such as `codex-crp` do not satisfy a provider cell.
- `preview` covers macOS host and sandbox installability for the produced preview `.app`.
- `release` covers every installable provider on macOS host, macOS sandbox, Linux host, and Linux sandbox before stable promotion.
- `nightly` repeats installability and may add launch/probe/first-turn breadth where deterministic credentials exist.
- Live runners must continue through all selected cells and report every failed provider/target before exiting non-zero.
