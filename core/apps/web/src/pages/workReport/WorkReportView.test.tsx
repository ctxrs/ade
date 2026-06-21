import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkspaceWorkInspector } from "@ctx/types";
import { WorkInspectorView } from "./WorkReportView";

const baseReport = (): WorkspaceWorkInspector => ({
  work: {
    work_id: "wrk_1234567890",
    workspace_id: "workspace-1",
    title: "Stabilize Work inspector route",
    objective: "Make local Work records legible",
    lifecycle: "ready_for_review",
    primary_branch: "ctx/work-observability",
    base_commit: null,
    head_commit: "abcdef1234567890",
    trust_verdict: "stale",
    summary_freshness: "stale",
    created_at: "2026-06-21T00:00:00Z",
    updated_at: "2026-06-21T00:01:00Z",
    schema_version: 1,
  },
  links: [],
  overview: {
    title: "Stabilize Work inspector route",
    objective: "Make local Work records legible",
    lifecycle: "ready_for_review",
    primary_branch: "ctx/work-observability",
    base_commit: null,
    head_commit: "abcdef1234567890",
    created_at: "2026-06-21T00:00:00Z",
    updated_at: "2026-06-21T00:01:00Z",
  },
  trust: {
    verdict: "failed",
    reason: "At least one linked evidence item failed.",
    recommended_next_action: "Fix the failing evidence before marking this ready.",
    open_risks: ["At least one linked evidence item failed."],
  },
  context: {
    value: {
      budget_tokens: 4000,
      summary: "Only redacted context is present.",
    },
    redacted: true,
    redaction_notes: ["test fixture"],
  },
  safe_json: {
    value: {
      safe_marker: "safe-visible-marker",
      redacted: "[redacted:workspace_root]",
    },
    redacted: true,
    redaction_notes: ["test fixture"],
  },
  raw_redacted_json: {
    value: {
      safe_marker: "safe-visible-marker",
      redacted: "[redacted:workspace_root]",
    },
    redacted: true,
    redaction_notes: ["test fixture"],
  },
  evidence_summary: {
    total: 2,
    passing: 1,
    failing: 1,
    stale: 1,
    missing: 0,
  },
  evidence: [
    {
      evidence_id: "wevdc_fail",
      work_id: "wrk_1234567890",
      workspace_id: "workspace-1",
      kind: "test",
      status: "observed_fail",
      freshness: "stale",
      claim: "Observed cargo test exited 101",
      command: "cargo test -p ctx-http",
      argv: ["cargo", "test", "-p", "ctx-http"],
      cwd: "[redacted:workspace_root]",
      exit_code: 101,
      head_sha: "abcdef1234567890",
      branch: "ctx/work-observability",
      output_ref: { log: "redacted failure output" },
      artifact_ref: { path: "[redacted:artifact]" },
      source: "worktree",
      fidelity: "exact",
      trust: "medium",
      started_at: "2026-06-21T00:00:00Z",
      finished_at: "2026-06-21T00:01:00Z",
      created_at: "2026-06-21T00:01:00Z",
      updated_at: "2026-06-21T00:01:00Z",
      schema_version: 1,
    },
    {
      evidence_id: "wevdc_pass",
      work_id: "wrk_1234567890",
      workspace_id: "workspace-1",
      kind: "lint",
      status: "observed_pass",
      freshness: "fresh",
      claim: "Observed lint exited 0",
      command: "pnpm lint",
      argv: ["pnpm", "lint"],
      cwd: "[redacted:workspace_root]",
      exit_code: 0,
      head_sha: "abcdef1234567890",
      branch: "ctx/work-observability",
      output_ref: null,
      artifact_ref: null,
      source: "worktree",
      fidelity: "exact",
      trust: "medium",
      started_at: "2026-06-21T00:02:00Z",
      finished_at: "2026-06-21T00:03:00Z",
      created_at: "2026-06-21T00:03:00Z",
      updated_at: "2026-06-21T00:03:00Z",
      schema_version: 1,
    },
  ],
  change_summary: {
    change_sets: 1,
    contributions: 2,
    pull_requests: [
      {
        provider: "github",
        owner: "ctxrs",
        repo: "ctx",
        number: 123,
        title: "Unsafe stored PR",
        url: "javascript:alert(1)",
        state: "draft",
      },
    ],
    commits: ["abcdef1234567890"],
  },
  artifact_summary: {
    total: 1,
    refs: [],
  },
  change_sets: [],
  contributions: [],
  summaries: [
    {
      summary_id: "wsum_1",
      work_id: "wrk_1234567890",
      workspace_id: "workspace-1",
      kind: "report_summary",
      audience: "reviewer",
      text: "Evidence is present but one item is stale.",
      structured_json: null,
      generation_method: "deterministic",
      provider: null,
      model: null,
      template: "ctx.work.deterministic.v1",
      source_material_left_machine: false,
      freshness: "stale",
      source_revision_key: "rev-1",
      generated_at: "2026-06-21T00:04:00Z",
      created_at: "2026-06-21T00:04:00Z",
      updated_at: "2026-06-21T00:04:00Z",
      schema_version: 1,
    },
  ],
  summary_claims: [],
  timeline: [
    {
      event_id: "wev_1",
      work_id: "wrk_1234567890",
      workspace_id: "workspace-1",
      sequence: 1,
      source_kind: "evidence",
      source_id: "wevdc_fail",
      event_type: "evidence_observed",
      event_time: "2026-06-21T00:01:00Z",
      actor_kind: "system",
      provider: null,
      harness: null,
      model: null,
      redaction_class: "local_redacted",
      source: "worktree",
      fidelity: "exact",
      trust: "medium",
      redacted_text: "Observed redacted command output.",
      created_at: "2026-06-21T00:01:00Z",
      schema_version: 1,
    },
  ],
  transcript: [
    {
      event_id: "msg_1",
      sequence: 1,
      event_type: "assistant_message",
      actor_kind: "agent",
      event_time: "2026-06-21T00:00:30Z",
      redaction_class: "local_redacted",
      text_preview: "I ran the focused tests.",
    },
  ],
  commands: [
    {
      id: "cmd_1",
      evidence_id: "wevdc_fail",
      command: "cargo test -p ctx-http",
      argv: ["cargo", "test", "-p", "ctx-http"],
      cwd: "[redacted:workspace_root]",
      exit_code: 101,
      status: "observed_fail",
      freshness: "stale",
      stdout_preview: "1 failing test",
      stderr_preview: null,
      output_truncated: false,
      started_at: "2026-06-21T00:00:00Z",
      finished_at: "2026-06-21T00:01:00Z",
      output_ref: { log: "redacted failure output" },
    },
  ],
  artifacts: [
    {
      id: "artifact_1",
      kind: "screenshot",
      label: "Unsafe screenshot link",
      url: "javascript:alert(1)",
      path: "[redacted:artifact_path]",
      ref: { mime: "image/png" },
      created_at: "2026-06-21T00:05:00Z",
    },
  ],
  timeline_items: [],
  duplicate_strong_links: [],
  raw_transcript_available: false,
  raw_transcript_included: false,
});

