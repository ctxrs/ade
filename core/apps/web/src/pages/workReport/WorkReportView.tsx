import { useMemo, useState } from "react";
import type {
  JsonValue,
  WorkspaceWorkEvidence,
  WorkspaceWorkInspector,
  WorkspaceWorkInspectorArtifact,
  WorkspaceWorkInspectorCommand,
  WorkspaceWorkInspectorTranscriptItem,
  WorkspaceWorkReport,
  WorkspaceWorkTrustSummary,
} from "@ctx/types";
import { ExternalLink } from "../../components/ExternalLink";

type WorkInspectorTab =
  | "overview"
  | "transcript"
  | "commands"
  | "evidence"
  | "timeline"
  | "changes"
  | "artifacts"
  | "context"
  | "raw";

const tabs: { id: WorkInspectorTab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "transcript", label: "Transcript" },
  { id: "commands", label: "Commands" },
  { id: "evidence", label: "Evidence" },
  { id: "timeline", label: "Timeline" },
  { id: "changes", label: "Changes" },
  { id: "artifacts", label: "Artifacts" },
  { id: "context", label: "Context" },
  { id: "raw", label: "Raw redacted JSON" },
];

const label = (value: string | null | undefined) =>
  String(value ?? "unknown").replaceAll("_", " ");

const shortSha = (value: string | null | undefined) => {
  if (!value) return "unknown";
  return value.length > 12 ? value.slice(0, 12) : value;
};

const trustClass = (verdict: string) => `work-report-trust work-report-trust-${verdict}`;

const evidenceClass = (item: WorkspaceWorkEvidence) =>
  `work-report-evidence-row work-report-evidence-${item.status} work-report-freshness-${item.freshness}`;

const compactJson = (value: JsonValue | null | undefined, limit = 220) => {
  if (value == null) return null;
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > limit ? `${text.slice(0, limit)}...` : text;
};

const prettyJson = (value: JsonValue) => JSON.stringify(value, null, 2);

const asRecord = (value: JsonValue | null | undefined): Record<string, JsonValue> | null => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value;
};

const pickString = (record: Record<string, JsonValue> | null, keys: string[]) => {
  if (!record) return null;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value;
    if (typeof value === "number") return String(value);
  }
  return null;
};

const safeExternalUrl = (value: string | null | undefined) => {
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? value : null;
  } catch {
    return null;
  }
};

const renderUrl = (value: string | null | undefined, labelText?: string | null) => {
  if (!value) return null;
  const href = safeExternalUrl(value);
  return href ? <ExternalLink href={href}>{labelText || value}</ExternalLink> : <span>{labelText || value}</span>;
};

const pullRequestLabel = (value: JsonValue, index: number, fallback?: string | null) => {
  const outer = asRecord(value);
  const nested = asRecord(outer?.pull_request) ?? outer;
  const title = pickString(nested, ["title", "name"]);
  const url = safeExternalUrl(pickString(nested, ["url", "html_url"]));
  const state = pickString(nested, ["state"]);
  const number = pickString(nested, ["number", "pr_number"]);
  const labelParts = [title || fallback || (number ? `PR #${number}` : `PR ${index + 1}`), state ? label(state) : null].filter(Boolean);
  return { label: labelParts.join(" · "), url };
};

const rawTranscriptStatus = (report: WorkspaceWorkInspector) => {
  if (report.raw_transcript_included) {
    return "Raw transcript detail is included in this response; review redaction before sharing.";
  }
  if (report.raw_transcript_available) {
    return "Raw transcripts are available locally but not included by default.";
  }
  return "Raw transcripts are not available in this inspector response.";
};

