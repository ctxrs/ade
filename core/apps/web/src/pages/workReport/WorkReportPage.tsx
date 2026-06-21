import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import type { WorkspaceWorkInspector } from "@ctx/types";
import { getWorkspaceWorkInspector } from "../../api/clientWorkspaces";
import { WorkInspectorView } from "./WorkReportView";

export function useWorkInspectorReport(workspaceId?: string, workId?: string) {
  const [report, setReport] = useState<WorkspaceWorkInspector | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadReport = useCallback(async () => {
    if (!workspaceId || !workId) {
      setError("Missing Work route parameters.");
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const next = await getWorkspaceWorkInspector(workspaceId, workId);
      setReport(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load Work inspector.");
    } finally {
      setLoading(false);
    }
  }, [workspaceId, workId]);

  useEffect(() => {
    void loadReport();
  }, [loadReport]);

  return { report, loading, error, reload: loadReport };
}

export function WorkInspectorPage() {
  const { id, workId } = useParams<{ id: string; workId: string }>();
  const { report, loading, error, reload } = useWorkInspectorReport(id, workId);

  if (!id || !workId) {
    return (
      <main className="work-report-page" role="alert">
        Missing Work route parameters.
      </main>
    );
  }
  if (loading && !report) {
    return (
      <main className="work-report-page work-report-loading" aria-busy="true" role="status">
        Loading Work inspector...
      </main>
    );
  }
  if (error && !report) {
    return (
      <main className="work-report-page work-report-error" role="alert">
        <h1>Work inspector unavailable</h1>
        <p>{error}</p>
        <button className="work-report-refresh" type="button" onClick={reload}>
          Retry
        </button>
        <Link to={`/workspaces/${encodeURIComponent(id)}`}>Back to workspace</Link>
      </main>
    );
  }
  if (!report) {
    return (
      <main className="work-report-page" role="status">
        No Work inspector is available.
      </main>
    );
  }
  return <WorkInspectorView report={report} onRefresh={reload} />;
}

export { WorkInspectorPage as WorkReportPage };
export default WorkInspectorPage;
