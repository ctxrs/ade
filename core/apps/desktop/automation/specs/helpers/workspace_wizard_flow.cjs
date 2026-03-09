const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const {
  waitForTauri,
  selectorForTestId,
  waitForTestId,
  clickTestId,
  setInputTestId,
  getConnectionInfo,
} = require("./tauri.cjs");
const { daemonJson, daemonJsonOnce, safeDaemonJson } = require("./daemon.cjs");

const REMOTE_HOST_RAW = process.env.CTX_AUTOMATION_REMOTE_HOST || "";
const REMOTE_HOST = REMOTE_HOST_RAW.trim();
const REMOTE_USER_DEFAULT = (process.env.CTX_AUTOMATION_REMOTE_USER || "devboxadmin").trim() || "devboxadmin";
const REMOTE_PASSWORD = process.env.CTX_AUTOMATION_REMOTE_PASSWORD || "";
const REMOTE_WIZARD_HOST_INPUT = (() => {
  const raw = REMOTE_HOST_RAW.trim();
  if (!raw) return "";
  if (raw.includes("@")) return raw;
  if (REMOTE_HOST) return `${REMOTE_USER_DEFAULT}@${REMOTE_HOST}`;
  return raw;
})();
const SCENARIO_FILTER = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);
const scenarioEnabled = (name, tags = []) => {
  if (SCENARIO_FILTER.size === 0) return true;
  const keys = [name, ...tags].map((s) => String(s).trim().toLowerCase()).filter(Boolean);
  return keys.some((k) => SCENARIO_FILTER.has(k));
};
const parsePort = (raw, fallback) => {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  const p = Math.trunc(n);
  if (p < 1 || p > 65535) return fallback;
  return p;
};
const parsePositiveInt = (raw, fallback) => {
  const n = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(n) || n <= 0) return fallback;
  return n;
};
const REMOTE_PORT = parsePort(process.env.CTX_AUTOMATION_REMOTE_PORT || "44099", 44099);
const REMOTE_DATA_DIR_RAW = process.env.CTX_AUTOMATION_REMOTE_DATA_DIR || "";
const CONTAINER_LAUNCH_TIMEOUT_MS = parsePositiveInt(
  process.env.CTX_AUTOMATION_CONTAINER_LAUNCH_TIMEOUT_MS || "900000",
  900000,
);
const REMOTE_LAUNCH_TIMEOUT_MS = parsePositiveInt(
  process.env.CTX_AUTOMATION_REMOTE_LAUNCH_TIMEOUT_MS || "300000",
  300000,
);
const SSH_NO_START_REMOTE = !["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "1").trim().toLowerCase(),
);
const RETRIABLE_WEBDRIVER_ERROR_PATTERNS = [
  "WebDriverError: Request failed with error code EADDRNOTAVAIL",
  "WebDriverError: Request failed with error code ECONNREFUSED",
  "WebDriverError: The operation was aborted due to timeout",
  "Download failed. Check connectivity and retry.",
  "Websocket connection lost",
  "socket hang up",
  "Error: Timeout",
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const shouldRetryWebdriverTransportError = (error) => {
  const text = String(error || "");
  return RETRIABLE_WEBDRIVER_ERROR_PATTERNS.some((pattern) => text.includes(pattern));
};

const browserExecuteWithRetry = async (fn, args = [], attempts = 4) => {
  let lastError = null;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await browser.execute(fn, ...args);
    } catch (error) {
      lastError = error;
      if (attempt >= attempts || !shouldRetryWebdriverTransportError(error)) {
        throw error;
      }
      await browser.pause(150 * attempt);
    }
  }
  throw lastError || new Error("browser.execute failed");
};

const run = (cmd, args, opts = {}) => {
  const res = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (res.status !== 0) {
    const stderr = String(res.stderr || "").trim();
    const stdout = String(res.stdout || "").trim();
    const msg = [
      `command failed: ${cmd} ${args.join(" ")}`,
      stderr ? `stderr: ${stderr}` : null,
      stdout ? `stdout: ${stdout}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    throw new Error(msg);
  }
  return String(res.stdout || "");
};

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

const initGitRepo = (dir, name) => {
  fs.mkdirSync(dir, { recursive: true });
  run("git", ["init", "--", dir]);
  run("git", ["-C", dir, "config", "user.email", "ctx-e2e@example.com"]);
  run("git", ["-C", dir, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(dir, "README.md"), `# ${name}\n`, "utf8");
  run("git", ["-C", dir, "add", "README.md"]);
  run("git", ["-C", dir, "commit", "-m", "init"]);
};

const waitForSelector = async (selector, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => await browser.execute((s) => Boolean(document.querySelector(s)), selector),
    { timeout: timeoutMs, timeoutMsg: `element not found: ${selector}` },
  );
};

const clickSelector = async (selector) => {
  await waitForSelector(selector);
  const ok = await browser.execute((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    (el).click();
    return true;
  }, selector);
  if (!ok) throw new Error(`failed to click selector: ${selector}`);
};

const setTextareaSelector = async (selector, value) => {
  await waitForSelector(selector);
  const ok = await browser.execute((s, v) => {
    const el = document.querySelector(s);
    if (!el) return false;
    if (!(el instanceof HTMLTextAreaElement)) return false;
    const desc = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    const setter = desc && desc.set;
    if (!setter) return false;
    setter.call(el, v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }, selector, String(value));
  if (!ok) throw new Error(`failed to set textarea for: ${selector}`);
};

const setCheckedTestId = async (id, checked) => {
  await waitForTestId(id);
  const sel = selectorForTestId(id);
  const ok = await browser.execute((s, want) => {
    const el = document.querySelector(s);
    if (!el) return false;
    if (!(el instanceof HTMLInputElement) || el.type !== "checkbox") return false;
    if (Boolean(el.checked) !== Boolean(want)) el.click();
    return true;
  }, sel, Boolean(checked));
  if (!ok) throw new Error(`failed to set checkbox for: ${sel}`);
};

const clickBack = async () => {
  const ok = await browser.execute(() => {
    const btn = document.querySelector('[data-testid="wizard-back"]')
      || document.querySelector('[data-testid="wizard-back-link"]');
    if (!btn) return false;
    (btn).click();
    return true;
  });
  return Boolean(ok);
};

const currentStepKey = async () => {
  return await browser.execute(() => {
    const el = document.querySelector('[data-testid="workspace-setup"]');
    return el ? el.getAttribute("data-step-key") : null;
  });
};

const waitForStep = async (key, timeoutMs = 30000) => {
  let last = null;
  await browser.waitUntil(async () => {
    last = await currentStepKey();
    return last === key;
  }, {
    timeout: timeoutMs,
    timeoutMsg: `expected step '${key}', got '${last || "unknown"}'`,
  });
};

const startWizardStepTrace = async () => {
  await browser.execute(() => {
    const read = () => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? String(root.getAttribute("data-step-key") || "") : "";
      return {
        step,
        pathname: String(window.location.pathname || ""),
      };
    };
    const trace = [read()];
    const record = () => {
      const next = read();
      const prev = trace[trace.length - 1];
      if (!prev || prev.step !== next.step || prev.pathname !== next.pathname) {
        trace.push(next);
      }
    };
    if (window.__ctxWizardTraceStop) {
      try { window.__ctxWizardTraceStop(); } catch { /* ignore */ }
    }
    const intervalId = window.setInterval(record, 50);
    window.__ctxWizardTrace = trace;
    window.__ctxWizardTraceStop = () => {
      window.clearInterval(intervalId);
    };
  });
};

const readWizardStepTrace = async () => {
  return await browser.execute(() => {
    const raw = Array.isArray(window.__ctxWizardTrace) ? window.__ctxWizardTrace : [];
    return raw.map((entry) => ({
      step: String(entry?.step || ""),
      pathname: String(entry?.pathname || ""),
    }));
  });
};

const stopWizardStepTrace = async () => {
  await browser.execute(() => {
    if (window.__ctxWizardTraceStop) {
      try { window.__ctxWizardTraceStop(); } catch { /* ignore */ }
    }
  });
};

const assertNoLocationRegression = (trace) => {
  let leftLocation = false;
  for (const entry of Array.isArray(trace) ? trace : []) {
    const step = String(entry?.step || "");
    if (step && step !== "location") {
      leftLocation = true;
      continue;
    }
    if (leftLocation && step === "location") {
      throw new Error(`wizard regressed to location after advancing: ${JSON.stringify(trace)}`);
    }
  }
};

const waitForSourceExitOrWorkspaceRoute = async (timeoutMs = 30000) => {
  const started = Date.now();
  let lastStep = null;
  let lastPath = "";
  while (Date.now() - started < timeoutMs) {
    try {
      const state = await browser.execute(() => {
        const root = document.querySelector('[data-testid="workspace-setup"]');
        const step = root ? root.getAttribute("data-step-key") : null;
        return {
          step,
          pathname: window.location.pathname,
        };
      });
      lastStep = state?.step || null;
      lastPath = String(state?.pathname || "");
      if (lastPath.startsWith("/workspaces/")) {
        const id = lastPath.split("/")[2] || "";
        if (!id) throw new Error("workspace id missing from route");
        return { kind: "workspace", workspaceId: id };
      }
      if (lastStep && lastStep !== "source") {
        return { kind: "step", step: lastStep };
      }
    } catch {
      // Best effort during route transitions where the webview can be mid-navigation.
    }
    await browser.pause(100);
  }
  throw new Error(
    `expected source transition to next step or workspace route; last_step='${lastStep || "unknown"}' last_path='${lastPath || "<none>"}'`,
  );
};

const clickOption = async (stepKey, optionId) => {
  const id = `wizard-option-${stepKey}-${optionId}`;
  const sel = selectorForTestId(id);

  const readState = async () => {
    return await browser.execute((selector, expectedStep, expectedId, sourceOptionId) => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const optionEl = document.querySelector(selector);
      if (step !== expectedStep) {
        return {
          step,
          done: true,
          reason: "step_changed",
          optionPresent: Boolean(optionEl),
          selectedOptionId: "",
          hasSourcePath: false,
          hasRepoUrl: false,
          hasWorkspaceName: false,
          optionTestIds: [],
        };
      }
      if (optionEl) {
        optionEl.click();
        return {
          step,
          done: true,
          reason: "clicked",
          optionPresent: true,
          selectedOptionId: "",
          hasSourcePath: false,
          hasRepoUrl: false,
          hasWorkspaceName: false,
          optionTestIds: [],
        };
      }

      const selected = root
        ? root.querySelector('[data-testid^="wizard-option-"].is-selected')
        : null;
      const selectedOptionId = String(selected?.getAttribute("data-testid") || "");
      const sourceAlreadySelected =
        expectedStep === "source"
        && (
          (sourceOptionId === "import" && Boolean(document.querySelector('[data-testid="wizard-source-path"]')))
          || (sourceOptionId === "clone" && Boolean(document.querySelector('[data-testid="wizard-repo-url"]')))
          || (sourceOptionId === "new" && Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')))
        );
      if (sourceAlreadySelected || selectedOptionId === expectedId) {
        return {
          step,
          done: true,
          reason: sourceAlreadySelected ? "source_selected" : "selected",
          optionPresent: false,
          selectedOptionId,
          hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
          hasRepoUrl: Boolean(document.querySelector('[data-testid="wizard-repo-url"]')),
          hasWorkspaceName: Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
          optionTestIds: [],
        };
      }

      return {
        step,
        done: false,
        reason: "missing",
        optionPresent: false,
        selectedOptionId,
        hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
        hasRepoUrl: Boolean(document.querySelector('[data-testid="wizard-repo-url"]')),
        hasWorkspaceName: Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
        optionTestIds: Array.from(document.querySelectorAll('[data-testid^="wizard-option-"]'))
          .map((el) => String(el.getAttribute("data-testid") || ""))
          .filter(Boolean),
      };
    }, sel, stepKey, id, optionId);
  };

  const deadline = Date.now() + 8000;
  let last = null;
  while (Date.now() < deadline) {
    const state = await readState();
    last = state;
    if (state?.done) return;
    await browser.pause(100);
  }
  throw new Error(`failed to select option '${id}'; diag=${JSON.stringify(last || {})}`);
};

const ensureContainerOptionVisible = async (optionId, timeoutMs = 15000) => {
  const optionSelector = `[data-testid="wizard-option-container-${optionId}"]`;
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute((selector) => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      if (step !== "container") {
        return { visible: false, toggled: false, step };
      }
      if (document.querySelector(selector)) {
        return { visible: true, toggled: false, step };
      }
      const toggle = document.querySelector('[data-testid="wizard-container-advanced-toggle"]');
      if (toggle) {
        toggle.click();
        return { visible: false, toggled: true, step };
      }
      return { visible: false, toggled: false, step };
    }, optionSelector);
    if (state.step !== "container") return false;
    if (state.visible) return true;
    await browser.pause(state.toggled ? 150 : 100);
  }
  return false;
};

