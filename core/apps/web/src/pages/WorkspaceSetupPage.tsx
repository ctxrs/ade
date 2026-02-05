import { Link } from "react-router-dom";
import LauncherBrand from "../components/LauncherBrand";

export default function WorkspaceSetupPage() {
  return (
    <div className="wizard-shell">
      <div className="wizard-panel">
        <div className="launcher-header">
          <LauncherBrand />
          <div>
            <div className="wizard-title">New workspace</div>
            <div className="wizard-subtitle">Container-first setup with minimal questions.</div>
          </div>
        </div>

        <div className="wizard-progress">Setup overview · Edit anytime from workspace settings.</div>

        <div className="wizard-steps">
          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Location</div>
              <div className="wizard-step-note">Where will this workspace run?</div>
            </div>
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
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Source</div>
              <div className="wizard-step-note">How should we create the workspace?</div>
            </div>
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
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Network policy</div>
              <div className="wizard-step-note">Choose how outbound access works.</div>
            </div>
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
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Verification command</div>
              <div className="wizard-step-note">Single command, editable later.</div>
            </div>
            <div className="wizard-input">
              <input placeholder="pnpm test" />
            </div>
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Setup command</div>
              <div className="wizard-step-note">One-time setup for the container.</div>
            </div>
            <div className="wizard-input">
              <input placeholder="pnpm install" />
            </div>
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Attachments</div>
              <div className="wizard-step-note">Read-only repos and docs for context.</div>
            </div>
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
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Merge queue</div>
              <div className="wizard-step-note">How should changes apply?</div>
            </div>
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
          </div>

          <div className="wizard-step">
            <div className="wizard-step-header">
              <div className="wizard-step-title">Session titling</div>
              <div className="wizard-step-note">Set once, reused later.</div>
            </div>
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
          </div>
        </div>

        <div className="wizard-actions">
          <Link to="/" className="wizard-secondary">Back</Link>
          <button type="button" className="wizard-primary">Create workspace</button>
        </div>
      </div>
    </div>
  );
}
