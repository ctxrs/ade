> NOTE: Development on the ctx ADE has stopped in favor of using first-party desktop apps. Specifically, I am using the Codex desktop app now with a detached model router, paired with ctx CLI for agent history search.
>
> The biggest reason for this: ACP (or in our case, CRP) is not rich enough of a protocol for multi-agent orchestration at scale. If you are making a first party desktop app, you can make custom APIs to talk directly to subagents status and lots of other long-tail stuff that isn't exposed top level of a session event stream.
>
> In theory, you could build richer protocols for every single harness, but they are starting to be so divergent that it doesn't make sense to make one desktop app to handle every harness. Maybe someone will do it one day, but for now the best bet is to pick whichever desktop app works best in a 1:1 relationship with the harness.
>
> If anyone wants to pick up work from here, my recommendation is to take this as a baseline, strip out all of the harness agnostic stuff, pick a single open harness (such as Pi or maybe Fx) and build everything around that. Make it interoperable with OpenAI and Anthropic endpoints and subscriptions, and try to make it good enough that nobody misses the first party harness.
>
> There are some really nice parts in here that I like such as the UI scroll virtualization. And I think that the ACP community can learn from the CRP implementation which was much stabler at scale.

# ctx ade

ctx ade (Agentic Development Environment) is a multi-harness desktop app for steering many concurrent coding agents.

Its built around harnesses like Claude Code, Codex, and Cursor, so you can keep using the same agent that you already like in the terminal, but in a desktop GUI.

The desktop interface affords many UX capabilities that are simply not available in the terminal, even with multiplexing solutions like tmux and zellij.

The ctx ade includes support for remote development via SSH and integrated sandboxing.

## Under the hood

- A Tauri desktop app with a React workbench and a local Rust daemon.
- Adapters for Codex, Claude, Gemini, Copilot, Cursor, OpenHands, Goose, and other agent runtimes.
- Local, SSH, and isolated sandbox workspaces.
- Concurrent sessions, child-agent tracking, task queues, terminals, attachments, and session history.
- Managed provider and runtime installation with pinned artifacts and checksums.
- Web, desktop, release, updater, mobile-access, telemetry, and control-plane components.
- A large test suite spanning unit, contract, browser, desktop, provider, and release behavior.

Provider model lists are discovered from the installed runtime when the provider supports it. A saved model choice is scoped to that provider, so new model IDs can be used without changing ADE's source.

## Repository map

| Path                   | Purpose                                                                    |
| ---------------------- | -------------------------------------------------------------------------- |
| `core/apps/desktop`    | Tauri desktop shell and desktop automation                                 |
| `core/apps/web`        | React workbench                                                            |
| `core/crates`          | Rust daemon, workspace, provider, sandbox, session, and transport services |
| `external-harnesses`   | Agent protocol bridges maintained with ADE                                 |
| `harness-adapters`     | Adapters for third-party coding agents                                     |
| `control-plane-worker` | Team, entitlement, billing, and mobile-access API surface                  |
| `release-api-worker`   | Release manifest and download service                                      |
| `llm-relay-worker`     | Optional model relay                                                       |
| `telemetry-worker`     | Optional telemetry ingress                                                 |
| `install-site`         | Installer and uninstaller endpoints                                        |

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

The repo contains merge, nightly, desktop, provider, and release suites. Several of those need platform-specific runners, provider credentials, remote hosts, or release infrastructure.

## License

Copyright © 2026 ctx engineering inc and contributors.

ctx ade is released under the [Apache License 2.0](LICENSE.md). Third-party code and assets retain their own licenses in the directories where they are included.