const clickNext = async () => {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    const state = await clickNextIfEnabled();
    if (state.clicked) return;
    await browser.pause(100);
  }
  const key = await currentStepKey();
  throw new Error(`wizard-next not clickable at step '${key || "unknown"}'`);
};

const clickNextIfEnabled = async () => {
  return await browserExecuteWithRetry(() => {
    const el = document.querySelector('[data-testid="wizard-next"]');
    if (!(el instanceof HTMLButtonElement)) {
      return { present: false, disabled: null, clicked: false };
    }
    if (el.disabled) {
      return { present: true, disabled: true, clicked: false };
    }
    el.click();
    return { present: true, disabled: false, clicked: true };
  });
};

const clickHarnessSkipIfEnabled = async () => {
  return await browserExecuteWithRetry(() => {
    const el = document.querySelector('[data-testid="wizard-harness-skip"]');
    if (!(el instanceof HTMLButtonElement)) {
      return { present: false, disabled: null, clicked: false };
    }
    if (el.disabled) {
      return { present: true, disabled: true, clicked: false };
    }
    el.click();
    return { present: true, disabled: false, clicked: true };
  });
};

const clickHarnessSkip = async () => {
  await waitForTestId("wizard-harness-skip");
  await clickTestId("wizard-harness-skip");
};

const readWizardHarnessDownloadsState = async () => {
  return await browserExecuteWithRetry(() => {
    const step = document.querySelector('[data-testid="wizard-step"][data-step-key="harness-downloads"]');
    const error = step?.querySelector(".wizard-error")
      ? String(step.querySelector(".wizard-error")?.textContent || "").trim()
      : "";
    const rows = Array.from(document.querySelectorAll('[data-testid^="wizard-harness-checkbox-"]'))
      .flatMap((node) => {
        if (!(node instanceof HTMLInputElement) || node.type !== "checkbox") {
          return [];
        }
        const providerId = String(node.getAttribute("data-testid") || "")
          .replace(/^wizard-harness-checkbox-/, "")
          .trim();
        if (!providerId) return [];
        const row = node.closest(".wizard-auth-import-row");
        return [{
          providerId,
          checked: Boolean(node.checked),
          disabled: Boolean(node.disabled),
          statusText: String(row?.querySelector(".wizard-auth-import-path")?.textContent || "").trim(),
          errorText: String(row?.querySelector(".wizard-error")?.textContent || "").trim(),
        }];
      });
    const next = document.querySelector('[data-testid="wizard-next"]');
    return {
      error,
      rows,
      nextDisabled: next instanceof HTMLButtonElement ? next.disabled : null,
      nextLabel: next ? String(next.textContent || "").trim() : "",
    };
  });
};

const rowNeedsHarnessInstall = (row) => !String(row?.statusText || "").trim().startsWith("Installed");

const pickHarnessProvidersForNonBlockingInstallProof = async (providerIds, timeoutMs = 10_000) => {
  const preferred = Array.from(new Set((providerIds || []).map((value) => String(value || "").trim()).filter(Boolean)));
  const started = Date.now();
  let lastHarnessState = null;
  while (Date.now() - started < timeoutMs) {
    const harnessState = await readWizardHarnessDownloadsState();
    lastHarnessState = harnessState;
    const rows = Array.isArray(harnessState?.rows) ? harnessState.rows : [];
    const installableRows = rows.filter((row) =>
      row
      && row.providerId
      && row.disabled !== true
      && !row.errorText
      && rowNeedsHarnessInstall(row),
    );

    for (const providerId of preferred) {
      const row = installableRows.find((entry) => entry.providerId === providerId);
      if (row) {
        return {
          providerIds: [providerId],
          requestedProviderIds: preferred,
          harnessState,
        };
      }
    }

    const fallback = installableRows.find((row) => !preferred.includes(row.providerId));
    if (fallback) {
      return {
        providerIds: [fallback.providerId],
        requestedProviderIds: preferred,
        harnessState,
      };
    }
    await browser.pause(150);
  }

  throw new Error(
    `no installable harness rows remained to prove non-blocking background progress: ${JSON.stringify({
      requestedProviderIds: preferred,
      harnessState: lastHarnessState,
    })}`,
  );
};

const syncHarnessSelections = async (providerIds, timeoutMs = 10_000) => {
  if (!Array.isArray(providerIds) || providerIds.length === 0) return;
  const selected = Array.from(new Set(providerIds.map((value) => String(value || "").trim()).filter(Boolean)));
  if (selected.length === 0) return;
  const started = Date.now();
  let lastState = null;
  while (Date.now() - started < timeoutMs) {
    const result = await browserExecuteWithRetry((wanted) => {
      const inputs = Array.from(document.querySelectorAll('[data-testid^="wizard-harness-checkbox-"]'));
      const desired = new Set(wanted);
      const seen = [];
      const checked = [];
      const disabled = [];
      const ready = [];
      for (const node of inputs) {
        if (!(node instanceof HTMLInputElement) || node.type !== "checkbox") continue;
        const testId = String(node.getAttribute("data-testid") || "");
        const providerId = testId.replace(/^wizard-harness-checkbox-/, "");
        if (!providerId) continue;
        const row = node.closest(".wizard-auth-import-row");
        const statusText = String(row?.querySelector(".wizard-auth-import-path")?.textContent || "").trim();
        seen.push(providerId);
        const wantChecked = desired.has(providerId);
        if (!node.disabled && Boolean(node.checked) !== wantChecked) {
          node.click();
        }
        if (node.disabled) disabled.push(providerId);
        if (statusText.startsWith("Installed")) ready.push(providerId);
        if (node.checked) checked.push(providerId);
      }
      return { seen, checked, disabled, ready };
    }, [selected]);
    lastState = result;
    if (!result || !Array.isArray(result.seen) || result.seen.length === 0) {
      await browser.pause(150);
      continue;
    }
    const missing = selected.filter((providerId) => !result.seen.includes(providerId));
    if (missing.length > 0) {
      throw new Error(`expected harness row(s) ${JSON.stringify(missing)} to be present`);
    }
    if (selected.every((providerId) => result.checked.includes(providerId) || result.ready.includes(providerId))) {
      return {
        checked: result.checked,
        ready: result.ready,
      };
    }
    await browser.pause(150);
  }
  throw new Error(`failed to select harness rows: ${JSON.stringify(lastState)}`);
};

