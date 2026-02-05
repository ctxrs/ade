import { useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import LauncherBrand from "../components/LauncherBrand";

type WizardOption = {
  id: string;
  title: string;
  desc: string;
};

type WizardStep = {
  key: string;
  title: string;
  note: string;
  options?: WizardOption[];
  body?: ReactNode;
};

export default function WorkspaceSetupPage() {
  const steps = useMemo<WizardStep[]>(() => ([
    {
      key: "location",
      title: "Location",
      note: "Where will this workspace run?",
      options: [
        { id: "local", title: "Local", desc: "Run on this machine." },
        { id: "remote", title: "Remote host", desc: "Connect to a dev box or server." },
      ],
    },
    {
      key: "source",
      title: "Source",
      note: "How should we create the workspace?",
      options: [
        { id: "clone", title: "Clone repo", desc: "Git URL + branch." },
        { id: "import", title: "Import folder", desc: "Copy into a managed volume." },
        { id: "new", title: "New empty", desc: "Initialize git by default." },
      ],
    },
    {
      key: "network",
      title: "Network policy",
      note: "Choose how outbound access works.",
      options: [
        { id: "providers", title: "Providers only", desc: "Only validated provider traffic." },
        { id: "allowlist", title: "Allowlist", desc: "Specific hosts you approve." },
        { id: "full", title: "Full access", desc: "Unrestricted outbound." },
      ],
      body: (
        <div className="wizard-input">
          <label>
            Allowlist (one per line)
            <textarea placeholder="registry.npmjs.org\napi.github.com" />
          </label>
        </div>
      ),
    },
    {
      key: "verify",
      title: "Verification command",
      note: "Single command, editable later.",
      body: (
        <div className="wizard-input">
          <input placeholder="pnpm test" />
        </div>
      ),
    },
    {
      key: "setup",
      title: "Setup command",
      note: "One-time setup for the container.",
      body: (
        <div className="wizard-input">
          <input placeholder="pnpm install" />
        </div>
      ),
    },
    {
      key: "attachments",
      title: "Attachments",
      note: "Read-only repos and docs for context.",
      options: [
        { id: "add", title: "Add attachment", desc: "Git repo, docs folder, or URL." },
        { id: "skip", title: "Skip for now", desc: "You can add these later." },
      ],
    },
    {
      key: "merge-queue",
      title: "Merge queue",
      note: "How should changes apply?",
      options: [
        { id: "manual", title: "Manual apply", desc: "You approve merges." },
        { id: "auto", title: "Auto-apply after verify", desc: "Apply when checks pass." },
      ],
      body: (
        <div className="wizard-input">
          <label>
            Base branch
            <input placeholder="main" />
          </label>
        </div>
      ),
    },
    {
      key: "session-titling",
      title: "Session titling",
      note: "Set once, reused later.",
      options: [
        { id: "local-model", title: "Local model", desc: "One-click download." },
        { id: "cloud-api", title: "Cloud API", desc: "Use your provider key." },
      ],
    },
    {
      key: "confirm",
      title: "Confirm and create",
      note: "Review your choices before provisioning.",
      body: (
        <div className="wizard-step-summary">
          <div>We will spin up a container workspace and keep the configuration editable.</div>
        </div>
      ),
    },
  ]), []);

  const [stepIndex, setStepIndex] = useState(0);
  const [selections, setSelections] = useState<Record<string, string>>({});
  const step = steps[stepIndex];
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;
  const requiresSelection = Boolean(step.options?.length);
  const hasSelection = Boolean(selections[step.key]);
  const canAdvance = !requiresSelection || hasSelection;

  const onSelect = (stepKey: string, optionId: string) => {
    setSelections((prev) => ({ ...prev, [stepKey]: optionId }));
  };

  return (
    <div className="launcher-shell launcher-shell--crt">
      <LauncherBrand fullScreen>
        <div className="wizard-panel">
          <div className="wizard-steps">
            <div className="wizard-step">
              <div className="wizard-step-header">
                <div className="wizard-step-title">{step.title}</div>
                <div className="wizard-step-note">{step.note}</div>
              </div>
              <div className="wizard-step-body">
                {step.options && (
                  <div className="wizard-option-grid">
                    {step.options.map((option) => {
                      const selected = selections[step.key] === option.id;
                      return (
                        <button
                          key={option.id}
                          type="button"
                          className={`wizard-option${selected ? " is-selected" : ""}`}
                          onClick={() => onSelect(step.key, option.id)}
                        >
                          <div className="wizard-option-title">{option.title}</div>
                          <div className="wizard-option-desc">{option.desc}</div>
                        </button>
                      );
                    })}
                  </div>
                )}
                {step.body}
              </div>
            </div>
          </div>

          <div className="wizard-pagination" role="tablist" aria-label="Setup steps">
            {steps.map((item, idx) => (
              <button
                key={item.key}
                type="button"
                className={`wizard-dot${idx === stepIndex ? " is-active" : ""}`}
                aria-label={`Go to step ${idx + 1}`}
                aria-current={idx === stepIndex ? "true" : undefined}
                onClick={() => setStepIndex(idx)}
              />
            ))}
          </div>

          <div className="wizard-actions">
            {isFirst ? (
              <Link to="/" className="wizard-secondary">Back</Link>
            ) : (
              <button type="button" className="wizard-secondary" onClick={() => setStepIndex((idx) => Math.max(0, idx - 1))}>
                Back
              </button>
            )}
            {isLast ? (
              <button type="button" className="wizard-primary" disabled={!canAdvance}>Create workspace</button>
            ) : (
              <button
                type="button"
                className="wizard-primary"
                disabled={!canAdvance}
                onClick={() => setStepIndex((idx) => Math.min(steps.length - 1, idx + 1))}
              >
                Next
              </button>
            )}
          </div>
        </div>
      </LauncherBrand>
    </div>
  );
}