const reportToInspector = (report: WorkspaceWorkReport): WorkspaceWorkInspector => ({
  ...report,
  overview: {
    title: report.work.title,
    objective: report.work.objective,
    lifecycle: report.work.lifecycle,
    primary_branch: report.work.primary_branch,
    base_commit: report.work.base_commit,
    head_commit: report.work.head_commit,
    created_at: report.work.created_at,
    updated_at: report.work.updated_at,
  },
  transcript: report.timeline.map((event): WorkspaceWorkInspectorTranscriptItem => ({
    event_id: event.event_id,
    sequence: event.sequence,
    event_type: event.event_type,
    actor_kind: event.actor_kind,
    event_time: event.event_time,
    redaction_class: event.redaction_class,
    text_preview: event.redacted_text,
  })),
  commands: report.evidence
    .filter((item) => item.command || item.argv.length > 0)
    .map((item): WorkspaceWorkInspectorCommand => ({
      id: item.evidence_id,
      evidence_id: item.evidence_id,
      command: item.command,
      argv: item.argv,
      cwd: item.cwd,
      exit_code: item.exit_code,
      status: item.status,
      freshness: item.freshness,
      stdout_preview: null,
      stderr_preview: null,
      output_truncated: false,
      started_at: item.started_at,
      finished_at: item.finished_at,
      output_ref: item.output_ref,
    })),
  artifacts: report.evidence
    .filter((item) => item.artifact_ref)
    .map((item): WorkspaceWorkInspectorArtifact => ({
      id: item.evidence_id,
      kind: item.kind,
      label: item.claim || item.command || item.evidence_id,
      ref: item.artifact_ref,
      created_at: item.created_at,
    })),
  artifact_summary: {
    total: report.evidence.filter((item) => item.artifact_ref).length,
    refs: [],
  },
  context: {
    value: {
      summaries: report.summaries,
      summary_claims: report.summary_claims,
      duplicate_strong_links: report.duplicate_strong_links,
    },
    redacted: true,
    redaction_notes: ["compatibility projection from v1 report"],
  },
  safe_json: {
    value: report as unknown as JsonValue,
    redacted: true,
    redaction_notes: ["compatibility projection from v1 report"],
  },
  raw_redacted_json: {
    value: report as unknown as JsonValue,
    redacted: true,
    redaction_notes: ["compatibility projection from v1 report"],
  },
  timeline_items: report.timeline.map((event) => ({
    sequence: event.sequence,
    event_time: event.event_time,
    kind: event.event_type,
    title: event.redacted_text || event.event_type,
    detail: event.source_kind,
    source_event_id: event.event_id,
  })),
});

function TrustStrip({ trust }: { trust: WorkspaceWorkTrustSummary }) {
  return (
    <section className={trustClass(trust.verdict)} aria-label="Work trust">
      <div>
        <span className="work-report-eyebrow">Trust</span>
        <strong>{label(trust.verdict)}</strong>
      </div>
      <p>{trust.reason}</p>
      <div className="work-report-next">{trust.recommended_next_action}</div>
    </section>
  );
}

function Empty({ children }: { children: string }) {
  return <p className="work-report-empty">{children}</p>;
}

function OverviewTab({ report }: { report: WorkspaceWorkInspector }) {
  const missingEvidence =
    report.evidence_summary.missing > 0 || report.trust.verdict === "missing_evidence";
  return (
    <>
      <TrustStrip trust={report.trust} />
      {missingEvidence ? (
        <section className="work-report-warning" aria-label="Missing evidence">
          <strong>Evidence is missing</strong>
          <p>{report.trust.recommended_next_action}</p>
        </section>
      ) : null}
      {report.duplicate_strong_links.length > 0 ? (
        <section className="work-report-warning" aria-label="Duplicate Work links">
          <strong>Merge-needed links</strong>
          {report.duplicate_strong_links.map((item) => (
            <p key={`${item.target_kind}:${item.target_id}`}>
              {label(item.target_kind)} {item.target_id} is linked to {item.work_ids.length} Work records.
            </p>
          ))}
        </section>
      ) : null}
      <section className="work-report-summary-grid" aria-label="Evidence summary">
        <Metric label="Evidence" value={report.evidence_summary.total} />
        <Metric label="Passing" value={report.evidence_summary.passing} />
        <Metric label="Failing" value={report.evidence_summary.failing} />
        <Metric label="Stale" value={report.evidence_summary.stale} />
        <Metric label="Missing" value={report.evidence_summary.missing} />
        <Metric label="Summaries" value={label(report.work.summary_freshness)} />
        <Metric label="Changes" value={report.change_summary.change_sets} />
        <Metric label="Commands" value={report.commands.length} />
      </section>
      <section className="work-report-panel" aria-label="Inspector status">
        <h2>Inspector status</h2>
        <p>{rawTranscriptStatus(report)}</p>
      </section>
    </>
  );
}