const readSelectedHarnessProviderIds = async () => {
  return await browser.execute(() => {
    return Array.from(document.querySelectorAll('[data-testid^="wizard-harness-checkbox-"]'))
      .flatMap((node) => {
        if (!(node instanceof HTMLInputElement) || node.type !== "checkbox" || !node.checked) {
          return [];
        }
        const testId = String(node.getAttribute("data-testid") || "");
        const providerId = testId.replace(/^wizard-harness-checkbox-/, "").trim();
        return providerId ? [providerId] : [];
      });
  });
};

const waitForSelectedHarnessInstallsToKickOff = async (providerIds, target = "host", timeoutMs = 30000) => {
  const selected = Array.from(new Set((providerIds || []).map((value) => String(value || "").trim()).filter(Boolean)));
  if (selected.length === 0) return;
  const started = Date.now();
  let lastProviders = [];
  while (Date.now() - started < timeoutMs) {
    const resp = await safeDaemonJson("GET", `/api/providers?target=${encodeURIComponent(target)}`);
    const providers = Array.isArray(resp.payload) ? resp.payload : [];
    lastProviders = providers
      .filter((provider) => selected.includes(String(provider?.provider_id || "")))
      .map((provider) => compactEntity(provider));
    const startedAll = providers
      .filter((provider) => selected.includes(String(provider?.provider_id || "")))
      .every((provider) => {
        const details = provider && typeof provider.details === "object" && provider.details
          ? provider.details
          : {};
        return provider.installed === true || details.install_running === "true";
      });
    if (startedAll && lastProviders.length === selected.length) {
      return;
    }
    await browser.pause(250);
  }
  throw new Error(
    `selected harness installs never kicked off: expected=${JSON.stringify(selected)} providers=${JSON.stringify(lastProviders)}`,
  );
};

const waitForWizardAdvanceWhileSelectedHarnessInstallsRun = async (
  providerIds,
  target = "host",
  timeoutMs = 30000,
) => {
  const selected = Array.from(new Set((providerIds || []).map((value) => String(value || "").trim()).filter(Boolean)));
  if (selected.length === 0) {
    throw new Error("expected pending selected harness installs to prove background progress, got none");
  }

  const started = Date.now();
  let lastSnapshot = null;
  while (Date.now() - started < timeoutMs) {
    const step = await currentStepKey();
    const resp = await safeDaemonJson("GET", `/api/providers?target=${encodeURIComponent(target)}`);
    const providers = Array.isArray(resp.payload) ? resp.payload : [];
    const relevant = providers.filter((provider) => selected.includes(String(provider?.provider_id || "")));
    const running = relevant.filter((provider) => {
      const details = provider && typeof provider.details === "object" && provider.details
        ? provider.details
        : {};
      return provider.installed !== true && details.install_running === "true";
    });
    lastSnapshot = {
      step,
      providers: relevant.map((provider) => compactEntity(provider)),
      error: resp.error || null,
    };

    if (step && step !== "harness-downloads" && running.length > 0) {
      return lastSnapshot;
    }

    if (
      step
      && step !== "harness-downloads"
      && relevant.length === selected.length
      && running.length === 0
      && relevant.every((provider) => provider.installed === true)
    ) {
      throw new Error(
        `wizard advanced only after selected harness installs completed; snapshot=${JSON.stringify(lastSnapshot)}`,
      );
    }

    await browser.pause(250);
  }

  throw new Error(
    `wizard never advanced while selected harness installs were still running: ${JSON.stringify(lastSnapshot)}`,
  );
};

const clickCreate = async (timeoutMs = 30000) => {
  await waitForTestId("wizard-create");
  const deadline = Date.now() + timeoutMs;
  let lastState = { present: false, disabled: null, text: "" };
  while (Date.now() < deadline) {
    const state = await browser.execute(() => {
      const el = document.querySelector('[data-testid="wizard-create"]');
      if (!(el instanceof HTMLButtonElement)) return { present: false, disabled: null, text: "" };
      return {
        present: true,
        disabled: Boolean(el.disabled),
        text: String(el.textContent || "").trim(),
      };
    });
    lastState = state || lastState;
    if (state?.present && state?.disabled === false) {
      await clickTestId("wizard-create");
      return;
    }
    await browser.pause(200);
  }
  if (!lastState?.present) throw new Error("wizard-create button missing at confirm step");
  throw new Error(`wizard-create button disabled at confirm step (label='${lastState.text || ""}')`);
};

const setInput = async (testId, value) => {
  await setInputTestId(testId, value);
};

const setChecked = async (testId, checked) => {
  await setCheckedTestId(testId, checked);
};


const compactEntity = (value) => {
  if (!value || typeof value !== "object") return value;
  const keys = [
    "id",
    "task_id",
    "session_id",
    "workspace_id",
    "status",
    "state",
    "title",
    "provider_id",
    "provider",
    "event_type",
    "type",
    "kind",
    "name",
    "role",
    "message_type",
    "status_reason",
    "created_at",
    "updated_at",
    "last_error",
    "error",
    "message",
    "content",
    "text",
  ];
  const out = {};
  for (const k of keys) {
    if (Object.prototype.hasOwnProperty.call(value, k)) out[k] = value[k];
  }
  const payloadKeys = ["payload", "event_json", "event", "data", "details", "delta"];
  for (const k of payloadKeys) {
    if (!Object.prototype.hasOwnProperty.call(value, k)) continue;
    try {
      const raw = value[k];
      const str = typeof raw === "string" ? raw : JSON.stringify(raw);
      out[k] = str.length > 800 ? `${str.slice(0, 800)}...` : str;
    } catch {
      out[k] = "[unserializable]";
    }
  }
  return Object.keys(out).length > 0 ? out : value;
};

const idString = (value) => {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (!value || typeof value !== "object") return "";
  if (typeof value.id === "string") return value.id;
  if (typeof value.value === "string") return value.value;
  if (typeof value.id === "number") return String(value.id);
  if (typeof value.value === "number") return String(value.value);
  return "";
};

const collectCodexSmokeDiagnostics = async (workspaceId) => {
  const diag = { workspaceId };
  const tasksResp = await safeDaemonJson("GET", `/api/workspaces/${workspaceId}/tasks`);
  diag.workspaceTasksStatus = tasksResp.status ?? null;
  if (tasksResp.error) diag.workspaceTasksError = tasksResp.error;
  const tasks = Array.isArray(tasksResp.payload) ? tasksResp.payload : [];
  diag.workspaceTasks = tasks.slice(-3).map((t) => compactEntity(t));

  const latestTask = tasks[tasks.length - 1];
  const taskId = idString(latestTask?.id || latestTask?.task_id || latestTask?.taskId);
  if (!taskId) return diag;
  diag.taskId = taskId;

  const sessionsResp = await safeDaemonJson("GET", `/api/tasks/${taskId}/sessions`);
  diag.taskSessionsStatus = sessionsResp.status ?? null;
  if (sessionsResp.error) diag.taskSessionsError = sessionsResp.error;
  const sessions = Array.isArray(sessionsResp.payload) ? sessionsResp.payload : [];
  diag.taskSessions = sessions.slice(-3).map((s) => compactEntity(s));

  const latestSession = sessions[sessions.length - 1];
  const sessionId = idString(latestSession?.id || latestSession?.session_id || latestSession?.sessionId);
  if (!sessionId) return diag;
  diag.sessionId = sessionId;

  const sessionStateResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/state`);
  diag.sessionStateStatus = sessionStateResp.status ?? null;
  if (sessionStateResp.error) diag.sessionStateError = sessionStateResp.error;
  diag.sessionState = compactEntity(sessionStateResp.payload);

  const sessionEventsResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/events?limit=5`);
  diag.sessionEventsStatus = sessionEventsResp.status ?? null;
  if (sessionEventsResp.error) diag.sessionEventsError = sessionEventsResp.error;
  const eventsPayload = sessionEventsResp.payload;
  const events = Array.isArray(eventsPayload?.events)
    ? eventsPayload.events
    : Array.isArray(eventsPayload)
      ? eventsPayload
      : [];
  diag.sessionEvents = events.slice(-5).map((evt) => compactEntity(evt));

  const sessionHistoryResp = await safeDaemonJson("GET", `/api/sessions/${sessionId}/history?limit=8`);
  diag.sessionHistoryStatus = sessionHistoryResp.status ?? null;
  if (sessionHistoryResp.error) diag.sessionHistoryError = sessionHistoryResp.error;
  const historyPayload = sessionHistoryResp.payload;
  const history = Array.isArray(historyPayload?.items)
    ? historyPayload.items
    : Array.isArray(historyPayload?.history)
      ? historyPayload.history
      : Array.isArray(historyPayload)
        ? historyPayload
        : [];
  diag.sessionHistory = history.slice(-8).map((item) => compactEntity(item));
  return diag;
};

