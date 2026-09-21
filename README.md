# ADE

ADE is a desktop workbench for running coding agents against local and remote repositories. It brings agent sessions, terminals, files, Git state, provider setup, and workspace isolation into one application.

This repository is the open-source release of the original ctx Agent Development Environment. It contains the retained product history from December 2025 through ADE 0.69.7, plus the final web-launcher fix. Internal planning, private operations, and personal data were removed while preserving product source and tests.

## What is here

- A Tauri desktop app with a React workbench and a local Rust daemon.
- Adapters for Codex, Claude, Gemini, Copilot, Cursor, OpenHands, Goose, and other agent runtimes.
- Local, SSH, and isolated sandbox workspaces.
- Concurrent sessions, child-agent tracking, task queues, terminals, attachments, and session history.
- Managed provider and runtime installation with pinned artifacts and checksums.
- Web, desktop, release, updater, mobile-access, telemetry, and control-plane components.
- A large test suite spanning unit, contract, browser, desktop, provider, and release behavior.

Provider model lists are discovered from the installed runtime when the provider supports it. A saved model choice is scoped to that provider, so new model IDs can be used without changing ADE's source.

## Project status

This is a source release of the last ADE product line. Active ctx development moved to the CLI after 0.69.7. The code is useful as a working system, a reference implementation, and a record of the engineering behind the product; it is not currently maintained as a supported hosted service.

Some installation, update, telemetry, and remote-access paths refer to public infrastructure under `ctx.rs`. The workers and service implementations are included in this repository, but a new operator must supply their own deployment configuration, credentials, signing keys, and artifact hosting. Local development does not require those hosted services.

## Repository map

| Path | Purpose |
| --- | --- |
| `core/apps/desktop` | Tauri desktop shell and desktop automation |
| `core/apps/web` | React workbench |
| `core/crates` | Rust daemon, workspace, provider, sandbox, session, and transport services |
| `external-harnesses` | Agent protocol bridges maintained with ADE |
| `harness-adapters` | Adapters for third-party coding agents |
| `control-plane-worker` | Team, entitlement, billing, and mobile-access API surface |
| `release-api-worker` | Release manifest and download service |
| `llm-relay-worker` | Optional model relay |
| `telemetry-worker` | Optional telemetry ingress |
| `install-site` | Installer and uninstaller endpoints |

## Development

The checked-in build uses Node 22.22.2, pnpm 9.15.1, Rust, and Bazel through Bazelisk. Desktop builds also need the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/).

Install JavaScript dependencies:

```sh
npx pnpm@9.15.1 -C core install --frozen-lockfile
```

Run the daemon and web workbench:

```sh
npx pnpm@9.15.1 -C core dev:daemon:web
```

Run a profile-scoped desktop development build:

```sh
make desktop-profile-dev PROFILE=dev PNPM="npx pnpm@9.15.1"
```

Run the quick verification suite:

```sh
make verify-quick PNPM="npx pnpm@9.15.1"
```

The repository contains broader merge, nightly, desktop, provider, and release suites. Several of those need platform-specific runners, provider credentials, remote hosts, or release infrastructure.

## License

Copyright © 2025–2026 Wills Manley and contributors.

ADE is released under the [GNU General Public License v3.0](LICENSE.md). Third-party code and assets retain their own licenses in the directories where they are included.