function Metric({ label: labelText, value }: { label: string; value: string | number }) {
  return (
    <div>
      <span className="work-report-eyebrow">{labelText}</span>
      <strong>{value}</strong>
    </div>
  );
}

function TranscriptTab({ items }: { items: WorkspaceWorkInspectorTranscriptItem[] }) {
  return (
    <section className="work-report-panel" aria-label="Transcript">
      <div className="work-report-panel-header">
        <h2>Transcript</h2>
        <span>{items.length ? `${items.length} entries` : "none recorded"}</span>
      </div>
      {items.length ? (
        <ol className="work-report-stack">
          {items.map((item, index) => (
            <li className="work-report-message" key={item.event_id || item.id || index}>
              <div className="work-report-meta">
                <span>{label(item.actor_kind)}</span>
                <span>{label(item.event_type)}</span>
                {item.event_time ? <time dateTime={item.event_time}>{new Date(item.event_time).toLocaleString()}</time> : null}
                {item.redaction_class ? <span>{label(item.redaction_class)}</span> : null}
              </div>
              <p>{item.text_preview || "No redacted text is available."}</p>
            </li>
          ))}
        </ol>
      ) : (
        <Empty>No transcript entries are available.</Empty>
      )}
    </section>
  );
}

function CommandsTab({ commands }: { commands: WorkspaceWorkInspectorCommand[] }) {
  return (
    <section className="work-report-panel" aria-label="Commands">
      <div className="work-report-panel-header">
        <h2>Commands</h2>
        <span>{commands.length ? `${commands.length} commands` : "none recorded"}</span>
      </div>
      {commands.length ? (
        <div className="work-report-evidence-list">
          {commands.map((command, index) => (
            <article className="work-report-command" key={command.id || index}>
              <strong>{command.command || command.argv.join(" ") || command.id}</strong>
              <div className="work-report-meta">
                {command.status ? <span>{label(command.status)}</span> : null}
                {command.freshness ? <span>{label(command.freshness)}</span> : null}
                {typeof command.exit_code === "number" ? <span>exit {command.exit_code}</span> : null}
                {command.cwd ? <span>{command.cwd}</span> : null}
                {command.output_truncated ? <span>output truncated</span> : null}
              </div>
              {command.stdout_preview ? <p className="work-report-ref">stdout: {command.stdout_preview}</p> : null}
              {command.stderr_preview ? <p className="work-report-ref">stderr: {command.stderr_preview}</p> : null}
              {command.output_ref ? <p className="work-report-ref">Output: {compactJson(command.output_ref)}</p> : null}
            </article>
          ))}
        </div>
      ) : (
        <Empty>No commands have been recorded.</Empty>
      )}
    </section>
  );
}

function EvidenceTab({ evidence }: { evidence: WorkspaceWorkEvidence[] }) {
  return (
    <section className="work-report-panel work-report-evidence" aria-label="Evidence">
      <div className="work-report-panel-header">
        <h2>Evidence</h2>
        <span>{evidence.length ? `${evidence.length} observed` : "none recorded"}</span>
      </div>
      {evidence.length ? (
        <div className="work-report-evidence-list">
          {evidence.map((item) => (
            <article className={evidenceClass(item)} key={item.evidence_id}>
              <div>
                <strong>{item.claim || item.command || item.evidence_id}</strong>
                <p>{item.command || item.argv.join(" ")}</p>
                <div className="work-report-evidence-detail">
                  <span>{label(item.source)}</span>
                  <span>{label(item.fidelity)}</span>
                  <span>{label(item.trust)}</span>
                  {item.head_sha ? <span>{shortSha(item.head_sha)}</span> : null}
                </div>
                {item.output_ref ? <p className="work-report-ref">Output: {compactJson(item.output_ref)}</p> : null}
                {item.artifact_ref ? <p className="work-report-ref">Artifact: {compactJson(item.artifact_ref)}</p> : null}
              </div>
              <div className="work-report-evidence-badges">
                <span>{label(item.kind)}</span>
                <span>{label(item.status)}</span>
                <span>{label(item.freshness)}</span>
              </div>
            </article>
          ))}
        </div>
      ) : (
        <Empty>No evidence has been recorded for this Work record.</Empty>
      )}
    </section>
  );
}

