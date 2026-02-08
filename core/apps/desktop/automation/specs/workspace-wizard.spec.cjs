const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const REMOTE_HOST_RAW = process.env.CTX_AUTOMATION_REMOTE_HOST || "";
const REMOTE_HOST = REMOTE_HOST_RAW.trim();
const REMOTE_USER_DEFAULT = "dev";

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

const waitForTauri = async () => {
  await browser.waitUntil(
    async () => {
      const hasTauri = await browser.execute(() => Boolean(window.__TAURI__));
      return Boolean(hasTauri);
    },
    { timeout: 30000, timeoutMsg: "Tauri bridge not available in time." },
  );
};

const selectorForTestId = (id) => `[data-testid="${id}"]`;

const waitForTestId = async (id, timeoutMs = 30000) => {
  const sel = selectorForTestId(id);
  await browser.waitUntil(
    async () => await browser.execute((s) => Boolean(document.querySelector(s)), sel),
    { timeout: timeoutMs, timeoutMsg: `element not found: ${sel}` },
  );
};

const clickTestId = async (id) => {
  await waitForTestId(id);
  const sel = selectorForTestId(id);
  const ok = await browser.execute((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    el.click();
    return true;
  }, sel);
  if (!ok) throw new Error(`failed to click: ${sel}`);
};

const setInputTestId = async (id, value) => {
  await waitForTestId(id);
  const sel = selectorForTestId(id);
  const ok = await browser.execute((s, v) => {
    const el = document.querySelector(s);
    if (!el) return false;
    const isTextArea = el instanceof HTMLTextAreaElement;
    const proto = isTextArea ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, "value");
    const setter = desc && desc.set;
    if (!setter) return false;
    setter.call(el, v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }, sel, String(value));
  if (!ok) throw new Error(`failed to set value for: ${sel}`);
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

const waitForStep = async (key) => {
  await browser.waitUntil(
    async () => (await currentStepKey()) === key,
    { timeout: 30000, timeoutMsg: `expected step '${key}'` },
  );
};

const clickOption = async (stepKey, optionId) => {
  await clickTestId(`wizard-option-${stepKey}-${optionId}`);
};

const clickNext = async () => {
  await clickTestId("wizard-next");
};

const clickCreate = async () => {
  await clickTestId("wizard-create");
};

const setInput = async (testId, value) => {
  await setInputTestId(testId, value);
};

const setChecked = async (testId, checked) => {
  await setCheckedTestId(testId, checked);
};

const daemonJson = async (method, apiPath, body) => {
  const exec = async (m, p, b) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const headers = [["content-type", "application/json"]];
      const req = {
        method: m,
        path: p,
        // Avoid passing null through the WebDriver arg marshaller (CrabNebula throws).
        body: typeof b === "undefined" ? null : JSON.stringify(b),
        headers,
      };
      const raw = await invoke("desktop_daemon_request", { req });
      const payload = JSON.parse(raw.body || "{}");
      return { status: raw.status, payload };
    } catch (e) {
      return { error: String(e) };
    }
  };

  // NOTE: CrabNebula WebDriver has stricter argument marshalling; do not pass null as an arg.
  const resp = typeof body === "undefined"
    ? await browser.execute(exec, method, apiPath)
    : await browser.execute(exec, method, apiPath, body);
  if (resp && typeof resp === "object" && resp.error) {
    throw new Error(resp.error);
  }
  return resp;
};

const getConnectionInfo = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_get_connection");
      return { info };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  return result.info;
};

const getWorkspace = async (id) => {
  const resp = await daemonJson("GET", `/api/workspaces/${id}`);
  if (resp.status !== 200) throw new Error(`GET /api/workspaces/${id} failed (${resp.status})`);
  return resp.payload;
};