const ensureCodexHarnessSelected = async (workspaceId = null, timeoutMs = 30000) => {
  const readHarnessState = async () => {
    return await browser.execute(() => {
      const trigger = document.querySelector(".wb-switcher-harness");
      const label = String(
        trigger?.querySelector(".wb-switcher-label")?.textContent
          || trigger?.getAttribute("aria-label")
          || "",
      ).trim();
      const menuOpen = Boolean(document.querySelector(".wb-harness-menu"));
      const codexRow = Array.from(document.querySelectorAll(".wb-harness-row")).find((row) => {
        const name = String(row.querySelector(".wb-harness-name")?.textContent || "").trim().toLowerCase();
        return name === "codex" || name.includes("codex");
      }) || null;
      const codexDisabled = codexRow
        ? Boolean(codexRow.querySelector(".wb-harness-row-main")?.hasAttribute("disabled"))
        : null;
      const rows = Array.from(document.querySelectorAll(".wb-harness-row")).map((row) => {
        const name = String(row.querySelector(".wb-harness-name")?.textContent || "").trim();
        const button = row.querySelector(".wb-harness-row-main");
        const disabled = Boolean(button?.hasAttribute("disabled"));
        const reason = button
          ? String(
            button.getAttribute("title")
            || button.getAttribute("aria-label")
            || button.getAttribute("data-disabled-reason")
            || "",
          ).trim()
          : "";
        return { name, disabled, reason };
      });
      return {
        label,
        menuOpen,
        hasCodexRow: Boolean(codexRow),
        codexDisabled,
        rows,
      };
    });
  };

  const openHarnessMenu = async () => {
    return await browser.execute(() => {
      if (document.querySelector(".wb-harness-menu")) return true;
      const trigger = document.querySelector(".wb-switcher-harness");
      if (!(trigger instanceof HTMLButtonElement)) return false;
      trigger.click();
      return true;
    });
  };

  const clickCodexRow = async () => {
    return await browser.execute(() => {
      const label = String(
        document.querySelector(".wb-switcher-harness .wb-switcher-label")?.textContent || "",
      ).trim();
      if (/codex/i.test(label)) return { ok: true, reason: "already-selected" };
      const rows = Array.from(document.querySelectorAll(".wb-harness-row"));
      const codexRow = rows.find((row) => {
        const name = String(row.querySelector(".wb-harness-name")?.textContent || "").trim().toLowerCase();
        return name === "codex" || name.includes("codex");
      });
      if (!codexRow) return { ok: false, reason: "missing" };
      const button = codexRow.querySelector(".wb-harness-row-main");
      if (!(button instanceof HTMLButtonElement)) return { ok: false, reason: "missing-button" };
      if (button.disabled) return { ok: false, reason: "disabled" };
      button.click();
      return { ok: true, reason: "clicked" };
    });
  };

  const triggerWorkbenchProviderRefresh = async () => {
    await browser.execute(() => {
      window.dispatchEvent(new Event("focus"));
      window.dispatchEvent(new Event("online"));
    });
  };

  let state = await readHarnessState();
  if (/codex/i.test(state.label)) return;

  let clicked = { ok: false, reason: "not-attempted" };
  const deadline = Date.now() + timeoutMs;
  let attempts = 0;
  await triggerWorkbenchProviderRefresh();
  while (Date.now() < deadline) {
    attempts += 1;
    if (!(await openHarnessMenu())) {
      throw new Error("harness selector trigger not found in composer");
    }
    state = await readHarnessState();
    if (/codex/i.test(state.label)) return;
    clicked = await clickCodexRow();
    if (clicked?.ok) break;
    if (clicked?.reason === "disabled" && attempts % 3 === 0) {
      await triggerWorkbenchProviderRefresh();
    }
    await browser.pause(250);
  }

  if (!clicked?.ok) {
    state = await readHarnessState();
    const diagnostics = await safeDaemonJson("GET", "/api/diagnostics");
    const workspaceBootstrap = workspaceId
      ? await safeDaemonJson("GET", `/api/workspaces/${workspaceId}/providers/bootstrap`)
      : null;
    const codexBootstrap =
      workspaceBootstrap?.payload?.providers?.find?.((p) => String(p?.provider_id || "") === "codex")
      || null;
    throw new Error(
      `unable to select Codex harness (${clicked?.reason || "unknown"}); harness_rows=${JSON.stringify(state.rows)}; diagnostics_status=${diagnostics.status ?? null}; startup_prewarm=${JSON.stringify(diagnostics.payload?.execution?.startup_prewarm || null)}; workspace_bootstrap_status=${workspaceBootstrap?.status ?? null}; codex_bootstrap=${JSON.stringify(codexBootstrap)}`,
    );
  }

  await browser.waitUntil(async () => {
    const s = await readHarnessState();
    return /codex/i.test(s.label);
  }, { timeout: timeoutMs, timeoutMsg: "harness selection did not settle on Codex" });
};

const runCodexComposerSmoke = async (workspaceId, timeoutMs = 240000) => {
  await waitForSelector("textarea.wb-new-composer-textarea", 60000);
  await ensureCodexHarnessSelected(workspaceId);
  await setTextareaSelector("textarea.wb-new-composer-textarea", "hello");
  await clickSelector("button.wb-send");

  let lastState = "{}";
  try {
    await browser.waitUntil(
      async () => {
        const state = await browser.execute(() => {
          const assistantEls = Array.from(document.querySelectorAll(".wb-assistant-entry, .msg.assistant"));
          const assistantText = assistantEls
            .map((el) => String(el.textContent || "").trim())
            .filter(Boolean)
            .join("\n");
          const wbBanner = Array.from(document.querySelectorAll(".wb-banner"))
            .map((el) => String(el.textContent || "").trim())
            .filter(Boolean)
            .join(" | ");
          const failHeader = Array.from(document.querySelectorAll(".wb-session-slot .banner strong"))
            .map((el) => String(el.textContent || "").trim())
            .find((t) => /failed to start/i.test(t)) || "";
          const failDetail = Array.from(document.querySelectorAll(".wb-session-slot .banner .error"))
            .map((el) => String(el.textContent || "").trim())
            .find(Boolean) || "";
          return {
            assistantText,
            wbBanner,
            failHeader,
            failDetail,
          };
        });
        const diag = {
          assistantText: String(state?.assistantText || ""),
          wbBanner: String(state?.wbBanner || ""),
          failHeader: String(state?.failHeader || ""),
          failDetail: String(state?.failDetail || ""),
        };
        lastState = JSON.stringify(diag);
        if (diag.failHeader || diag.failDetail || /failed to start/i.test(diag.wbBanner)) {
          throw new Error(`Codex session failed to start: ${lastState}`);
        }
        return diag.assistantText.length > 0;
      },
      {
        timeout: timeoutMs,
        interval: 250,
        timeoutMsg: `no assistant response received in time; last_state=${lastState}`,
      },
    );
  } catch (err) {
    const apiDiag = await collectCodexSmokeDiagnostics(workspaceId);
    throw new Error(
      `codex smoke did not complete: ${String(err)}; ui=${lastState}; api=${JSON.stringify(apiDiag)}`,
    );
  }
};

const runProviderFirstTurnApiSmoke = async (
  workspaceId,
  {
    providerId = "codex",
    modelId = "default",
    prompt = "hello",
  } = {},
  timeoutMs = 240000,
) => {
  const taskResp = await daemonJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: `${providerId}-smoke-${Date.now()}`,
    description: `Desktop automation smoke task for ${providerId}`,
    create_default_session: false,
  });
  if (taskResp.status !== 200) {
    throw new Error(`task create failed (${taskResp.status}): ${JSON.stringify(taskResp.payload || null)}`);
  }
  const taskId = String(taskResp.payload?.id || "").trim();
  if (!taskId) {
    throw new Error(`task create response missing id: ${JSON.stringify(taskResp.payload || null)}`);
  }

  const sessionResp = await daemonJson("POST", `/api/tasks/${taskId}/sessions`, {
    provider_id: providerId,
    model_id: modelId,
    env_target: "worktree",
  });
  if (sessionResp.status !== 200) {
    throw new Error(`session create failed (${sessionResp.status}): ${JSON.stringify(sessionResp.payload || null)}`);
  }
  const sessionId = String(sessionResp.payload?.id || "").trim();
  if (!sessionId) {
    throw new Error(`session create response missing id: ${JSON.stringify(sessionResp.payload || null)}`);
  }

  const postResp = await daemonJson("POST", `/api/sessions/${sessionId}/messages`, {
    content: prompt,
    delivery: "immediate",
    attachments: [],
  });
  if (postResp.status !== 200) {
    throw new Error(`message post failed (${postResp.status}): ${JSON.stringify(postResp.payload || null)}`);
  }

  const startedAt = Date.now();
  let lastHistory = null;
  while (Date.now() - startedAt < timeoutMs) {
    const history = await safeDaemonJson("GET", `/api/sessions/${sessionId}/history?limit=200`);
    if (history.status === 200 && history.payload) {
      lastHistory = history.payload;
      const messages = Array.isArray(history.payload.messages) ? history.payload.messages : [];
      const assistantMessage = messages
        .filter((m) => String(m?.role || "").toLowerCase() === "assistant")
        .map((m) => String(m?.content || "").trim())
        .find((content) => content.length > 0) || "";
      if (assistantMessage) {
        return {
          taskId,
          sessionId,
          assistantMessage,
        };
      }

      const turns = Array.isArray(history.payload.turns) ? history.payload.turns : [];
      const latestTurn = turns.length ? turns[turns.length - 1] : null;
      const latestStatus = String(latestTurn?.status || "").trim().toLowerCase();
      if (latestStatus === "failed" || latestStatus === "cancelled") {
        const turnId = String(latestTurn?.turn_id || latestTurn?.id || "").trim();
        const events = await safeDaemonJson("GET", `/api/sessions/${sessionId}/events?limit=200`);
        const eventRows = Array.isArray(events.payload?.events) ? events.payload.events : [];
        const turnEvents = turnId
          ? eventRows.filter((event) => String(event?.turn_id || "") === turnId)
          : eventRows;
        const errorEvent = turnEvents
          .slice()
          .reverse()
          .find((event) => String(event?.event_type || "").toLowerCase() === "error");
        const errorMessage = String(
          errorEvent?.payload_json?.message
            || errorEvent?.payload_json?.details
            || latestTurn?.error
            || latestTurn?.error_message
            || "turn failed",
        ).trim();
        throw new Error(
          `${providerId} first turn failed (status=${latestStatus}, turn_id=${turnId || "unknown"}): ${errorMessage}`,
        );
      }
    }
    await sleep(500);
  }

  throw new Error(
    `${providerId} first turn timed out (session=${sessionId}); last_history=${JSON.stringify(lastHistory)}`,
  );
};

