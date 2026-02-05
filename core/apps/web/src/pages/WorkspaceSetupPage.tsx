import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import LauncherBrand from "../components/LauncherBrand";

export default function WorkspaceSetupPage() {
  const steps = useMemo(() => ([
    {
      key: "location",
      title: "Location",
      note: "Where will this workspace run?",
      body: (
        <div className="wizard-option-grid">
          <div className="wizard-option">
            <div className="wizard-option-title">Local</div>
            <div className="wizard-option-desc">Run on this machine.</div>
          </div>
          <div className="wizard-option">
            <div className="wizard-option-title">Remote host</div>
            <div className="wizard-option-desc">Connect to a dev box or server.</div>
          </div>
        </div>
      ),
    },
    {
      key: "source",
      title: "Source",
      note: "How should we create the workspace?",
      body: (
        <div className="wizard-option-grid">
          <div className="wizard-option">
            <div className="wizard-option-title">Clone repo</div>
            <div className="wizard-option-desc">Git URL + branch.</div>
          </div>
          <div className="wizard-option">
            <div className="wizard-option-title">Import folder</div>
            <div className="wizard-option-desc">Copy into a managed volume.</div>
          </div>
          <div className="wizard-option">
            <div className="wizard-option-title">New empty</div>
            <div className="wizard-option-desc">Initialize git by default.</div>
          </div>
        </div>
      ),
    },
    {
      key: "network",
      title: "Network policy",
      note: "Choose how outbound access works.",
      body: (
        <>
          <div className="wizard-option-grid">
            <div className="wizard-option">
              <div className="wizard-option-title">Providers only</div>
              <div className="wizard-option-desc">Only validated provider traffic.</div>
            </div>
            <div className="wizard-option">
              <div className="wizard-option-title">Allowlist</div>
              <div className="wizard-option-desc">Specific hosts you approve.</div>
            </div>
            <div className="wizard-option">
              <div className="wizard-option-title">Full access</div>
              <div className="wizard-option-desc">Unrestricted outbound.</div>
            </div>
          </div>
          <div className="wizard-input">
            <label>
              Allowlist (one per line)
              <textarea placeholder="registry.npmjs.org\napi.github.com" />
            </label>
          </div>
        </>
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
      body: (
        <div className="wizard-option-grid">
          <div className="wizard-option">
            <div className="wizard-option-title">Add attachment</div>
            <div className="wizard-option-desc">Git repo, docs folder, or URL.</div>
          </div>
          <div className="wizard-option">
            <div className="wizard-option-title">Skip for now</div>
            <div className="wizard-option-desc">You can add these later.</div>
          </div>
        </div>
      ),
    },
    {
      key: "merge-queue",
      title: "Merge queue",
      note: "How should changes apply?",
      body: (
        <>
          <div className="wizard-option-grid">
            <div className="wizard-option">
              <div className="wizard-option-title">Manual apply</div>
              <div className="wizard-option-desc">You approve merges.</div>
            </div>
            <div className="wizard-option">
              <div className="wizard-option-title">Auto-apply after verify</div>
              <div className="wizard-option-desc">Apply when checks pass.</div>
            </div>
          </div>
          <div className="wizard-input">
            <label>
              Base branch
              <input placeholder="main" />
            </label>
          </div>
        </>
      ),
    },
    {
      key: "session-titling",
      title: "Session titling",
      note: "Set once, reused later.",
      body: (
        <div className="wizard-option-grid">
          <div className="wizard-option">
            <div className="wizard-option-title">Local model</div>
            <div className="wizard-option-desc">One-click download.</div>
          </div>
          <div className="wizard-option">
            <div className="wizard-option-title">Cloud API</div>
            <div className="wizard-option-desc">Use your provider key.</div>
          </div>
        </div>
      ),
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
  const step = steps[stepIndex];
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;

  return (
    <div className="launcher-shell launcher-shell--crt">
      <LauncherBrand fullScreen>
        <div className="wizard-panel">
          <div>
            <div className="wizard-title">New workspace</div>
            <div className="wizard-subtitle">Container-first setup with minimal questions.</div>
          </div>

          <div className="wizard-progress">Step {stepIndex + 1} of {steps.length} · {step.title}</div>

          <div className="wizard-steps">
            <div className="wizard-step">
              <div className="wizard-step-header">
                <div className="wizard-step-title">{step.title}</div>
                <div className="wizard-step-note">{step.note}</div>
              </div>
              <div className="wizard-step-body">{step.body}</div>
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
              <button type="button" className="wizard-primary">Create workspace</button>
            ) : (
              <button type="button" className="wizard-primary" onClick={() => setStepIndex((idx) => Math.min(steps.length - 1, idx + 1))}>
                Next
              </button>
            )}
          </div>
        </div>
      </LauncherBrand>
    </div>
  );
}
