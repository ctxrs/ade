const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const { resolveRemoteFixtureEnv } = require("../helpers/remote_fixture_contract.cjs");

const LAUNCHER_RECENTS_KEY = "wb.launcher_recents";
const remoteFixture = resolveRemoteFixtureEnv({ lane: "host" });

const runChecked = (cmd, args, options = {}) => {
  const result = spawnSync(cmd, args, {
    encoding: "utf8",
    ...options,
  });
  if (result.status === 0) return result;
  throw new Error(
    [
      `command failed: ${cmd} ${args.join(" ")}`,
      String(result.stderr || "").trim(),
      String(result.stdout || "").trim(),
    ].filter(Boolean).join("\n"),
  );
};

const initGitRepo = (root) => {
  runChecked("git", ["init", "--", root]);
  runChecked("git", ["-C", root, "config", "user.email", "ctx-e2e@example.com"]);
  runChecked("git", ["-C", root, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(root, "README.md"), "# recent workspace regression\n", "utf8");
  runChecked("git", ["-C", root, "add", "README.md"]);
  runChecked("git", ["-C", root, "commit", "-m", "init"]);
};

const createAliasedRepo = () => {
  const actualRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-recent-workspace-actual-"));
  initGitRepo(actualRoot);
  const aliasRoot = path.join(os.tmpdir(), `ctx-recent-workspace-alias-${Date.now()}-${Math.trunc(Math.random() * 100000)}`);
  fs.symlinkSync(actualRoot, aliasRoot, process.platform === "win32" ? "junction" : "dir");
  return {
    actualRoot,
    workspaceRoot: fs.realpathSync(actualRoot),
    aliasRoot,
    cleanup() {
      fs.rmSync(aliasRoot, { recursive: true, force: true });
      fs.rmSync(actualRoot, { recursive: true, force: true });
    },
  };
};

const createTempRepo = (prefix) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  initGitRepo(root);
  return {
    root,
    cleanup() {
      fs.rmSync(root, { recursive: true, force: true });
    },
  };
};

const tauriInvoke = async (command, args) => {
  const result = await browser.executeAsync(({ cmd, payload }, done) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      done({ ok: false, error: "Tauri invoke not available" });
      return;
    }
    Promise.resolve()
      .then(() => invoke(cmd, payload || {}))
      .then((value) => done({ ok: true, value }))
      .catch((error) => done({ ok: false, error: String(error) }));
  }, { cmd: command, payload: args });
  if (!result || !result.ok) {
    throw new Error(result?.error || `invoke failed: ${command}`);
  }
  return result.value;
};

const getDesktopConnection = async () => {
  const info = await tauriInvoke("desktop_get_connection");
  if (info && info.base_url && info.browser_query_secret) return info;
  throw new Error(`desktop_get_connection returned no daemon details: ${JSON.stringify(info || null)}`);
};

const daemonJson = async (method, apiPath, body) => {
  const connection = await getDesktopConnection();
  const url = new URL(apiPath, String(connection.base_url || "")).toString();
  const response = await fetch(url, {
    method,
    headers: {
      authorization: `Bearer ${String(connection.browser_query_secret || "")}`,
      "content-type": "application/json",
    },
    body: typeof body === "undefined" ? undefined : JSON.stringify(body),
  });
  const raw = await response.text();
  let payload = {};
  if (raw.trim()) {
    try {
      payload = JSON.parse(raw);
    } catch {
      payload = { raw };
    }
  }
  return {
    status: response.status,
    payload,
  };
};

const readWorkspaceId = (response) => {
  const workspaceId = String(response?.payload?.id || "").trim();
  if (!workspaceId) {
    throw new Error(`workspace id missing from response: ${JSON.stringify(response || null)}`);
  }
  return workspaceId;
};

const readRepoStagingPath = async () => {
  const response = await daemonJson("GET", "/api/repo/staging_path");
  if (response.status !== 200) {
    throw new Error(`repo staging path failed: ${JSON.stringify(response)}`);
  }
  const stagingPath = String(response?.payload?.path || "").trim();
  if (!stagingPath) {
    throw new Error(`repo staging path missing from response: ${JSON.stringify(response || null)}`);
  }
  return stagingPath;
};

const seedLauncherRecents = async (entries) => {
  await tauriInvoke("desktop_storage_batch", {
    ops: [
      {
        kind: "set",
        key: LAUNCHER_RECENTS_KEY,
        value: {
          v: 1,
          entries,
          updatedAtMs: Date.now(),
        },
      },
    ],
  });
};

const readLauncherRecents = async () => {
  return await tauriInvoke("desktop_storage_get", { key: LAUNCHER_RECENTS_KEY });
};