const runCodexFirstTurnApiSmoke = async (workspaceId, options = {}, timeoutMs = 240000) => {
  return await runProviderFirstTurnApiSmoke(
    workspaceId,
    {
      providerId: options.providerId || "codex",
      modelId: options.modelId || "default",
      prompt: options.prompt || "hello",
    },
    timeoutMs,
  );
};

const getWorkspace = async (id) => {
  const resp = await daemonJson("GET", `/api/workspaces/${id}`);
  if (resp.status !== 200) throw new Error(`GET /api/workspaces/${id} failed (${resp.status})`);
  return resp.payload;
};

const getWorkspaceHarnessContainer = async (workspaceId) => {
  const resp = await daemonJson("GET", `/api/workspaces/${workspaceId}/harness_container`);
  if (resp.status !== 200) {
    throw new Error(`GET /api/workspaces/${workspaceId}/harness_container failed (${resp.status})`);
  }
  return resp.payload;
};

const createWorkspaceTerminal = async (workspaceId, body = {}) => {
  const resp = await daemonJson("POST", `/api/workspaces/${workspaceId}/terminals`, body);
  if (resp.status !== 200) {
    throw new Error(`POST /api/workspaces/${workspaceId}/terminals failed (${resp.status})`);
  }
  return resp.payload;
};

const deleteTerminal = async (terminalId) => {
  const resp = await daemonJson("DELETE", `/api/terminals/${terminalId}`);
  if (resp.status !== 204 && resp.status !== 404) {
    throw new Error(`DELETE /api/terminals/${terminalId} failed (${resp.status})`);
  }
};

const getWorkspaceTerminalCwd = async (workspaceId) => {
  const terminal = await createWorkspaceTerminal(workspaceId, {});
  const terminalId = typeof terminal.id === "string"
    ? terminal.id
    : String(terminal.id?.id || terminal.id?.value || terminal.id?.["0"] || "");
  if (!terminalId) {
    throw new Error(`terminal id missing: ${JSON.stringify(terminal)}`);
  }
  try {
    return String(terminal.cwd || "");
  } finally {
    await deleteTerminal(terminalId);
  }
};

const assertWorkspaceTerminalCwdPrefix = async (workspaceId, expectedPrefix) => {
  const cwd = await getWorkspaceTerminalCwd(workspaceId);
  if (!cwd.startsWith(expectedPrefix)) {
    throw new Error(`expected terminal cwd to start with '${expectedPrefix}', got '${cwd}'`);
  }
};

const daemonOverlayText = async () => {
  return await browserExecuteWithRetry(() => {
    const el = document.querySelector(".daemon-overlay");
    if (!el) return "";
    const text = String(el.textContent || "").trim();
    return text || "daemon overlay visible";
  });
};

const assertNoDaemonOverlayFor = async (durationMs = 2000) => {
  const started = Date.now();
  while (Date.now() - started < durationMs) {
    const text = await daemonOverlayText();
    if (text) throw new Error(`daemon unavailable overlay rendered: ${text}`);
    await browser.pause(100);
  }
};

const waitForRemoteStepAfterLocation = async (timeoutMs = 60000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const errEl = document.querySelector(".wizard-error");
      const err = errEl ? String(errEl.textContent || "").trim() : "";
      return { step, err };
    });
    const step = String(state?.step || "");
    const err = String(state?.err || "");
    if (err) throw new Error(`remote verification failed: ${err}`);
    if (step && step !== "location") return step;
    await browser.pause(100);
  }
  throw new Error("remote verification did not advance from location step within timeout");
};

const clickAuthImportSkip = async () => {
  const ok = await browser.execute(() => {
    const step = document.querySelector('[data-testid="wizard-step"][data-step-key="auth-import"]');
    if (!step) return false;
    const buttons = Array.from(step.querySelectorAll("button"));
    const skip = buttons.find((btn) => String(btn.textContent || "").trim() === "Skip for now");
    if (!skip) return false;
    skip.click();
    return true;
  });
  if (!ok) throw new Error("failed to skip auth-import step");
};

const skipAuthImportAndWait = async (timeoutMs = 30000) => {
  await clickAuthImportSkip();
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const key = await currentStepKey();
    if (key !== "auth-import") return key;
    await browser.pause(150);
  }
  throw new Error("auth-import step did not advance after skip");
};

const clickTitlingSkip = async () => {
  await waitForTestId("wizard-titling-skip");
  await clickTestId("wizard-titling-skip");
};

const ensureReadyForSourceSelection = async (
  {
    location,
    container,
    harnessDownloads,
    selectedHarnessProviderIds = null,
    requireSelectedHarnessInstallsNonBlocking = false,
    forbidLocationRegression = false,
    locationProgress = { leftLocation: false },
  },
  timeoutMs = harnessDownloads === "skip" ? 60000 : 300000,
) => {
  const started = Date.now();
  const shouldDownloadHarnesses = harnessDownloads === true || harnessDownloads === "download";
  while (Date.now() - started < timeoutMs) {
    const key = await currentStepKey();
    if (key && key !== "location") {
      locationProgress.leftLocation = true;
    }
    if (key === "source") return;
    if (key === "auth-import") {
      await skipAuthImportAndWait();
      continue;
    }
    if (key === "session-titling") {
      await clickTitlingSkip();
      await browser.pause(100);
      continue;
    }
    if (key === "container") {
      if (!container) throw new Error("container step reached but scenario.container is missing");
      if (container === "host-mounted") {
        const hostMountedVisible = await ensureContainerOptionVisible("host-mounted");
        if (!hostMountedVisible) {
          await browser.pause(100);
          continue;
        }
      }
      await clickOption("container", container);
      await browser.pause(100);
      const afterSelect = await currentStepKey();
      if (afterSelect === "container") {
        const next = await clickNextIfEnabled();
        if (next.clicked) {
          await browser.pause(100);
        }
      }
      continue;
    }
    if (key === "harness-downloads") {
      if (!shouldDownloadHarnesses) {
        const harnessState = await readWizardHarnessDownloadsState();
        const checkedRows = harnessState.rows.filter((row) => row.checked);
        if (checkedRows.length === 0 && harnessState.nextDisabled === false) {
          const next = await clickNextIfEnabled();
          if (next.clicked) {
            await browser.pause(100);
            continue;
          }
        }
        const skipped = await clickHarnessSkipIfEnabled();
        if (skipped.clicked) {
          await browser.pause(100);
          continue;
        }
        await browser.pause(150);
        continue;
      }
      if (Array.isArray(selectedHarnessProviderIds) && selectedHarnessProviderIds.length > 0) {
        let effectiveSelectedHarnessProviderIds = Array.from(
          new Set(selectedHarnessProviderIds.map((value) => String(value || "").trim()).filter(Boolean)),
        );
        if (requireSelectedHarnessInstallsNonBlocking) {
          const installProofSelection = await pickHarnessProvidersForNonBlockingInstallProof(
            effectiveSelectedHarnessProviderIds,
          );
          effectiveSelectedHarnessProviderIds = installProofSelection.providerIds;
        }
        const harnessSelectionState = await syncHarnessSelections(effectiveSelectedHarnessProviderIds);
        const readyProviders = new Set(Array.isArray(harnessSelectionState?.ready) ? harnessSelectionState.ready : []);
        const expectedKickoffProviderIds = Array.from(
          new Set(effectiveSelectedHarnessProviderIds.map((value) => String(value || "").trim()).filter(Boolean)),
        ).filter((providerId) => !readyProviders.has(providerId));
        const next = await clickNextIfEnabled();
        if (next.clicked) {
          const installTarget = container === "no-container" ? "host" : "container";
          if (requireSelectedHarnessInstallsNonBlocking) {
            if (expectedKickoffProviderIds.length === 0) {
              throw new Error(
                `selected harness installs were already complete before wizard could prove background progress: ${JSON.stringify({
                  selectedHarnessProviderIds: effectiveSelectedHarnessProviderIds,
                  requestedHarnessProviderIds: selectedHarnessProviderIds,
                  ready: Array.from(readyProviders),
                  installTarget,
                })}`,
              );
            }
            await waitForWizardAdvanceWhileSelectedHarnessInstallsRun(
              expectedKickoffProviderIds,
              installTarget,
              30000,
            );
          } else if (expectedKickoffProviderIds.length > 0) {
            await waitForSelectedHarnessInstallsToKickOff(
              expectedKickoffProviderIds,
              installTarget,
              30000,
            );
          }
          await browser.pause(100);
        } else {
          const harnessState = await readWizardHarnessDownloadsState();
          const selected = new Set(
            effectiveSelectedHarnessProviderIds.map((value) => String(value || "").trim()).filter(Boolean),
          );
          const blocked = harnessState.rows.filter((row) =>
            selected.has(row.providerId) && row.errorText,
          );
          if (blocked.length > 0) {
            throw new Error(
              `selected harness downloads blocked source selection: ${JSON.stringify({
                error: harnessState.error,
                nextDisabled: harnessState.nextDisabled,
                nextLabel: harnessState.nextLabel,
                blocked,
              })}`,
            );
          }
          // Planning work can temporarily disable Next before the kickoff transition settles.
          await browser.pause(150);
        }
        continue;
      }
      const expectedKickoffProviderIds = await readSelectedHarnessProviderIds();
      const next = await clickNextIfEnabled();
      if (next.clicked) {
        const installTarget = container === "no-container" ? "host" : "container";
        await waitForSelectedHarnessInstallsToKickOff(
          expectedKickoffProviderIds,
          installTarget,
          15000,
        );
        await browser.pause(100);
      } else {
        if (Array.isArray(selectedHarnessProviderIds) && selectedHarnessProviderIds.length > 0) {
          const harnessState = await readWizardHarnessDownloadsState();
          const selected = new Set(
            selectedHarnessProviderIds.map((value) => String(value || "").trim()).filter(Boolean),
          );
          const blocked = harnessState.rows.filter((row) =>
            selected.has(row.providerId) && row.errorText,
          );
          if (blocked.length > 0) {
            throw new Error(
              `selected harness downloads blocked source selection: ${JSON.stringify({
                error: harnessState.error,
                nextDisabled: harnessState.nextDisabled,
                nextLabel: harnessState.nextLabel,
                blocked,
              })}`,
            );
          }
        }
        // Planning work can temporarily disable Next before the kickoff transition settles.
        await browser.pause(150);
      }
      continue;
    }
    if (key === "location") {
      if (forbidLocationRegression && locationProgress.leftLocation) {
        throw new Error("wizard regressed to location after advancing");
      }
      if (!location) throw new Error("location step reached but scenario.location is missing");
      try {
        await clickOption("location", location);
      } catch (error) {
        const stepNow = await currentStepKey();
        if (!(location === "local" && stepNow === "container")) {
          throw error;
        }
      }
      await browser.pause(100);
      const afterSelect = await currentStepKey();
      if (afterSelect === "location") {
        const next = await clickNextIfEnabled();
        if (next.clicked) {
          await browser.pause(100);
        }
      }
      continue;
    }
    if (key === "network" || key === "setup" || key === "merge-queue" || key === "confirm") {
      throw new Error(`source step skipped unexpectedly; current step is '${key}'`);
    }
    await browser.pause(100);
  }
  const finalStep = await currentStepKey();
  throw new Error(`expected step 'source', got '${finalStep || "unknown"}'`);
};