function TimelineTab({ report }: { report: WorkspaceWorkInspector }) {
  return (
    <section className="work-report-panel work-report-timeline" aria-label="Timeline">
      <div className="work-report-panel-header">
        <h2>Timeline</h2>
        <span>{report.timeline.length ? `${report.timeline.length} events` : "none recorded"}</span>
      </div>
      {report.timeline.length ? (
        <ol>
          {report.timeline.map((event) => (
            <li key={event.event_id}>
              <span>{label(event.event_type)}</span>
              <time dateTime={event.event_time}>{new Date(event.event_time).toLocaleString()}</time>
              {event.redacted_text ? <p>{event.redacted_text}</p> : null}
            </li>
          ))}
        </ol>
      ) : (
        <Empty>No timeline events are available.</Empty>
      )}
    </section>
  );
}

function ChangesTab({ report }: { report: WorkspaceWorkInspector }) {
  const pullRequests = [
    ...report.change_summary.pull_requests.map((value, index) => pullRequestLabel(value, index)),
    ...report.links
      .filter((link) => link.target_kind === "pull_request")
      .map((link, index) => pullRequestLabel(link.target_json ?? null, index, link.target_id)),
  ].filter(
    (item, index, items) =>
      items.findIndex((candidate) => candidate.label === item.label && candidate.url === item.url) === index,
  );
  const commits = report.change_summary.commits.length
    ? report.change_summary.commits
    : report.links
        .filter((link) => link.target_kind === "commit" && link.target_id)
        .map((link) => link.target_id as string);
  return (
    <section className="work-report-panel" aria-label="Changes">
      <div className="work-report-panel-header">
        <h2>Changes</h2>
        <span>{report.change_summary.change_sets} change sets</span>
      </div>
      <div className="work-report-linked-items">
        {pullRequests.map((pr, index) =>
          pr.url ? (
            <ExternalLink key={`${pr.url}:${index}`} href={pr.url}>
              {pr.label}
            </ExternalLink>
          ) : (
            <span key={`${pr.label}:${index}`}>{pr.label}</span>
          ),
        )}
        {commits.map((commit) => (
          <span key={commit}>commit {shortSha(commit)}</span>
        ))}
        {report.change_summary.contributions > 0 ? <span>{report.change_summary.contributions} contributions</span> : null}
      </div>
      {report.change_sets.length ? <pre className="work-report-json">{prettyJson(report.change_sets as JsonValue)}</pre> : null}
    </section>
  );
}

function ArtifactsTab({ artifacts }: { artifacts: WorkspaceWorkInspectorArtifact[] }) {
  return (
    <section className="work-report-panel" aria-label="Artifacts">
      <div className="work-report-panel-header">
        <h2>Artifacts</h2>
        <span>{artifacts.length ? `${artifacts.length} artifacts` : "none recorded"}</span>
      </div>
      {artifacts.length ? (
        <div className="work-report-evidence-list">
          {artifacts.map((artifact, index) => (
            <article className="work-report-command" key={artifact.id || index}>
              <strong>{artifact.label || artifact.path || artifact.url || artifact.id}</strong>
              <div className="work-report-meta">
                {artifact.kind ? <span>{label(artifact.kind)}</span> : null}
                {artifact.created_at ? <time dateTime={artifact.created_at}>{new Date(artifact.created_at).toLocaleString()}</time> : null}
              </div>
              {artifact.url ? <p>{renderUrl(artifact.url)}</p> : null}
              {artifact.path ? <p className="work-report-ref">{artifact.path}</p> : null}
              {artifact.ref ? <p className="work-report-ref">Ref: {compactJson(artifact.ref)}</p> : null}
            </article>
          ))}
        </div>
      ) : (
        <Empty>No artifacts have been recorded.</Empty>
      )}
    </section>
  );
}