const daemonOverlayText = async () => {
  return await browser.execute(() => {
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

const waitForWorkspaceRoute = async () => {
  const timeoutMs = 120000;
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
  throw new Error("did not navigate to /workspaces/:id");
};

const assertLocalWorkspaceConfig = (rootPath, expectations) => {
  const cfg = path.join(rootPath, ".ctx", "config.toml");
  if (!fs.existsSync(cfg)) {
    throw new Error(`missing workspace config: ${cfg}`);
  }
  const text = fs.readFileSync(cfg, "utf8");
  if (expectations.mergeQueueEnabled === false) {
    // When the user skips merge queue setup, the wizard intentionally does not write any
    // merge_queue config (the daemon defaults still apply).
  } else if (expectations.mergeQueueEnabled === true) {
    if (!/merge_queue/i.test(text)) {
      throw new Error(`expected merge queue config to be written: ${cfg}`);
    }
    if (expectations.targetBranch && !text.includes(expectations.targetBranch)) {
      throw new Error(`expected target branch '${expectations.targetBranch}' in ${cfg}`);
    }
  }
  if (expectations.setupHook) {
    if (!text.includes(expectations.setupHook)) {
      throw new Error(`expected setup hook '${expectations.setupHook}' in ${cfg}`);
    }
  }
};

// Quote a string for safe embedding in a single-quoted POSIX shell string.
const shSingleQuote = (s) => `'${String(s).replace(/'/g, `'\"'\"'`)}'`;

const ssh = (target, cmd) =>
  run("ssh", ["-o", "BatchMode=yes", target, `bash -lc ${shSingleQuote(cmd)}`], { stdio: ["ignore", "pipe", "pipe"] });

const ensureRemoteTarget = () => {
  if (!REMOTE_HOST) return null;
  if (REMOTE_HOST.includes("@")) return REMOTE_HOST;
  return `${REMOTE_USER_DEFAULT}@${REMOTE_HOST}`;
};

const assertConnectedLocalAndListening = async () => {
  const info = await getConnectionInfo();
  if (!info || info.kind !== "local") {
    throw new Error(`expected local daemon connection, got: ${JSON.stringify(info)}`);
  }
  if (!info.base_url || !info.token) {
    throw new Error(`expected base_url+token in connection info, got: ${JSON.stringify(info)}`);
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

  await waitForStep("location");
  await clickOption("location", scenario.location);

  if (scenario.location === "remote") {
    await setInput("wizard-remote-host", scenario.remoteHost);
    await clickNext(); // verifies SSH and advances
  }

  await waitForStep("source");
  await clickOption("source", scenario.source.kind);

  if (scenario.source.kind === "import") {
    await setInput("wizard-source-path", scenario.source.path);
  } else if (scenario.source.kind === "clone") {
    await setInput("wizard-source-path", scenario.source.destPath);
    await setInput("wizard-repo-url", scenario.source.repoUrl);
    if (scenario.source.branch) {
      await setInput("wizard-repo-branch", scenario.source.branch);
    }
  } else if (scenario.source.kind === "new") {
    if (scenario.source.destPath) {
      await setInput("wizard-source-path", scenario.source.destPath);
    }
    if (scenario.source.workspaceName) {
      await setInput("wizard-workspace-name", scenario.source.workspaceName);
    }
  }
  await clickNext();

  await waitForStep("setup");
  if (scenario.setupHook) {
    await setInput("wizard-setup-hook", scenario.setupHook);
  }
  await clickNext();

  await waitForStep("merge-queue");
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

  await waitForStep("confirm");
  await clickCreate();

  const id = await waitForWorkspaceRoute();
  // The workbench must never render the "daemon unavailable" overlay on first navigation.
  // If connect_local returns before the daemon is reachable, this can flash briefly.
  await assertNoDaemonOverlayFor(2000);
  return id;
};

describe("launcher workspace wizard (e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-e2e-${runId}-`);
  const localImportRepo = path.join(localBase, "import-repo");
  const localCloneSrc = path.join(localBase, "clone-src");

  const remoteTarget = ensureRemoteTarget();
  const remoteBase = `/tmp/ctx-e2e-${runId}`;

  before(async () => {
    initGitRepo(localImportRepo, "local-import");
    initGitRepo(localCloneSrc, "local-clone-src");

    if (remoteTarget) {
      // Pre-create remote repos for import/clone without touching the daemon.
      const script = [
        "set -euo pipefail",
        `base=${JSON.stringify(remoteBase)}`,
        "rm -rf \"$base\"",
        "mkdir -p \"$base\"",
        "mkdir -p \"$base/import-repo\"",
        "git init \"$base/import-repo\"",
        "git -C \"$base/import-repo\" config user.email ctx-e2e@example.com",
        "git -C \"$base/import-repo\" config user.name ctx-e2e",
        "echo hi >\"$base/import-repo/README.md\"",
        "git -C \"$base/import-repo\" add README.md",
        "git -C \"$base/import-repo\" commit -m init",
        "mkdir -p \"$base/clone-src\"",
        "git init \"$base/clone-src\"",
        "git -C \"$base/clone-src\" config user.email ctx-e2e@example.com",
        "git -C \"$base/clone-src\" config user.name ctx-e2e",
        "echo hi >\"$base/clone-src/README.md\"",
        "git -C \"$base/clone-src\" add README.md",
        "git -C \"$base/clone-src\" commit -m init",
      ].join("\n");
      ssh(remoteTarget, script);
    }
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore
    }
    if (remoteTarget) {
      try {
        ssh(remoteTarget, `rm -rf ${JSON.stringify(remoteBase)}`);
      } catch {
        // ignore
      }
    }
  });

  it("local import works end-to-end", async () => {
    const id = await runWizardScenario({
      location: "local",
      source: { kind: "import", path: localImportRepo },
      setupHook: "pnpm install",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    assertLocalWorkspaceConfig(ws.root_path, { mergeQueueEnabled: false, setupHook: "pnpm install" });
  });

  it("local clone works end-to-end (merge queue enabled)", async () => {
    const destParent = path.join(localBase, "clone-dest");
    fs.mkdirSync(destParent, { recursive: true });
    const destPath = `${destParent}/`; // trailing slash -> derive repo name

    const id = await runWizardScenario({
      location: "local",
      source: { kind: "clone", repoUrl: localCloneSrc, branch: "", destPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "pnpm -v" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    assertLocalWorkspaceConfig(ws.root_path, { mergeQueueEnabled: true, targetBranch: "main", setupHook: "pnpm install" });
  });

  it("local new empty works end-to-end", async () => {
    const dest = path.join(localBase, "new-sealed");
    const id = await runWizardScenario({
      location: "local",
      source: { kind: "new", destPath: dest, workspaceName: "sealed-ws" },
      setupHook: "",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "" },
    });

    await assertConnectedLocalAndListening();
    const ws = await getWorkspace(id);
    assertLocalWorkspaceConfig(ws.root_path, { mergeQueueEnabled: true, targetBranch: "main" });
  });

  it("remote import works end-to-end", async function () {
    if (!remoteTarget) this.skip();
    const importPath = `${remoteBase}/import-repo`;

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteTarget,
      source: { kind: "import", path: importPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "skip" },
    });

    const ws = await getWorkspace(id);
    ssh(remoteTarget, `test -f ${JSON.stringify(ws.root_path + "/.ctx/config.toml")}`);
  });

  it("remote clone works end-to-end", async function () {
    if (!remoteTarget) this.skip();
    const destParent = `${remoteBase}/clone-dest`;
    const destPath = `${destParent}/`;
    const src = `${remoteBase}/clone-src`;
    ssh(remoteTarget, `mkdir -p ${JSON.stringify(destParent)}`);

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteTarget,
      source: { kind: "clone", repoUrl: src, branch: "", destPath },
      setupHook: "pnpm install",
      mergeQueue: { kind: "enabled", targetBranch: "main", verifyCommand: "echo ok" },
    });

    const ws = await getWorkspace(id);
    ssh(remoteTarget, `test -f ${JSON.stringify(ws.root_path + "/.ctx/config.toml")}`);
  });

  it("remote new empty works end-to-end", async function () {
    if (!remoteTarget) this.skip();
    const dest = `${remoteBase}/new-sealed`;
    ssh(remoteTarget, `rm -rf ${JSON.stringify(dest)}`);

    const id = await runWizardScenario({
      location: "remote",
      remoteHost: remoteTarget,
      source: { kind: "new", destPath: dest, workspaceName: "sealed-remote" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    const ws = await getWorkspace(id);
    ssh(remoteTarget, `test -f ${JSON.stringify(ws.root_path + "/.ctx/config.toml")}`);
  });
});