const selectSourceOptionWithRetry = async (
  {
    location,
    container,
    harnessDownloads,
    sourceKind,
    selectedHarnessProviderIds = null,
    requireSelectedHarnessInstallsNonBlocking = false,
    forbidLocationRegression = false,
  },
  attempts = 6,
) => {
  const sourceSelectionReady = async () => {
    const state = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      return {
        step,
        hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
        hasRepoUrl: Boolean(document.querySelector('[data-testid="wizard-repo-url"]')),
        hasWorkspaceName: Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
      };
    });
    if (state.step !== "source") return false;
    if (sourceKind === "import") return state.hasSourcePath;
    if (sourceKind === "clone") return state.hasRepoUrl;
    if (sourceKind === "new") return state.hasWorkspaceName;
    return false;
  };

  let lastError = null;
  const locationProgress = { leftLocation: false };
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    await ensureReadyForSourceSelection({
      location,
      container,
      harnessDownloads,
      selectedHarnessProviderIds,
      requireSelectedHarnessInstallsNonBlocking,
      forbidLocationRegression,
      locationProgress,
    });
    try {
      await clickOption("source", sourceKind);
      if (await sourceSelectionReady()) return;
      lastError = new Error(`source option '${sourceKind}' click did not settle UI`);
    } catch (error) {
      lastError = error;
    }
    if (attempt === attempts - 1) break;
    await browser.pause(150);
  }
  if (lastError instanceof Error) throw lastError;
  throw new Error("failed to select source option");
};

const collectWorkspaceRouteDiagnostics = async () => {
  const ui = await browser.execute(() => {
    const pathname = window.location.pathname;
    const root = document.querySelector('[data-testid="workspace-setup"]');
    const step = root ? root.getAttribute("data-step-key") : null;
    const errEl = document.querySelector(".wizard-error");
    const err = errEl ? String(errEl.textContent || "").trim() : "";
    const overlayEl = document.querySelector(".daemon-overlay");
    const overlay = overlayEl ? String(overlayEl.textContent || "").trim() : "";
    const creatingEl = document.querySelector('[data-testid="wizard-creating-status"]');
    const creating = creatingEl ? String(creatingEl.textContent || "").trim() : "";
    const createBtn = document.querySelector('[data-testid="wizard-create"]');
    const createDisabled = createBtn instanceof HTMLButtonElement ? Boolean(createBtn.disabled) : null;
    const createLabel = createBtn ? String(createBtn.textContent || "").trim() : "";
    const summaryRows = Array.from(document.querySelectorAll(".wizard-summary-row"))
      .map((row) => {
        const k = row.querySelector(".wizard-summary-k");
        const v = row.querySelector(".wizard-summary-v");
        return {
          k: k ? String(k.textContent || "").trim() : "",
          v: v ? String(v.textContent || "").trim() : "",
        };
      })
      .filter((r) => r.k || r.v);
    return { pathname, step, err, overlay, creating, createDisabled, createLabel, summaryRows };
  });
  let conn;
  try {
    conn = await getConnectionInfo();
  } catch (error) {
    conn = { error: String(error) };
  }
  const wsResp = await safeDaemonJson("GET", "/api/workspaces");
  const workspaces = Array.isArray(wsResp.payload) ? wsResp.payload : [];
  return {
    ui,
    connection: conn,
    workspaceStatus: wsResp.status ?? null,
    workspaceError: wsResp.error || null,
    workspaces: workspaces.slice(-5).map((w) => compactEntity(w)),
  };
};

const waitForWorkspaceRoute = async (timeoutMs = 120000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute(() => {
      const pathname = window.location.pathname;
      const errEl = document.querySelector(".wizard-error");
      const err = errEl ? String(errEl.textContent || "").trim() : "";
      const overlayEl = document.querySelector(".daemon-overlay");
      const overlay = overlayEl ? String(overlayEl.textContent || "").trim() : "";
      return { pathname, err, overlay };
    });
    if (state?.err) {
      throw new Error(`wizard create error: ${state.err}`);
    }
    if (state?.overlay) {
      throw new Error(`daemon unavailable overlay rendered: ${state.overlay}`);
    }
    const p = String(state?.pathname || "");
    if (p.startsWith("/workspaces/")) {
      const id = p.split("/")[2] || "";
      if (!id) throw new Error("workspace id missing from route");
      return id;
    }
    await browser.pause(200);
  }
  const diag = await collectWorkspaceRouteDiagnostics();
  throw new Error(`did not navigate to /workspaces/:id; diag=${JSON.stringify(diag)}`);
};

const waitForLaunchLogsOrWorkspaceRoute = async (timeoutMs = 15000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const state = await browser.execute(() => {
      const pathname = window.location.pathname;
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const lines = document.querySelectorAll(".wizard-launch-log-line").length;
      const note = document.querySelector(".wizard-launch-log-body .wizard-note");
      const noteText = note ? String(note.textContent || "").trim() : "";
      return { pathname, step, lines, noteText };
    });
    const pathname = String(state?.pathname || "");
    if (pathname.startsWith("/workspaces/")) {
      return { kind: "workspace" };
    }
    if (state?.step === "confirm" && Number(state?.lines || 0) > 0) {
      return { kind: "logs" };
    }
    await browser.pause(150);
  }
  const diag = await collectWorkspaceRouteDiagnostics();
  throw new Error(`launch logs never appeared before workspace navigation; diag=${JSON.stringify(diag)}`);
};

const connectionSignature = (info) => JSON.stringify({
  kind: info?.kind || null,
  base_url: info?.base_url || null,
  token: info?.token || null,
  host: info?.host || null,
  user: info?.user || null,
  remote_port: info?.remote_port ?? null,
  remote_data_dir: info?.remote_data_dir || null,
});