function ContextTab({ report }: { report: WorkspaceWorkInspector }) {
  return (
    <section className="work-report-panel work-report-side" aria-label="Context">
      <h2>Context</h2>
      {report.summaries.length > 0 ? (
        report.summaries.map((summary) => (
          <article className="work-report-summary" key={summary.summary_id}>
            <div className="work-report-meta">
              <span>{label(summary.kind)}</span>
              <span>{label(summary.freshness)}</span>
            </div>
            <p>{summary.text}</p>
          </article>
        ))
      ) : (
        <Empty>No summary has been generated yet.</Empty>
      )}
      <pre className="work-report-json">{prettyJson(report.context.value)}</pre>
    </section>
  );
}

function RawJsonTab({ report }: { report: WorkspaceWorkInspector }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <section className="work-report-panel" aria-label="Raw redacted JSON">
      <div className="work-report-panel-header">
        <h2>Raw redacted JSON</h2>
        <span>safe_json only</span>
      </div>
      <button
        aria-expanded={expanded}
        className="work-report-refresh"
        type="button"
        onClick={() => setExpanded((value) => !value)}
      >
        {expanded ? "Collapse JSON" : "Expand JSON"}
      </button>
      {expanded ? <pre className="work-report-json">{prettyJson(report.raw_redacted_json.value)}</pre> : null}
    </section>
  );
}

export function WorkInspectorView({
  report,
  onRefresh,
}: {
  report: WorkspaceWorkInspector;
  onRefresh?: () => void;
}) {
  const [selectedTab, setSelectedTab] = useState<WorkInspectorTab>("overview");
  const title = report.work.title || "Untitled Work";
  const selected = useMemo(() => tabs.find((tab) => tab.id === selectedTab) ?? tabs[0], [selectedTab]);
  return (
    <main className="work-report-page">
      <header className="work-report-header">
        <div>
          <span className="work-report-eyebrow">Work Inspector</span>
          <h1>{title}</h1>
          <div className="work-report-meta">
            <span>{report.work.work_id}</span>
            <span>{label(report.work.lifecycle)}</span>
            <span>{report.work.primary_branch || "branch unknown"}</span>
            <span>{shortSha(report.work.head_commit)}</span>
          </div>
        </div>
        {onRefresh ? (
          <button className="work-report-refresh" type="button" onClick={onRefresh}>
            Refresh
          </button>
        ) : null}
      </header>

      <nav className="work-report-tabs" role="tablist" aria-label="Work Inspector sections">
        {tabs.map((tab) => (
          <button
            aria-controls={`work-report-panel-${tab.id}`}
            aria-selected={selected.id === tab.id}
            id={`work-report-tab-${tab.id}`}
            key={tab.id}
            role="tab"
            type="button"
            onClick={() => setSelectedTab(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </nav>

      <div
        aria-labelledby={`work-report-tab-${selected.id}`}
        className="work-report-tab-panel"
        id={`work-report-panel-${selected.id}`}
        role="tabpanel"
      >
        {selected.id === "overview" ? <OverviewTab report={report} /> : null}
        {selected.id === "transcript" ? <TranscriptTab items={report.transcript} /> : null}
        {selected.id === "commands" ? <CommandsTab commands={report.commands} /> : null}
        {selected.id === "evidence" ? <EvidenceTab evidence={report.evidence} /> : null}
        {selected.id === "timeline" ? <TimelineTab report={report} /> : null}
        {selected.id === "changes" ? <ChangesTab report={report} /> : null}
        {selected.id === "artifacts" ? <ArtifactsTab artifacts={report.artifacts} /> : null}
        {selected.id === "context" ? <ContextTab report={report} /> : null}
        {selected.id === "raw" ? <RawJsonTab report={report} /> : null}
      </div>
    </main>
  );
}

export function WorkReportView({
  report,
  onRefresh,
}: {
  report: WorkspaceWorkReport;
  onRefresh?: () => void;
}) {
  return <WorkInspectorView report={reportToInspector(report)} onRefresh={onRefresh} />;
}