describe("WorkInspectorView", () => {
  it("renders the dashboard shell and overview metrics", () => {
    const onRefresh = vi.fn();
    render(<WorkInspectorView report={baseReport()} onRefresh={onRefresh} />);

    expect(screen.getByRole("heading", { name: "Stabilize Work inspector route" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Raw redacted JSON" })).toBeInTheDocument();
    expect(screen.getByLabelText("Work trust")).toHaveTextContent("failed");
    expect(screen.getByLabelText("Evidence summary")).toHaveTextContent("Commands");
    expect(screen.getByText("Raw transcripts are not available in this inspector response.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("switches between transcript, commands, and evidence tabs deterministically", () => {
    render(<WorkInspectorView report={baseReport()} />);

    fireEvent.click(screen.getByRole("tab", { name: "Transcript" }));
    expect(screen.getByRole("tab", { name: "Transcript" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("I ran the focused tests.");
    expect(screen.queryByText("Observed cargo test exited 101")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Commands" }));
    expect(screen.getByRole("tabpanel")).toHaveTextContent("cargo test -p ctx-http");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("exit 101");

    fireEvent.click(screen.getByRole("tab", { name: "Evidence" }));
    expect(screen.getByRole("tabpanel")).toHaveTextContent("Observed cargo test exited 101");
    expect(screen.getAllByText("worktree").length).toBeGreaterThan(0);
  });

  it("renders unsafe URLs as text in tab content", () => {
    render(<WorkInspectorView report={baseReport()} />);

    fireEvent.click(screen.getByRole("tab", { name: "Changes" }));
    expect(screen.queryByRole("link", { name: "Unsafe stored PR · draft" })).not.toBeInTheDocument();
    expect(screen.getByText("Unsafe stored PR · draft")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Artifacts" }));
    const artifacts = screen.getByRole("tabpanel");
    expect(within(artifacts).queryByRole("link", { name: "javascript:alert(1)" })).not.toBeInTheDocument();
    expect(within(artifacts).getByText("javascript:alert(1)")).toBeInTheDocument();
  });

  it("keeps raw redacted JSON collapsed and renders only safe_json when expanded", () => {
    const report = baseReport();
    const unsafePayload = { secret: "/home/daddy/private-token" };
    (report.raw_redacted_json as typeof report.raw_redacted_json & { unsafe_json?: unknown }).unsafe_json = unsafePayload;

    render(<WorkInspectorView report={report} />);
    fireEvent.click(screen.getByRole("tab", { name: "Raw redacted JSON" }));

    expect(screen.getByRole("button", { name: "Expand JSON" })).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("safe-visible-marker")).not.toBeInTheDocument();
    expect(screen.queryByText("/home/daddy/private-token")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Expand JSON" }));
    expect(screen.getByRole("button", { name: "Collapse JSON" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/safe-visible-marker/)).toBeInTheDocument();
    expect(screen.queryByText("/home/daddy/private-token")).not.toBeInTheDocument();
  });
});