const assertDesktopConnectionStable = async (durationMs = 5000, intervalMs = 250) => {
  const initial = await getConnectionInfo();
  if (!initial || typeof initial !== "object" || initial.kind === "none") {
    throw new Error(`expected active desktop connection, got: ${JSON.stringify(initial || null)}`);
  }
  const baseline = connectionSignature(initial);
  const samples = [];
  const started = Date.now();
  while (Date.now() - started < durationMs) {
    const current = await getConnectionInfo();
    const signature = connectionSignature(current);
    samples.push(current);
    if (signature !== baseline) {
      throw new Error(
        `desktop connection churn detected after workspace launch: baseline=${baseline} current=${signature} samples=${JSON.stringify(samples)}`,
      );
    }
    let health;
    try {
      health = await daemonJsonOnce("GET", "/api/health");
    } catch (error) {
      throw new Error(
        `desktop daemon health request failed immediately after workspace launch: ${String(error)}; samples=${JSON.stringify(samples)}`,
      );
    }
    if (Number(health.status) !== 200) {
      throw new Error(`desktop daemon health degraded after workspace launch: ${JSON.stringify(health)}`);
    }
    await browser.pause(intervalMs);
  }
};

const finalizeWizardSuccess = async (workspaceId) => {
  const trace = await readWizardStepTrace();
  await stopWizardStepTrace();
  assertNoLocationRegression(trace);
  // The workbench must never render the "daemon unavailable" overlay on first navigation.
  // If connect_local returns before the daemon is reachable, this can flash briefly.
  await assertNoDaemonOverlayFor(2000);
  await assertDesktopConnectionStable(5000, 250);
  return workspaceId;
};

const assertLocalWorkspaceConfig = async (workspaceId, expectations) => {
  const resp = await daemonJson("GET", `/api/workspaces/${workspaceId}/execution_config`);
  if (resp.status !== 200) {
    throw new Error(`GET /api/workspaces/${workspaceId}/execution_config failed (${resp.status})`);
  }
  const cfg = resp.payload || {};

  if (expectations.environment && cfg.environment !== expectations.environment) {
    throw new Error(`expected execution.environment=${expectations.environment}, got ${cfg.environment || "<missing>"}`);
  }

  if (expectations.networkMode) {
    const got = cfg.network_mode || "";
    if (got !== expectations.networkMode) {
      throw new Error(`expected execution.network_mode=${expectations.networkMode}, got ${got || "<missing>"}`);
    }
  }

  if (expectations.allowlist) {
    const got = Array.isArray(cfg.allowlist) ? cfg.allowlist : [];
    for (const entry of expectations.allowlist) {
      if (!got.includes(entry)) {
        throw new Error(`expected execution.allowlist to include '${entry}', got ${JSON.stringify(got)}`);
      }
    }
  }
};

// Quote a string for safe embedding in a single-quoted POSIX shell string.
const shSingleQuote = (s) => `'${String(s).replace(/'/g, `'\"'\"'`)}'`;

const sshArgs = (target, cmd) => {
  const args = [
    "-o", "StrictHostKeyChecking=accept-new",
    "-o", "ConnectTimeout=15",
  ];
  if (!REMOTE_PASSWORD) {
    args.push("-o", "BatchMode=yes");
  }
  args.push(target, `bash -lc ${shSingleQuote(cmd)}`);
  return args;
};

const ssh = (target, cmd) => {
  const args = sshArgs(target, cmd);
  if (REMOTE_PASSWORD) {
    return run("sshpass", ["-e", "ssh", ...args], {
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, SSHPASS: REMOTE_PASSWORD },
    });
  }
  return run("ssh", args, { stdio: ["ignore", "pipe", "pipe"] });
};

const ensureRemoteTarget = () => {
  if (!REMOTE_HOST) return null;
  if (REMOTE_HOST.includes("@")) return REMOTE_HOST;
  return `${REMOTE_USER_DEFAULT}@${REMOTE_HOST}`;
};