const openLauncher = async (tag) => {
  await navigateToTauriUrl(`tauri://localhost/?recent_open_e2e=${encodeURIComponent(String(tag || Date.now()))}`);
  await browser.waitUntil(
    async () => await browser.execute(() => Boolean(document.querySelector(".launcher-panel"))),
    { timeout: 30000, timeoutMsg: "launcher did not render" },
  );
};

const clickLauncherRecent = async (label) => {
  await browser.waitUntil(
    async () => await browser.execute((recentLabel) => {
      const buttons = Array.from(document.querySelectorAll(".launcher-recent-item"));
      const button = buttons.find((node) => String(node.textContent || "").includes(String(recentLabel)));
      if (!(button instanceof HTMLElement)) return false;
      button.click();
      return true;
    }, label),
    { timeout: 30000, timeoutMsg: `launcher recent not clickable: ${label}` },
  );
};

const waitForWorkspaceRoute = async (workspaceId) => {
  await browser.waitUntil(
    async () => {
      const pathname = await browser.execute(() => window.location.pathname || "");
      return pathname === `/workspaces/${workspaceId}`;
    },
    { timeout: 90000, timeoutMsg: `expected to reach /workspaces/${workspaceId}` },
  );
  await browser.waitUntil(
    async () => await browser.execute(() => Boolean(document.querySelector(".wb-main"))),
    { timeout: 30000, timeoutMsg: "workbench main area did not render" },
  );
  const wizardVisible = await browser.execute(
    () => Boolean(document.querySelector('[data-testid="workspace-setup"]')),
  );
  if (wizardVisible) {
    throw new Error("workspace setup wizard remained visible after clicking recent workspace");
  }
};

const sshBaseArgs = (fixture, { useKey = true } = {}) => {
  const args = [
    "-o", "StrictHostKeyChecking=no",
    "-o", "UserKnownHostsFile=/dev/null",
    "-o", "ConnectTimeout=10",
  ];
  if (fixture.sshConfigPath) {
    args.unshift(fixture.sshConfigPath);
    args.unshift("-F");
  } else {
    args.unshift("/dev/null");
    args.unshift("-F");
  }
  if (fixture.sshPort > 0) {
    args.push("-p", String(fixture.sshPort));
  }
  if (useKey && fixture.sshKeyPath) {
    args.unshift("IdentitiesOnly=yes");
    args.unshift("-o");
    args.unshift(fixture.sshKeyPath);
    args.unshift("-i");
  }
  return args;
};

const remoteSsh = (fixture, command) => {
  const authMode = String(fixture.authMode || "").trim().toLowerCase();
  const usePassword = authMode === "password";
  const baseArgs = sshBaseArgs(fixture, { useKey: !usePassword });
  let binary = "ssh";
  let args = [...baseArgs, fixture.target, command];
  let env = process.env;
  if (usePassword) {
    const password = String(fixture.passwordActual || fixture.password || "").trim();
    if (!password) {
      throw new Error("remote fixture requested password auth but no password is configured");
    }
    binary = "sshpass";
    args = [
      "-e",
      "ssh",
      ...baseArgs,
      "-o", "BatchMode=no",
      "-o", "PreferredAuthentications=password,keyboard-interactive",
      "-o", "NumberOfPasswordPrompts=1",
      fixture.target,
      command,
    ];
    env = { ...process.env, SSHPASS: password };
  }
  const result = runChecked(binary, args, {
    stdio: ["ignore", "pipe", "pipe"],
    env,
  });
  return String(result.stdout || "").trim();
};

const ensureRemoteWorkspaceRepo = (fixture) => {
  const remoteRoot = `/tmp/ctx-recent-open-remote-${Date.now()}-${Math.trunc(Math.random() * 100000)}`;
  const setupScript = [
    "set -euo pipefail",
    `mkdir -p ${JSON.stringify(remoteRoot)}`,
    `cd ${JSON.stringify(remoteRoot)}`,
    "git init >/dev/null 2>&1",
    "git config user.email ctx-e2e@example.com",
    "git config user.name ctx-e2e",
    "printf '%s\\n' '# remote recent workspace regression' > README.md",
    "git add README.md",
    "if ! git rev-parse --verify HEAD >/dev/null 2>&1; then git commit -m 'init' >/dev/null 2>&1; fi",
  ].join("; ");
  remoteSsh(fixture, `bash -lc ${JSON.stringify(setupScript)}`);
  return remoteRoot;
};

