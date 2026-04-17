const FAMILIES = [
  {
    id: "repo-contracts",
    title: "Repo Contracts",
    description: "Repo-owned docs, scripts, manifests, and pipeline contracts.",
  },
  {
    id: "toolchain-bootstrap",
    title: "Toolchain Bootstrap",
    description: "Machine bootstrap, install prerequisites, and tool availability contracts.",
  },
  {
    id: "build-graph",
    title: "Build Graph",
    description: "Bazel, package-manager, crate-graph, and compilation orchestration shape.",
  },
  {
    id: "workspace-stream",
    title: "Workspace Stream",
    description: "Workspace snapshots, stream ordering, replay, gap handling, and cache rehydration.",
  },
  {
    id: "provider-auth",
    title: "Provider Auth",
    description: "Provider installation, account state, auth import, and auth callback flows.",
  },
  {
    id: "provider-runtime",
    title: "Provider Runtime",
    description: "Runtime model selection, provider probes, runtime launch, and provider lifecycle behavior.",
  },
  {
    id: "repo-vcs",
    title: "Repo VCS",
    description: "Repo bootstrap, worktree lifecycle, merge queue, VCS snapshots, and diff flows.",
  },
  {
    id: "lsp-editing",
    title: "LSP Editing",
    description: "LSP integration, editing plans, file completions, and buffer orchestration.",
  },
  {
    id: "turns-terminal",
    title: "Turns And Terminal",
    description: "Turns, streaming assistant output, terminal orchestration, and message durability.",
  },
  {
    id: "attachments-artifacts",
    title: "Attachments And Artifacts",
    description: "Attachment upload/materialization, artifact routing, and attachment-facing UI flows.",
  },
  {
    id: "subagents-orchestration",
    title: "Subagents And Orchestration",
    description: "Subagent control, MCP/oracle integration, and orchestration behavior.",
  },
  {
    id: "updates-release",
    title: "Updates And Release",
    description: "Updater UX, release safety, channel state, manifests, and release-facing correctness.",
  },
  {
    id: "sandbox-runtime",
    title: "Sandbox Runtime",
    description: "Sandbox execution, cloud/runtime boundaries, and external runtime behavior.",
  },
  {
    id: "web-workbench",
    title: "Web Workbench",
    description: "Workbench UI, frontend state, diff/editor UX, and browser-facing product behavior.",
  },
  {
    id: "desktop-shell",
    title: "Desktop Shell",
    description: "Desktop automation, shell integration, local app UX, and desktop-only flows.",
  },
  {
    id: "distribution-install",
    title: "Distribution And Install",
    description: "Install surfaces, package assembly, runtime locks, and platform delivery contracts.",
  },
  {
    id: "settings-config",
    title: "Settings And Config",
    description: "Settings, config surfaces, and user-facing configuration behavior.",
  },
  {
    id: "artifacts-provenance",
    title: "Artifacts And Provenance",
    description: "Archive integrity, provenance policy, checksums, and artifact lineage.",
  },
  {
    id: "resilience-performance",
    title: "Resilience And Performance",
    description: "Anomaly, fault injection, fuzzing, load, soak, and performance safety work.",
  },
];

function getFamiliesById() {
  return new Map(FAMILIES.map((family) => [family.id, family]));
}

module.exports = {
  FAMILIES,
  getFamiliesById,
};