const connectLocalDesktop = async () => {
  const result = await browserExecuteWithRetry(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_connect_local");
      return { info };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  return result?.info || null;
};

const assertConnectedLocalAndListening = async () => {
  let info = await getConnectionInfo();
  if (!info || info.kind !== "local" || !info.base_url || !info.token) {
    await connectLocalDesktop();
    await browser.waitUntil(async () => {
      const refreshed = await getConnectionInfo();
      return Boolean(refreshed && refreshed.kind === "local" && refreshed.base_url && refreshed.token);
    }, {
      timeout: 30000,
      timeoutMsg: `expected local daemon connection after desktop_connect_local, got: ${JSON.stringify(info || null)}`,
    });
    info = await getConnectionInfo();
  }
  if (!info || info.kind !== "local") {
    throw new Error(`expected local daemon connection, got: ${JSON.stringify(info || null)}`);
  }
  if (!info.base_url || !info.token) {
    throw new Error(`expected base_url+token in connection info, got: ${JSON.stringify(info || null)}`);
  }
  // Sanity: confirm something is actually listening on the connected port.
  const u = new URL(info.base_url);
  const port = Number(u.port || "");
  if (!port) {
    throw new Error(`expected base_url to include a port, got: ${info.base_url}`);
  }
  const ls = spawnSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN"], { encoding: "utf8" });
  if (ls.status !== 0) {
    throw new Error(`expected daemon to be listening on port ${port}`);
  }
};

const runWizardScenario = async (scenario) => {
  if (Object.prototype.hasOwnProperty.call(scenario, "downloadHarnesses")) {
    throw new Error(
      "runWizardScenario(shared helper) expects scenario.harnessDownloads; " +
      "scenario.downloadHarnesses is only supported by workspace-wizard.spec.cjs's local helper",
    );
  }
  // Use a unique query param to force a real navigation (avoid SPA state re-use).
  await browser.url(`tauri://localhost/workspace-setup?e2e=${Date.now()}`);
  await waitForTauri();
  // Hard-reset wizard UI state between scenarios.
  // The wizard persists progress in webview storage; without clearing, the app can reopen mid-step.
  await browser.execute(() => {
    try { localStorage.clear(); } catch { /* ignore */ }
    try { sessionStorage.clear(); } catch { /* ignore */ }
  });
  await waitForTestId("workspace-setup", 60000);

  // If we land mid-wizard (e.g. due to single-instance or a prior run), walk back to the start.
  for (let i = 0; i < 12; i += 1) {
    const key = await currentStepKey();
    if (key === "location") break;
    const clicked = await clickBack();
    if (!clicked) break;
    await browser.pause(50);
  }

  await waitForStep("location", 90000);
  await startWizardStepTrace();
  try {
    await clickOption("location", scenario.location);
  } catch (error) {
    const stepNow = await currentStepKey();
    // The local location can already be selected and auto-advance into container.
    if (!(scenario.location === "local" && stepNow === "container")) {
      throw error;
    }
  }

  if (scenario.location === "remote") {
    await setInput("wizard-remote-host", scenario.remoteHost);
    if (
      typeof scenario.remotePort === "number"
      || typeof scenario.remoteDataDir === "string"
    ) {
      const hasRemotePortInput = await browser.execute(
        () => Boolean(document.querySelector('[data-testid="wizard-remote-port"]')),
      );
      if (hasRemotePortInput && typeof scenario.remotePort === "number") {
        await setInput("wizard-remote-port", String(scenario.remotePort));
      }
      const hasRemoteDataDirInput = await browser.execute(
        () => Boolean(document.querySelector('[data-testid="wizard-remote-data-dir"]')),
      );
      if (
        hasRemoteDataDirInput
        && typeof scenario.remoteDataDir === "string"
        && scenario.remoteDataDir.trim()
      ) {
        await setInput("wizard-remote-data-dir", scenario.remoteDataDir.trim());
      }
    }
    await clickNext(); // verifies SSH and advances
  }

  if (scenario.location === "remote") {
    const afterLocation = await waitForRemoteStepAfterLocation();
    if (afterLocation === "source" && scenario.container && scenario.container !== "no-container") {
      throw new Error("remote wizard did not expose container step (container modes unavailable)");
    }
    if (
      afterLocation !== "source"
      && afterLocation !== "container"
      && afterLocation !== "harness-downloads"
    ) {
      const wizardErr = await browser.execute(() => {
        const el = document.querySelector(".wizard-error");
        return el ? String(el.textContent || "").trim() : "";
      });
      throw new Error(
        `expected step 'container'|'harness-downloads'|'source', got '${afterLocation}' (wizard-error: ${wizardErr || "none"})`,
      );
    }
  }

  await selectSourceOptionWithRetry({
    location: scenario.location,
    container: scenario.container,
    harnessDownloads: scenario.harnessDownloads,
    sourceKind: scenario.source.kind,
    selectedHarnessProviderIds: Array.isArray(scenario.selectedHarnessProviderIds)
      ? scenario.selectedHarnessProviderIds
      : null,
    requireSelectedHarnessInstallsNonBlocking: Boolean(scenario.requireSelectedHarnessInstallsNonBlocking),
    forbidLocationRegression: true,
  });

  const sourcePathVisible = await browser.execute(
    () => Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
  );
  const expectsManagedStagingSource = (
    scenario.container === "disk-isolated"
    && (scenario.source.kind === "clone" || scenario.source.kind === "new")
  );

  if (scenario.source.kind === "import") {
    if (!sourcePathVisible) {
      throw new Error("wizard-source-path missing for import source");
    }
    await setInput("wizard-source-path", scenario.source.path);
  } else if (scenario.source.kind === "clone") {
    if (sourcePathVisible) {
      await setInput("wizard-source-path", scenario.source.destPath);
    } else if (!expectsManagedStagingSource) {
      throw new Error("wizard-source-path missing for clone source");
    }
    await setInput("wizard-repo-url", scenario.source.repoUrl);
    if (scenario.source.branch) {
      await setInput("wizard-repo-branch", scenario.source.branch);
    }
  } else if (scenario.source.kind === "new") {
    if (scenario.source.destPath && sourcePathVisible) {
      await setInput("wizard-source-path", scenario.source.destPath);
    } else if (scenario.source.destPath && !expectsManagedStagingSource) {
      throw new Error("wizard-source-path missing for new source");
    }
    if (scenario.source.workspaceName) {
      const hasWorkspaceNameInput = await browser.execute(
        () => Boolean(document.querySelector('[data-testid="wizard-workspace-name"]')),
      );
      if (hasWorkspaceNameInput) {
        await setInput("wizard-workspace-name", scenario.source.workspaceName);
      } else if (!expectsManagedStagingSource) {
        throw new Error("wizard-workspace-name missing for new source");
      }
    }
  }
  await clickNext();

  let afterSource;
  try {
    const transition = await waitForSourceExitOrWorkspaceRoute();
    if (transition.kind === "workspace") {
      return await finalizeWizardSuccess(transition.workspaceId);
    }
    afterSource = transition.step;
  } catch (error) {
    const sourceDiag = await browser.execute(() => {
      const root = document.querySelector('[data-testid="workspace-setup"]');
      const step = root ? root.getAttribute("data-step-key") : null;
      const errEl = document.querySelector(".wizard-error");
      const err = errEl ? String(errEl.textContent || "").trim() : "";
      const srcPath = document.querySelector('[data-testid="wizard-source-path"]');
      const wsName = document.querySelector('[data-testid="wizard-workspace-name"]');
      const next = document.querySelector('[data-testid="wizard-next"]');
      const selectedSource = root
        ? root.querySelector('[data-testid^="wizard-option-source-"].is-selected')?.getAttribute("data-testid") || ""
        : "";
      return {
        step,
        pathname: window.location.pathname,
        err,
        selectedSource,
        sourcePath: srcPath instanceof HTMLInputElement ? srcPath.value : "",
        workspaceName: wsName instanceof HTMLInputElement ? wsName.value : "",
        nextDisabled: next instanceof HTMLButtonElement ? next.disabled : null,
      };
    });
    throw new Error(
      `source step did not advance after Next: ${String(error)}; source_diag=${JSON.stringify(sourceDiag)}`,
    );
  }
  let current = afterSource;
  for (let i = 0; i < 18; i += 1) {
    if (current === "source") {
      const next = await clickNextIfEnabled();
      if (!next.clicked) {
        await browser.pause(150);
        current = await currentStepKey();
        continue;
      }
      const transition = await waitForSourceExitOrWorkspaceRoute(30000);
      if (transition.kind === "workspace") {
        return await finalizeWizardSuccess(transition.workspaceId);
      }
      current = transition.step;
      continue;
    }

    if (current === "auth-import") {
      current = await skipAuthImportAndWait();
      continue;
    }

    if (current === "session-titling") {
      await clickTitlingSkip();
      await browser.pause(100);
      current = await currentStepKey();
      continue;
    }

    if (current === "harness-downloads") {
      if (scenario.harnessDownloads === "skip") {
        const skipped = await clickHarnessSkipIfEnabled();
        await browser.pause(skipped.clicked ? 100 : 150);
        current = await currentStepKey();
        continue;
      }
      const next = await clickNextIfEnabled();
      await browser.pause(next.clicked ? 100 : 150);
      current = await currentStepKey();
      continue;
    }

    if (current === "network") {
      await clickOption("network", scenario.network);
      const afterSelectDiag = await browser.execute(() => {
        const root = document.querySelector('[data-testid="workspace-setup"]');
        const step = root ? root.getAttribute("data-step-key") : null;
        const selectedOptionId = root
          ? root.querySelector('[data-testid^="wizard-option-network-"].is-selected')?.getAttribute("data-testid") || ""
          : "";
        const hasAllowlistInput = Boolean(document.querySelector('[data-testid="wizard-network-allowlist"]'));
        return { step, selectedOptionId, hasAllowlistInput };
      });
      if (scenario.network === "allowlist") {
        if (afterSelectDiag.step !== "network") {
          throw new Error(
            `network step advanced before allowlist entry; diag=${JSON.stringify(afterSelectDiag)}`,
          );
        }
        if (!afterSelectDiag.hasAllowlistInput) {
          throw new Error(
            `wizard-network-allowlist missing after selecting allowlist; diag=${JSON.stringify(afterSelectDiag)}`,
          );
        }
        await setInput("wizard-network-allowlist", scenario.networkAllowlist || "github.com");
      }
      const afterNetworkSelection = await currentStepKey();
      if (afterNetworkSelection === "network") {
        await clickNext();
      }
      current = await currentStepKey();
      continue;
    }

    if (current === "setup") {
      if (scenario.setupHook) {
        await setInput("wizard-setup-hook", scenario.setupHook);
      }
      await clickNext();
      current = await currentStepKey();
      continue;
    }

    if (current === "merge-queue") {
      if (scenario.mergeQueue.kind === "skip") {
        await clickTestId("wizard-merge-skip");
      } else {
        await setInput("wizard-merge-target-branch", scenario.mergeQueue.targetBranch || "main");
        if (scenario.mergeQueue.verifyCommand) {
          await setInput("wizard-merge-verify-command", scenario.mergeQueue.verifyCommand);
        }
        if (scenario.mergeQueue.pushOnSuccess) {
          await clickTestId("wizard-merge-advanced-toggle");
          await setChecked("wizard-merge-push-on-success", true);
          if (scenario.mergeQueue.pushRemote) {
            await setInput("wizard-merge-push-remote", scenario.mergeQueue.pushRemote);
          }
          if (scenario.mergeQueue.pushBranch) {
            await setInput("wizard-merge-push-branch", scenario.mergeQueue.pushBranch);
          }
        }
        await clickNext();
      }
      current = await currentStepKey();
      continue;
    }

    if (current === "confirm") {
      break;
    }

    throw new Error(`expected post-source wizard step, got '${current || "unknown"}'`);
  }

  if (current !== "confirm") {
    throw new Error(`expected step 'confirm', got '${current || "unknown"}'`);
  }

  await waitForStep("confirm");
  if (typeof scenario.beforeCreate === "function") {
    await scenario.beforeCreate();
  }
  await clickCreate(
    scenario.container && scenario.container !== "no-container" ? CONTAINER_LAUNCH_TIMEOUT_MS : 30000,
  );
  if (scenario.container && scenario.container !== "no-container") {
    const launchVisibility = await waitForLaunchLogsOrWorkspaceRoute(15000);
    if (launchVisibility.kind === "logs" && typeof scenario.onLaunchLogsVisible === "function") {
      await scenario.onLaunchLogsVisible();
    }
  }

  const workspaceRouteTimeoutMs = scenario.location === "remote"
    ? REMOTE_LAUNCH_TIMEOUT_MS
    : (scenario.container && scenario.container !== "no-container" ? CONTAINER_LAUNCH_TIMEOUT_MS : 120000);
  const id = await waitForWorkspaceRoute(workspaceRouteTimeoutMs);
  return await finalizeWizardSuccess(id);
};


module.exports = {
  scenarioEnabled,
  parsePort,
  mkTempDir,
  initGitRepo,
  waitForSelector,
  clickSelector,
  setTextareaSelector,
  setCheckedTestId,
  clickBack,
  currentStepKey,
  waitForStep,
  startWizardStepTrace,
  readWizardStepTrace,
  stopWizardStepTrace,
  assertNoLocationRegression,
  waitForSourceExitOrWorkspaceRoute,
  clickOption,
  ensureContainerOptionVisible,
  clickNext,
  clickNextIfEnabled,
  clickHarnessSkipIfEnabled,
  clickHarnessSkip,
  clickCreate,
  setInput,
  setChecked,
  compactEntity,
  idString,
  collectCodexSmokeDiagnostics,
  ensureCodexHarnessSelected,
  runCodexComposerSmoke,
  runCodexFirstTurnApiSmoke,
  runProviderFirstTurnApiSmoke,
  getWorkspace,
  getWorkspaceHarnessContainer,
  createWorkspaceTerminal,
  deleteTerminal,
  getWorkspaceTerminalCwd,
  assertWorkspaceTerminalCwdPrefix,
  daemonOverlayText,
  assertNoDaemonOverlayFor,
  assertDesktopConnectionStable,
  waitForRemoteStepAfterLocation,
  clickAuthImportSkip,
  clickTitlingSkip,
  ensureReadyForSourceSelection,
  selectSourceOptionWithRetry,
  collectWorkspaceRouteDiagnostics,
  waitForWorkspaceRoute,
  waitForLaunchLogsOrWorkspaceRoute,
  finalizeWizardSuccess,
  assertLocalWorkspaceConfig,
  shSingleQuote,
  sshArgs,
  ssh,
  ensureRemoteTarget,
  assertConnectedLocalAndListening,
  runWizardScenario,
  REMOTE_HOST_RAW,
  REMOTE_HOST,
  REMOTE_USER_DEFAULT,
  REMOTE_PASSWORD,
  REMOTE_WIZARD_HOST_INPUT,
  REMOTE_PORT,
  REMOTE_DATA_DIR_RAW,
  SSH_NO_START_REMOTE,
};