const connectSshWithPolling = async (req, timeoutMs = 240000) => {
  const begin = await tauriInvoke("desktop_connect_ssh_begin", { req }).catch((error) => ({ error: String(error) }));
  if (begin && begin.error) {
    const detail = String(begin.error || "").toLowerCase();
    if (detail.includes("desktop_connect_ssh_begin") || detail.includes("unknown command")) {
      return await tauriInvoke("desktop_connect_ssh", { req });
    }
    throw new Error(begin.error);
  }

  const jobId = String(begin || "").trim();
  if (!jobId) {
    throw new Error("desktop_connect_ssh_begin returned an empty job id");
  }

  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const snapshot = await tauriInvoke("desktop_connect_ssh_poll", {
      req: { job_id: jobId, consume: false },
    });
    const status = String(snapshot?.status || "").trim().toLowerCase();
    if (status === "succeeded") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      return snapshot?.info || null;
    }
    if (status === "failed") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      throw new Error(String(snapshot?.error || "desktop_connect_ssh failed"));
    }
    await browser.pause(500);
  }

  await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } }).catch(() => {});
  throw new Error("desktop_connect_ssh timed out waiting for completion");
};

describe("recent workspace open automation", () => {
  const cleanups = [];

  afterEach(async () => {
    while (cleanups.length > 0) {
      const cleanup = cleanups.pop();
      try {
        cleanup?.();
      } catch {
        // best-effort cleanup only
      }
    }
    try {
      await tauriInvoke("desktop_disconnect");
    } catch {
      // ignore disconnect failures during cleanup
    }
  });

  it("opens local host recents even when the stored recent path is a non-canonical alias", async () => {
    const repo = createAliasedRepo();
    cleanups.push(() => repo.cleanup());

    await openLauncher("local-host");
    await tauriInvoke("desktop_disconnect").catch(() => {});
    await tauriInvoke("desktop_connect_local");

    const createWorkspaceResp = await daemonJson("POST", "/api/workspaces", {
      root_path: repo.workspaceRoot,
      name: "Recent Host Workspace",
    });
    if (createWorkspaceResp.status !== 200 && createWorkspaceResp.status !== 201) {
      throw new Error(`workspace creation failed: ${JSON.stringify(createWorkspaceResp)}`);
    }
    const workspaceId = readWorkspaceId(createWorkspaceResp);
    const storedWorkspaceRoot = String(createWorkspaceResp?.payload?.root_path || "").trim() || repo.workspaceRoot;
    if (storedWorkspaceRoot === repo.aliasRoot) {
      throw new Error(`host recent test did not create a distinct alias path: ${storedWorkspaceRoot}`);
    }

    await seedLauncherRecents([
      {
        kind: "local",
        label: "Recent Host Workspace",
        root_path: repo.aliasRoot,
        execution_environment: "host",
        updated_at_ms: Date.now(),
      },
    ]);

    await openLauncher("local-host-seeded");
    await clickLauncherRecent("Recent Host Workspace");
    await waitForWorkspaceRoute(workspaceId);
    await browser.waitUntil(
      async () => {
        const persisted = await readLauncherRecents();
        return String(persisted?.entries?.[0]?.root_path || "") === storedWorkspaceRoot;
      },
      {
        timeout: 10000,
        timeoutMsg: `expected launcher recents to heal to '${storedWorkspaceRoot}'`,
      },
    );
  });

  it("opens local container recents directly into the target workspace", async () => {
    const repo = createTempRepo("ctx-recent-workspace-container-");
    cleanups.push(() => repo.cleanup());

    await openLauncher("local-container");
    await tauriInvoke("desktop_disconnect").catch(() => {});
    await tauriInvoke("desktop_connect_local");

    const createWorkspaceResp = await daemonJson("POST", "/api/workspaces", {
      root_path: repo.root,
      name: "Recent Container Workspace",
    });
    if (createWorkspaceResp.status !== 200 && createWorkspaceResp.status !== 201) {
      throw new Error(`workspace creation failed: ${JSON.stringify(createWorkspaceResp)}`);
    }
    const workspaceId = readWorkspaceId(createWorkspaceResp);

    const updateConfigResp = await daemonJson(
      "POST",
      `/api/workspaces/${workspaceId}/execution_config`,
      { environment: "sandbox" },
    );
    if (updateConfigResp.status !== 200) {
      throw new Error(`execution config update failed: ${JSON.stringify(updateConfigResp)}`);
    }

    await seedLauncherRecents([
      {
        kind: "local",
        label: "Recent Container Workspace",
        root_path: repo.root,
        execution_environment: "sandbox",
        updated_at_ms: Date.now(),
      },
    ]);

    await openLauncher("local-container-seeded");
    await clickLauncherRecent("Recent Container Workspace");
    await waitForWorkspaceRoute(workspaceId);
  });

  it("reopens managed staging sandbox recents with visible pending state", async () => {
    await openLauncher("local-staging-container");
    await tauriInvoke("desktop_disconnect").catch(() => {});
    await tauriInvoke("desktop_connect_local");

    const stagingRoot = await readRepoStagingPath();
    initGitRepo(stagingRoot);
    cleanups.push(() => {
      fs.rmSync(stagingRoot, { recursive: true, force: true });
    });

    const createWorkspaceResp = await daemonJson("POST", "/api/workspaces", {
      root_path: stagingRoot,
      name: "Managed Staging Sandbox",
    });
    if (createWorkspaceResp.status !== 200 && createWorkspaceResp.status !== 201) {
      throw new Error(`workspace creation failed: ${JSON.stringify(createWorkspaceResp)}`);
    }
    const workspaceId = readWorkspaceId(createWorkspaceResp);

    const updateConfigResp = await daemonJson(
      "POST",
      `/api/workspaces/${workspaceId}/execution_config`,
      { environment: "sandbox" },
    );
    if (updateConfigResp.status !== 200) {
      throw new Error(`execution config update failed: ${JSON.stringify(updateConfigResp)}`);
    }

    const recentLabel = path.basename(stagingRoot);
    await seedLauncherRecents([
      {
        kind: "local",
        label: recentLabel,
        root_path: stagingRoot,
        execution_environment: "sandbox",
        updated_at_ms: Date.now(),
      },
    ]);

    await openLauncher("local-staging-container-seeded");
    await clickLauncherRecent(recentLabel);
    await browser.waitUntil(
      async () => await browser.execute((expectedLabel) => {
        const buttons = Array.from(document.querySelectorAll(".launcher-recent-item"));
        const row = buttons.find((node) => String(node.textContent || "").includes(String(expectedLabel)));
        if (!(row instanceof HTMLElement)) return false;
        const status = row.querySelector(".launcher-recent-inline-status");
        if (!(status instanceof HTMLElement)) return false;
        const text = String(status.textContent || "");
        return (
          text.includes("Preparing sandbox")
          || text.includes("Restarting VM")
          || text.includes("Checking sandbox")
        );
      }, recentLabel),
      { timeout: 30000, timeoutMsg: "expected launcher pending state while reopening sandbox recent" },
    );
    const launcherError = await browser.execute(
      () => String(document.querySelector(".launcher-error")?.textContent || "").trim(),
    );
    if (launcherError === "500") {
      throw new Error("launcher surfaced a raw 500 while reopening a managed staging sandbox recent");
    }
    await waitForWorkspaceRoute(workspaceId);
  });

  it("opens remote recents directly into the target workspace when remote automation is configured", async function remoteRecentOpen() {
    if (!remoteFixture.ready) {
      this.skip();
      return;
    }

    const remoteWorkspaceRoot = ensureRemoteWorkspaceRepo(remoteFixture);
    cleanups.push(() => {
      try {
        remoteSsh(remoteFixture, `bash -lc ${JSON.stringify(`rm -rf ${remoteWorkspaceRoot}`)}`);
      } catch {
        // ignore remote cleanup failures
      }
    });

    await openLauncher("remote");
    await tauriInvoke("desktop_disconnect").catch(() => {});
    await connectSshWithPolling({
      host: remoteFixture.host,
      user: remoteFixture.user || null,
      remote_port: remoteFixture.port,
      start_remote: true,
      remote_data_dir: remoteFixture.dataDir || null,
      password_once: String(remoteFixture.authMode || "").trim().toLowerCase() === "password"
        ? (remoteFixture.passwordActual || remoteFixture.password || null)
        : null,
    });

    const createWorkspaceResp = await daemonJson("POST", "/api/workspaces", {
      root_path: remoteWorkspaceRoot,
      name: "Recent Remote Workspace",
    });
    if (createWorkspaceResp.status !== 200 && createWorkspaceResp.status !== 201) {
      throw new Error(`remote workspace creation failed: ${JSON.stringify(createWorkspaceResp)}`);
    }
    const workspaceId = readWorkspaceId(createWorkspaceResp);

    await seedLauncherRecents([
      {
        kind: "ssh",
        label: "Recent Remote Workspace",
        host: remoteFixture.host,
        user: remoteFixture.user || null,
        remote_port: remoteFixture.port,
        start_remote: true,
        remote_data_dir: remoteFixture.dataDir || null,
        workspace_root_path: remoteWorkspaceRoot,
        execution_environment: "host",
        updated_at_ms: Date.now(),
      },
    ]);

    await openLauncher("remote-seeded");
    await clickLauncherRecent("Recent Remote Workspace");
    await waitForWorkspaceRoute(workspaceId);
  });
});
