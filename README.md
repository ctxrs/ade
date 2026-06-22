<p align="center">
  <img src="assets/readme/header-homepage-20260403.png" alt="ctx ADE desktop workbench for coding agents" />
</p>

**A local-first desktop workbench for coding agents.**

ctx ADE is an open-source Agentic Development Environment (ADE): one place to run, supervise, review, and land work from the coding agents you already use.

If you use Claude Code, Codex, Cursor, or other agents across multiple tasks, the work quickly spreads across tmux panes, terminal tabs, provider session files, worktrees, diffs, screenshots, and GitHub tabs. ctx ADE pulls that workflow into one local, hackable desktop app: tasks, sessions, transcripts, artifacts, diffs, containers, remote machines, and merge queue state all live in the same review surface.

Use ctx ADE on your own machine or against a remote devbox or VPS you control. ctx ADE is local-first: your repos, task state, transcripts, and artifacts live on the machine running the workspace, and you bring your own agents, providers, models, and credentials.

<p align="center">
  <img src="assets/videos/ctx-homepage-demo-20260406-900w-6s-10fps.gif" alt="A short demo of ctx showing the workbench, task thread, and agent workflow." />
</p>

## Install

```bash
curl -fsSL https://ctx.rs/install | sh
```

Platform support: macOS and Linux today. Windows is on the roadmap.

- Website: https://ctx.rs
- Blog: https://ctx.rs/blog
- Install guide: https://ctx.rs/getting-started/install-and-launch/

## What ctx ADE helps you do

- Use Claude Code, Codex, Cursor, and other coding agents in a desktop workbench instead of flickering terminal panes
- Run agents in isolated containers with explicit disk and network controls
- Let agents run in yolo mode inside explicit disk and network boundaries instead of choosing between approval prompts and full access to your shell, files, and network
- Keep tasks, sessions, diffs, transcripts, and artifacts in one review surface
- Run work locally or on remote machines you control
- Keep parallel tasks isolated in separate worktrees and land them cleanly with the agent merge queue

## Why ctx ADE exists

- A bare diff is not enough: ctx ADE keeps the prompt, transcript, commands, artifacts, and worktree state that produced it
- Parallel agents need isolated worktrees and a sane way to land branches without manual branch juggling
- Agent sessions should be durable and inspectable instead of trapped in one provider's app state
- You should be able to change harnesses and models without rebuilding your whole workflow
- Teams can add stronger runtime, review, and provenance controls later without forcing everyone into one agent

## How is this different from Codex app?

Codex app, Cursor, and Antigravity are first-party environments for their own agent stacks. ctx ADE is an open-source, local-first workbench around the agent harness you use today and the ones you may want to try later.

If one vendor's app is already your whole workflow, you may prefer that app. ctx ADE is for engineers who want the workbench to stay stable as agents, models, and harnesses change.

The difference is the layer ctx ADE cares about. ctx ADE focuses on task state, worktrees, transcripts, artifacts, diffs, containers, remote machines, review, and landing branches. The workbench itself is open source, so you can inspect it, modify it, script it, and keep your workflow independent of any single agent provider.

## The Pi ethos, one layer up

Pi makes the case that an agent harness should be yours: inspectable, adaptable, built from primitives, and shaped around your workflow instead of sealed behind a vendor product.

ctx ADE is trying to bring that same ethos to the Agentic Development Environment. The ADE is the layer where agent sessions run, transcripts accumulate, diffs are reviewed, artifacts are captured, and branches land. If that layer becomes central to software development, it should be open and hackable too.

ctx ADE does not yet have Pi-style extensibility or plugin primitives. Making the workbench more extensible, plugin-ready, and hot-reloadable is an active area of development. If that direction interests you, we would love contributions.

## Under the hood

ctx ADE is built around a local Rust daemon, real agent harnesses, and per-task worktrees.

- The workbench talks to a local Rust daemon that owns sessions, transcripts, artifacts, diffs, workspace state, provider setup, and merge queue state
- ctx ADE runs real agent harnesses instead of replacing them with one internal agent loop; adapters use structured runtime protocols where available instead of scraping terminal output
- CRP adapters normalize provider streams into one durable session model for the UI, review surface, local SQLite store, and artifact system
- Each task runs in its own worktree, so transcripts, artifacts, diffs, and review state are tied to the exact execution root that produced them
- Sandbox runs are materialized into an isolated container data plane instead of mutating a host checkout in place
- Containerized runs can use explicit network egress policies such as LLM-provider-only, allowlist, or full access
- The local merge queue replays patches against the latest target branch in queue worktrees, runs verification, and only advances the target when the entry applies cleanly
- The desktop app is Tauri with a TypeScript UI, but the runtime path is Rust. The repo uses Bazel for the larger validation graph alongside normal Cargo and pnpm workflows

## Get started

- [Install and launch](docs/getting-started/install-and-launch.mdx)
- [Connect a provider](docs/getting-started/connect-provider.mdx)
- [Add a workspace](docs/getting-started/add-workspace.mdx)
- [Run your first task](docs/getting-started/first-task.mdx)

## Learn more

- [Workbench tour](docs/workbench/tour.mdx)
- [Containerization](docs/containerization.mdx)
- [Agent Merge Queue Overview](docs/agent-merge-queue-overview.mdx)
- [What is a worktree?](docs/what-is-a-worktree.mdx)
- [ADE vs CLI](docs/ade-vs-cli.mdx)
- [ADE vs IDE](docs/ade-vs-ide.mdx)

## Build From Source

Prerequisites:

- Rust stable
- Node.js 20+ with Corepack
- Platform desktop dependencies if building the Tauri app, including GTK/WebKitGTK on Linux

Install dependencies and build the Rust workspace:

```bash
git clone https://github.com/ctxrs/ade.git
cd ade
cd core
corepack enable
pnpm install --frozen-lockfile
cargo build --workspace
```

Run the daemon and web workbench locally in separate terminals:

```bash
cd core
cargo run -p ctx-http --bin ctx -- serve --bind 127.0.0.1:4399 --data-dir "${CTX_DATA_DIR:-$HOME/.ctx}"
```

```bash
cd core
pnpm -C apps/web dev
```

Launch the desktop app from source:

```bash
cd core
pnpm desktop:dev
```

Build the desktop app from source:

```bash
cd core
pnpm desktop:build
```
