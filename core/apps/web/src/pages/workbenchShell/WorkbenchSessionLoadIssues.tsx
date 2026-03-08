import React from "react";

type SessionLoadIssue = {
  key: "state" | "subagentInvocations";
  message: string;
};

export function WorkbenchSessionLoadIssues({
  issues,
  onRetry,
}: {
  issues: SessionLoadIssue[];
  onRetry?: () => void;
}) {
  if (issues.length === 0) return null;

  return (
    <div className="wb-banner wb-session-load-issues" role="alert" data-testid="workbench-session-load-issues">
      <div className="wb-session-load-issues-title">Some session details failed to load.</div>
      {issues.map((issue) => (
        <div key={issue.key}>{issue.message}</div>
      ))}
      {onRetry ? (
        <div>
          <button type="button" className="wb-session-load-issues-retry" onClick={onRetry}>
            Retry
          </button>
        </div>
      ) : null}
    </div>
  );
}
