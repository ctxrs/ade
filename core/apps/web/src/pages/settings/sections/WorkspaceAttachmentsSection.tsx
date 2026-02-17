import { Card, Row } from "../../SettingsPage.components";
import { formatAge, guessAttachmentName, truncateText } from "../../SettingsPage.utils";
import { formatAttachmentStatus } from "../../SettingsPage.helpers";
import { idToString } from "../../../api/client";
import { useWorkspaceAttachmentsController } from "../hooks/useWorkspaceAttachmentsController";

type WorkspaceAttachmentsSectionProps = {
  workspaceId: string | null;
  active: boolean;
};

export function WorkspaceAttachmentsSection({ workspaceId, active }: WorkspaceAttachmentsSectionProps) {
  const {
    workspaces,
    attachmentSource,
    setAttachmentSource,
    attachmentName,
    setAttachmentName,
    attachmentRevision,
    setAttachmentRevision,
    handleAddAttachment,
    attachmentBusy,
    syncWorkspaceAttachmentsNow,
    attachmentSyncBusy,
    attachmentsLoading,
    attachments,
    attachmentDeleteBusy,
    handleRemoveAttachment,
    docsAttachmentSource,
    setDocsAttachmentSource,
    docsAttachmentName,
    setDocsAttachmentName,
    handleAddDocsAttachment,
    docsAttachmentBusy,
    attachmentsError,
  } = useWorkspaceAttachmentsController({
    workspaceId,
    enabled: active,
  });

  const selectedWorkspace = workspaces.find((ws) => idToString((ws as { id?: string | null }).id) === workspaceId) ?? null;
  const configPath = selectedWorkspace ? `${selectedWorkspace.root_path}/.ctx/attachments.toml` : ".ctx/attachments.toml";
  const canAdd = Boolean(workspaceId && attachmentSource.trim());
  const canAddDocs = Boolean(workspaceId && docsAttachmentSource.trim());

  return (
    <>
      <Card title="Workspace Attachments">
        <Row
          title="Config file"
          description="Repo-scoped attachments configuration."
          control={<span className="settings-pill wb-mono">{configPath}</span>}
        />
        <Row
          title="Mount paths"
          description="Reference repos are mounted inside each track."
          control={<span className="settings-pill wb-mono">.ctx/attachments/refs/&lt;name&gt;</span>}
        />
      </Card>

      <Card title="Reference Repos">
        <div className="settings-card-block">
          <div className="settings-attachments-form">
            <div className="settings-attachments-field">
              <label className="settings-attachments-label" htmlFor="attachments-source">
                Repository URL
              </label>
              <input
                id="attachments-source"
                className="settings-control"
                value={attachmentSource}
                onChange={(e) => setAttachmentSource(e.target.value)}
                placeholder="git@github.com:org/repo.git"
              />
            </div>
            <div className="settings-attachments-field">
              <label className="settings-attachments-label" htmlFor="attachments-name">
                Display name
              </label>
              <input
                id="attachments-name"
                className="settings-control"
                value={attachmentName}
                onChange={(e) => setAttachmentName(e.target.value)}
                placeholder={guessAttachmentName(attachmentSource) || "reference"}
              />
            </div>
            <div className="settings-attachments-field">
              <label className="settings-attachments-label" htmlFor="attachments-revision">
                Revision (optional)
              </label>
              <input
                id="attachments-revision"
                className="settings-control"
                value={attachmentRevision}
                onChange={(e) => setAttachmentRevision(e.target.value)}
                placeholder="main or tag"
              />
            </div>
          </div>
          <div className="settings-attachments-actions">
            <button
              type="button"
              className="settings-btn"
              onClick={() => {
                void handleAddAttachment();
              }}
              disabled={!canAdd || attachmentBusy || !workspaceId}
            >
              {attachmentBusy ? "Adding…" : "Add repo"}
            </button>
            <button
              type="button"
              className="settings-btn settings-btn-secondary"
              onClick={() => {
                void syncWorkspaceAttachmentsNow();
              }}
              disabled={!workspaceId || attachmentSyncBusy}
            >
              {attachmentSyncBusy ? "Syncing…" : "Sync now"}
            </button>
          </div>
          <div className="settings-attachments-hint">
            Use SSH URLs for private repos. The daemon must have access to your SSH keys.
          </div>
        </div>
        <div className="settings-card-block">
          {attachmentsLoading ? <div className="settings-empty-compact">Loading attachments…</div> : null}
          {!attachmentsLoading && attachments.length === 0 ? (
            <div className="settings-empty-compact">No workspace attachments yet.</div>
          ) : null}
          {!attachmentsLoading && attachments.length > 0 ? (
            <div className="settings-table settings-table-attachments">
              <div className="settings-table-head">
                <div>Attachment</div>
                <div>Source</div>
                <div>Mount</div>
                <div>Status</div>
                <div>Updated</div>
                <div />
              </div>
              {attachments.map((attachment) => {
                const updatedAt = attachment.last_sync_at ?? attachment.updated_at;
                const updatedMs = updatedAt ? Date.parse(updatedAt) : Number.NaN;
                const updatedLabel = Number.isFinite(updatedMs)
                  ? `${formatAge(Date.now() - updatedMs)} ago`
                  : "—";
                const statusLabel = formatAttachmentStatus(attachment.status);
                const statusTitle = attachment.error_message ? `${statusLabel}: ${attachment.error_message}` : statusLabel;
                const statusClass =
                  attachment.status === "error"
                    ? "settings-table-sub settings-table-sub-error"
                    : "settings-table-sub";
                const deleteBusy = attachmentDeleteBusy[idToString(attachment.id)] ?? false;
                return (
                  <div key={idToString(attachment.id)} className="settings-table-row">
                    <div>
                      <div className="settings-table-title">{attachment.name}</div>
                      <div className="settings-table-sub">
                        {attachment.kind === "reference_repo" ? "Reference repo" : "Docs mirror"}
                      </div>
                    </div>
                    <div className="settings-table-mono" title={attachment.source}>
                      {truncateText(attachment.source, 64)}
                    </div>
                    <div className="settings-table-mono" title={attachment.mount_relpath}>
                      {truncateText(attachment.mount_relpath, 32)}
                    </div>
                    <div className={statusClass} title={statusTitle}>
                      {statusLabel}
                    </div>
                    <div className="settings-table-sub">{updatedLabel}</div>
                    <div className="settings-row-right">
                      <button
                        type="button"
                        className="settings-btn settings-btn-secondary settings-btn-compact"
                        onClick={() => {
                          void handleRemoveAttachment(attachment);
                        }}
                        disabled={deleteBusy}
                      >
                        {deleteBusy ? "Removing…" : "Remove"}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : null}
        </div>
      </Card>

      <Card title="Docs">
        <div className="settings-card-block">
          <div className="settings-attachments-form">
            <div className="settings-attachments-field">
              <label className="settings-attachments-label" htmlFor="attachments-docs-source">
                Docs URL
              </label>
              <input
                id="attachments-docs-source"
                className="settings-control"
                value={docsAttachmentSource}
                onChange={(e) => setDocsAttachmentSource(e.target.value)}
                placeholder="https://docs.example.com/"
              />
            </div>
            <div className="settings-attachments-field">
              <label className="settings-attachments-label" htmlFor="attachments-docs-name">
                Display name
              </label>
              <input
                id="attachments-docs-name"
                className="settings-control"
                value={docsAttachmentName}
                onChange={(e) => setDocsAttachmentName(e.target.value)}
                placeholder={guessAttachmentName(docsAttachmentSource) || "docs"}
              />
            </div>
          </div>
          <div className="settings-attachments-actions">
            <button
              type="button"
              className="settings-btn"
              onClick={() => {
                void handleAddDocsAttachment();
              }}
              disabled={!canAddDocs || docsAttachmentBusy || !workspaceId}
            >
              {docsAttachmentBusy ? "Adding…" : "Add docs"}
            </button>
          </div>
          <div className="settings-attachments-hint">
            Paste any docs page URL. The daemon will infer the crawl entrypoint and mirror it.
          </div>
        </div>
      </Card>
      {attachmentsError ? <div className="settings-banner settings-banner-error">{attachmentsError}</div> : null}
    </>
  );
}
