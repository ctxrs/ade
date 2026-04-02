import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import net from "node:net";

import {
  bundleExecutablePaths,
  buildCnBackendEnv,
  buildCnDriverEnv,
  buildDemoDesktopConnectionPayload,
  buildDemoWorkbenchWindowPayload,
  buildPlaybackAppEnv,
  captureArtifactsPaneState,
  captureDiffPaneState,
  captureBrowserState,
  createCliAlias,
  desktopSyncCommand,
  ensureWorkbenchVisible,
  findBundledProviderRuntime,
  killProcesses,
  navigateBrowserToWorkspace,
  openArtifactsPane,
  openDiffPane,
  parseArgs,
  primeDemoDesktopConnection,
  primeDemoDesktopConnectionToWorkspace,
  resolveBackendPort,
  resolveDaemonBinaryPath,
  playbackBuildEnv,
  prepareAutomationAppForLaunch,
  requireWorkspacePackage,
  seedProviderRuntimeConfig,
  setArtifactsPanePlaybackHidden,
  setDiffPanePlaybackHidden,
  shouldRecycleSharedCrabNebulaBackend,
  submitComposerPrompt,
  waitForProcessReady,
  waitForArtifactsPane,
  waitForDiffPaneReady,
  waitForSessionTurnCompletion,
  waitForTcpPort,
} from "./demo_ping_pong_playback.mjs";

test("resolveBackendPort uses the shared macOS backend port", () => {
  if (process.platform === "darwin") {
    assert.equal(resolveBackendPort(39001), 3000);
    return;
  }
  assert.equal(resolveBackendPort(39001), 39001);
});

test("createCliAlias creates a symlink alias", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "demo-playback-alias-"));
  try {
    const aliasPath = createCliAlias("/tmp/fake-cli.js", "ctx-demo-cli", dir);
    assert.equal(readlinkSync(aliasPath), "/tmp/fake-cli.js");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("seedProviderRuntimeConfig writes a direct provider runtime command", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "demo-playback-provider-config-"));
  try {
    const configPath = seedProviderRuntimeConfig(dir, "codex", "/tmp/codex-crp");
    const config = JSON.parse(readFileSync(configPath, "utf8"));
    assert.equal(config.providers.codex.command, "/tmp/codex-crp");
    assert.deepEqual(config.providers.codex.args, []);
    assert.deepEqual(config.providers.codex.dependencies, []);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("findBundledProviderRuntime returns null when no local managed runtime exists", () => {
  assert.equal(findBundledProviderRuntime("missing-provider", "missing-binary"), null);
});

test("desktopSyncCommand points at the desktop resource sync script", () => {
  const [command, args] = desktopSyncCommand("debug");
  assert.equal(command, process.execPath);
  assert.deepEqual(args, ["core/scripts/desktop_sync_resources.cjs", "--profile", "debug"]);
});

test("bundleExecutablePaths targets the automation app binary", () => {
  const candidates = bundleExecutablePaths("/tmp/ctx-demo-target/debug/bundle/macos/ctx-demo.app");
  assert.ok(candidates.some((candidate) => candidate.endsWith("/ctx-demo.app/Contents/MacOS/ctx")));
  assert.ok(candidates.some((candidate) => candidate.endsWith("/ctx-demo.app/Contents/MacOS/ctx-demo")));
});

test("prepareAutomationAppForLaunch copies and rewrites the macOS automation bundle", () => {
  if (process.platform !== "darwin") {
    const sourcePath = "/tmp/ctx-demo-target/debug/bundle/macos/ctx-demo.app";
    assert.equal(prepareAutomationAppForLaunch(sourcePath, "/tmp/demo-artifacts"), sourcePath);
    return;
  }

  const dir = mkdtempSync(path.join(tmpdir(), "demo-playback-app-"));
  const sourceAppPath = path.join(dir, "ctx-demo.app");
  const infoPlistPath = path.join(sourceAppPath, "Contents/Info.plist");
  const executablePath = path.join(sourceAppPath, "Contents/MacOS/ctx");
  mkdirSync(path.dirname(infoPlistPath), { recursive: true });
  mkdirSync(path.dirname(executablePath), { recursive: true });
  writeFileSync(
    infoPlistPath,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>rs.ctx.desktop</string>
  <key>CFBundleName</key>
  <string>ctx</string>
  <key>CFBundleDisplayName</key>
  <string>ctx</string>
  <key>CFBundleExecutable</key>
  <string>ctx</string>
</dict>
  </plist>
`,
    "utf8",
  );
  writeFileSync(executablePath, "#!/bin/sh\n", "utf8");
  const launchAppPath = prepareAutomationAppForLaunch(sourceAppPath, path.join(dir, "artifacts"));
  assert.equal(launchAppPath, path.join(dir, "artifacts", "ctx-demo.app"));
  const plistDump = readFileSync(path.join(launchAppPath, "Contents/Info.plist"), "utf8");
  assert.match(plistDump, /rs\.ctx\.desktop\.demo/);
  assert.match(plistDump, /ctx-demo/);
  assert.match(plistDump, /ctx demo/);
  assert.equal(existsSync(path.join(launchAppPath, "Contents/MacOS/ctx")), false);
  assert.equal(existsSync(path.join(launchAppPath, "Contents/MacOS/ctx-demo")), true);
});

test("killProcesses terminates matching helper processes", async () => {
  const token = `demo-playback-kill-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  const proc = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)", token], {
    stdio: "ignore",
  });
  await new Promise((resolve) => setTimeout(resolve, 150));
  const killed = killProcesses((_pid, command) => command.includes(token));
  assert.ok(killed.includes(proc.pid));
  await new Promise((resolve) => proc.once("exit", resolve));
});

test("playbackBuildEnv keeps bundle sync disabled for demo builds", () => {
  const env = playbackBuildEnv("/tmp/demo-target");
  assert.equal(env.CARGO_TARGET_DIR, "/tmp/demo-target");
  assert.equal(env.CTX_DESKTOP_SYNC_BUNDLES, "0");
  assert.equal(env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD, "1");
});

test("resolveDaemonBinaryPath points at the bundled desktop daemon binary", () => {
  const expectedName = process.platform === "win32" ? "ctx-daemon.exe" : "ctx-daemon";
  assert.match(resolveDaemonBinaryPath("/tmp/demo-target"), new RegExp(`core/apps/desktop/src-tauri/bin/${expectedName}$`));
});

test("parseArgs derives appPath from an overridden tauriTargetDir", () => {
  const options = parseArgs(["--tauri-target-dir", "/tmp/demo-target-live"]);
  assert.equal(options.tauriTargetDir, "/tmp/demo-target-live");
  assert.equal(options.appPath, "/tmp/demo-target-live/debug/bundle/macos/ctx-demo.app");
});

test("buildPlaybackAppEnv opens directly to the seeded workspace route", () => {
  const env = buildPlaybackAppEnv(
    {
      daemon: { url: "http://127.0.0.1:4416", data_dir: "/tmp/demo-daemon" },
      fixture: { workspace_id: "ws-123" },
    },
    "/tmp/demo-workspace",
    "token-123",
    "/tmp/demo-target",
  );
  assert.equal(env.CTX_DESKTOP_DAEMON_URL, "http://127.0.0.1:4416");
  assert.equal(env.CTX_DESKTOP_DAEMON_TOKEN, "token-123");
  assert.equal(env.CTX_DESKTOP_ALLOW_DEMO_COMMANDS, "1");
  assert.equal("CTX_DESKTOP_START_WORKSPACE_PATHS" in env, false);
  assert.equal(env.CTX_DESKTOP_START_PATH, "/workspaces/ws-123?ctxE2E=1");
  assert.equal(env.CTX_AUTOMATION_WORKSPACE_PATH, "/tmp/demo-workspace");
  assert.equal(env.CTX_BUNDLE_DIR, "/tmp/demo-target/debug/bundles");
  assert.equal(env.CTX_DESKTOP_DEV_BIN_DIR, "/tmp/demo-target/debug");
});

test("parseArgs accepts an explicit relay scenario fixture", () => {
  const options = parseArgs(["--relay-scenario", "/tmp/demo-relay.json"]);
  assert.equal(options.relayScenarioPath, "/tmp/demo-relay.json");
});

test("CrabNebula backend env carries the playback app overrides", () => {
  const env = buildCnBackendEnv(
    {
      CTX_DESKTOP_DAEMON_URL: "http://127.0.0.1:4416",
      CTX_DESKTOP_START_PATH: "/workspaces/ws-123?ctxE2E=1",
    },
    3000,
  );
  assert.equal(env.TEST_RUNNER_BACKEND_PORT, "3000");
  assert.equal(env.CTX_DESKTOP_DAEMON_URL, "http://127.0.0.1:4416");
  assert.equal(env.CTX_DESKTOP_START_PATH, "/workspaces/ws-123?ctxE2E=1");
});

test("CrabNebula driver env carries backend routing and playback app overrides", () => {
  const env = buildCnDriverEnv(
    {
      CTX_DESKTOP_DAEMON_TOKEN: "token-123",
    },
    4444,
    "http://127.0.0.1:3000",
  );
  assert.equal(env.TAURI_DRIVER_PORT, "4444");
  assert.equal(env.REMOTE_WEBDRIVER_URL, "http://127.0.0.1:3000");
  assert.equal(env.CTX_DESKTOP_DAEMON_TOKEN, "token-123");
});

test("shared macOS CrabNebula backend is recycled only for known playback commands", () => {
  assert.equal(
    shouldRecycleSharedCrabNebulaBackend({
      platform: "darwin",
      backendPort: 3000,
      backendAlreadyListening: true,
      processEntries: [
        { pid: 1, command: "/opt/homebrew/bin/node /tmp/ctx-cnb-cli --host 127.0.0.1 --port 3000" },
      ],
    }),
    true,
  );
  assert.equal(
    shouldRecycleSharedCrabNebulaBackend({
      platform: "darwin",
      backendPort: 3000,
      backendAlreadyListening: true,
      processEntries: [
        { pid: 2, command: "/usr/bin/python3 -m http.server 3000" },
      ],
    }),
    false,
  );
});

test("requireWorkspacePackage fails with an install hint when dependency is missing", () => {
  assert.throws(
    () => requireWorkspacePackage("@ctx/definitely-missing-demo-package"),
    /Run `pnpm -C core install --frozen-lockfile` before running the demo playback/,
  );
});

test("buildDemoDesktopConnectionPayload seeds desktop daemon storage", () => {
  const payload = buildDemoDesktopConnectionPayload("http://127.0.0.1:4416", "token-123");
  const sessionConnection = JSON.parse(payload.sessionConnection);
  const persistedBase = JSON.parse(payload.persistedBase);
  assert.equal(sessionConnection.baseUrl, "http://127.0.0.1:4416");
  assert.equal(sessionConnection.wsBaseUrl, "ws://127.0.0.1:4416");
  assert.equal(sessionConnection.authToken, "token-123");
  assert.equal(sessionConnection.source, "desktop");
  assert.equal(persistedBase.baseUrl, "http://127.0.0.1:4416");
  assert.equal(persistedBase.wsBaseUrl, "ws://127.0.0.1:4416");
});

test("buildDemoWorkbenchWindowPayload seeds an active task tab for the fixture session", () => {
  const payload = buildDemoWorkbenchWindowPayload("ws-123", "task-456", "session-789", "demo-window");
  const sessionWindow = JSON.parse(payload.sessionWindow);
  assert.equal(payload.windowId, "demo-window");
  assert.equal(payload.windowName, "ctx-ui-window-id:demo-window");
  assert.equal(payload.sessionWindowKey, "wb.window.session.v1.ws-123.demo-window");
  assert.equal(sessionWindow.focusedLeafId, "ctx-demo-leaf");
  assert.equal(sessionWindow.layout.kind, "leaf");
  assert.equal(sessionWindow.layout.activeTabId, "ctx-demo-task-tab");
  assert.deepEqual(sessionWindow.layout.tabs, [
    {
      id: "ctx-demo-task-tab",
      kind: "task",
      ref: {
        taskId: "task-456",
        sessionId: "session-789",
      },
    },
  ]);
});

test("waitForProcessReady rejects when process exits before readiness", async () => {
  const proc = spawn(process.execPath, ["-e", "setTimeout(() => process.exit(3), 30)"], {
    stdio: "ignore",
  });
  await assert.rejects(
    () =>
      waitForProcessReady({
        proc,
        name: "fixture-proc",
        readyPromise: new Promise(() => {}),
        detail: "detail marker",
      }),
    /fixture-proc exited before ready .*detail marker/,
  );
});

test("waitForTcpPort resolves once a port starts listening", async () => {
  const server = net.createServer();
  const port = await new Promise((resolve, reject) => {
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address !== "object") {
        reject(new Error("missing address"));
        return;
      }
      resolve(address.port);
    });
    server.once("error", reject);
  });
  try {
    await waitForTcpPort("127.0.0.1", port, 2_000, "test tcp port");
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

test("captureBrowserState reads basic browser diagnostics", async () => {
  const browser = {
    execute: async () => ({
      href: "tauri://localhost/workspaces/ws-1",
      pathname: "/workspaces/ws-1",
      search: "",
      title: "ctx",
      readyState: "complete",
      bodyClassName: "ready",
      rootSnippet: "<div class=\"wb-root\"></div>",
      bodySnippet: "<div id=\"root\"><div class=\"wb-root\"></div></div>",
    }),
  };
  const state = await captureBrowserState(browser);
  assert.equal(state.pathname, "/workspaces/ws-1");
  assert.match(state.rootSnippet, /wb-root/);
});

test("navigateBrowserToWorkspace forces the seeded workbench route", async () => {
  let currentPath = "/";
  const browser = {
    execute: async (_fn, pathArg) => {
      if (typeof pathArg === "string") {
        currentPath = pathArg;
        return undefined;
      }
      return {
        href: `tauri://localhost${currentPath}`,
        pathname: currentPath.split("?")[0],
        search: currentPath.includes("?") ? `?${currentPath.split("?")[1]}` : "",
        title: "ctx",
        readyState: "complete",
        bodyClassName: "",
        rootSnippet: "",
        bodySnippet: "",
      };
    },
    waitUntil: async (predicate) => {
      const ok = await predicate();
      if (!ok) {
        throw new Error("navigation wait failed");
      }
    },
  };
  await navigateBrowserToWorkspace(browser, "ws-123");
  assert.equal(currentPath, "/workspaces/ws-123");
});

test("primeDemoDesktopConnection seeds desktop and workbench state before targeting the workspace route", async () => {
  const calls = [];
  let currentHref = "/";
  let focusEnabled = false;
  let focusAttempts = 0;
  const browser = {
    execute: async (fn, payload) => {
      calls.push(payload);
      if (typeof payload === "string") {
        currentHref = payload;
        focusEnabled = true;
        return undefined;
      }
      if (payload?.nextPath) {
        currentHref = payload.nextPath;
        focusEnabled = true;
        return undefined;
      }
      if (payload?.taskId) {
        if (!focusEnabled) return false;
        focusAttempts += 1;
        return focusAttempts >= 2;
      }
      if (payload === undefined && String(fn).includes("__ctxE2E")) {
        return focusEnabled;
      }
      const url = new URL(`tauri://localhost${currentHref}`);
      return {
        href: url.href,
        pathname: url.pathname,
        search: url.search,
        title: "ctx",
        readyState: "complete",
        bodyClassName: "",
        rootSnippet: "",
        bodySnippet: "",
      };
    },
    waitUntil: async (predicate) => {
      for (let index = 0; index < 3; index += 1) {
        if (await predicate()) {
          return;
        }
      }
      throw new Error("waitUntil exhausted");
    },
  };
  await primeDemoDesktopConnection(browser, "http://127.0.0.1:4416", "token-123", "ws-123", "task-456", "session-789");
  const payloadCalls = calls.filter((value) => value && typeof value === "object");
  assert.deepEqual(payloadCalls[0], {
    nextBaseUrl: "http://127.0.0.1:4416",
    nextToken: "token-123",
    storage: buildDemoDesktopConnectionPayload("http://127.0.0.1:4416", "token-123"),
  });
  assert.deepEqual(payloadCalls[1], buildDemoWorkbenchWindowPayload("ws-123", "task-456", "session-789"));
  assert.deepEqual(payloadCalls[2], {
    taskId: "task-456",
    sessionId: "session-789",
  });
  assert.deepEqual(payloadCalls[3], {
    taskId: "task-456",
    sessionId: "session-789",
  });
  assert.equal(currentHref, "/workspaces/ws-123");
  assert.equal(focusAttempts, 2);
});

test("submitComposerPrompt syncs the composer value and clicks send", async () => {
  let receivedText = null;
  const browser = {
    execute: async (_fn, text) => {
      receivedText = text;
      return {
        value: text,
        send_disabled: false,
      };
    },
  };
  const result = await submitComposerPrompt(browser, "Make a ping pong game.");
  assert.equal(receivedText, "Make a ping pong game.");
  assert.deepEqual(result, {
    value: "Make a ping pong game.",
    send_disabled: false,
  });
});

test("waitForSessionTurnCompletion accepts wrapped event payloads", async () => {
  const completed = await waitForSessionTurnCompletion("http://127.0.0.1:4416", "token-123", "session-123", {
    apiImpl: async () => ({
      session_id: "session-123",
      events: [
        { event_type: "assistant_message_inserted", payload_json: {} },
        { event_type: "turn_finished", payload_json: { status: "completed" }, seq: 42 },
      ],
    }),
    timeoutMs: 50,
    intervalMs: 1,
  });
  assert.equal(completed.seq, 42);
});

test("waitForSessionTurnCompletion ignores earlier completed turns", async () => {
  let callCount = 0;
  const completed = await waitForSessionTurnCompletion("http://127.0.0.1:4416", "token-123", "session-123", {
    apiImpl: async () => {
      callCount += 1;
      if (callCount === 1) {
        return {
          session_id: "session-123",
          events: [
            { seq: 10, event_type: "turn_finished", payload_json: { status: "completed" } },
          ],
        };
      }
      return {
        session_id: "session-123",
        events: [
          { seq: 10, event_type: "turn_finished", payload_json: { status: "completed" } },
          { seq: 14, event_type: "turn_finished", payload_json: { status: "completed" } },
        ],
      };
    },
    minSeqExclusive: 10,
    timeoutMs: 50,
    intervalMs: 1,
  });
  assert.equal(completed.seq, 14);
});

test("openDiffPane invokes the ctxE2E bridge", async () => {
  let bridgeChecked = false;
  let executeCount = 0;
  const browser = {
    waitUntil: async (predicate) => {
      bridgeChecked = await predicate();
      if (!bridgeChecked) {
        throw new Error("bridge unavailable");
      }
    },
    execute: async () => {
      executeCount += 1;
      if (executeCount === 1) {
        return true;
      }
      return {
        invoked: true,
        before: { diff_open: false, right_pane_count: 0 },
        after: { diff_open: true, right_pane_count: 1 },
      };
    },
  };
  const result = await openDiffPane(browser);
  assert.equal(bridgeChecked, true);
  assert.deepEqual(result, {
    invoked: true,
    before: { diff_open: false, right_pane_count: 0 },
    after: { diff_open: true, right_pane_count: 1 },
  });
});

test("openArtifactsPane invokes the ctxE2E bridge", async () => {
  let bridgeChecked = false;
  let executeCount = 0;
  const browser = {
    waitUntil: async (predicate) => {
      bridgeChecked = await predicate();
      if (!bridgeChecked) {
        throw new Error("bridge unavailable");
      }
    },
    execute: async () => {
      executeCount += 1;
      if (executeCount === 1) {
        return true;
      }
      return {
        invoked: true,
        before: { artifact_card_count: 0, video_count: 0 },
        after: { artifact_card_count: 1, video_count: 1 },
      };
    },
  };
  const result = await openArtifactsPane(browser);
  assert.equal(bridgeChecked, true);
  assert.deepEqual(result, {
    invoked: true,
    before: { artifact_card_count: 0, video_count: 0 },
    after: { artifact_card_count: 1, video_count: 1 },
  });
});

test("captureDiffPaneState reads the diff pane markers", async () => {
  const browser = {
    execute: async () => ({
      hasDiffPaneClass: true,
      hasDiffContent: true,
      rightPaneCount: 1,
      togglePressed: true,
      loadingChangesVisible: false,
      loadingChangedFilesVisible: false,
      loadingDiffVisible: false,
      parsingDiffVisible: false,
      fileRowCount: 3,
      openFileCount: 0,
      diffSummaryCount: 3,
      statusSummaryCount: 0,
      editorShellCount: 0,
      monacoEditorCount: 0,
      hiddenByPlayback: false,
    }),
  };
  const result = await captureDiffPaneState(browser);
  assert.deepEqual(result, {
    hasDiffPaneClass: true,
    hasDiffContent: true,
    rightPaneCount: 1,
    togglePressed: true,
    loadingChangesVisible: false,
    loadingChangedFilesVisible: false,
    loadingDiffVisible: false,
    parsingDiffVisible: false,
    fileRowCount: 3,
    openFileCount: 0,
    diffSummaryCount: 3,
    statusSummaryCount: 0,
    editorShellCount: 0,
    monacoEditorCount: 0,
    hiddenByPlayback: false,
  });
});

test("waitForDiffPaneReady waits for stable parsed summaries", async () => {
  let attempts = 0;
  const browser = {
    waitUntil: async (predicate) => {
      for (let i = 0; i < 6; i += 1) {
        attempts += 1;
        if (await predicate()) {
          return true;
        }
      }
      throw new Error("diff pane never stabilized");
    },
    execute: async () => {
      if (attempts <= 1) {
        return {
          hasDiffPaneClass: true,
          hasDiffContent: true,
          rightPaneCount: 1,
          togglePressed: true,
          loadingChangesVisible: false,
          loadingChangedFilesVisible: false,
          loadingDiffVisible: false,
          parsingDiffVisible: false,
          fileRowCount: 3,
          openFileCount: 0,
          diffSummaryCount: 0,
          statusSummaryCount: 3,
          editorShellCount: 0,
          monacoEditorCount: 0,
          hiddenByPlayback: true,
        };
      }
      return {
        hasDiffPaneClass: true,
        hasDiffContent: true,
        rightPaneCount: 1,
        togglePressed: true,
        loadingChangesVisible: false,
        loadingChangedFilesVisible: false,
        loadingDiffVisible: false,
        parsingDiffVisible: false,
        fileRowCount: 3,
        openFileCount: 0,
        diffSummaryCount: 3,
        statusSummaryCount: 0,
        editorShellCount: 0,
        monacoEditorCount: 0,
        hiddenByPlayback: true,
      };
    },
  };
  await waitForDiffPaneReady(browser);
  assert.ok(attempts >= 3);
});

test("setDiffPanePlaybackHidden proxies the requested hidden state", async () => {
  const browser = {
    execute: async (_fn, hidden) => Boolean(hidden),
  };
  assert.equal(await setDiffPanePlaybackHidden(browser, true), true);
  assert.equal(await setDiffPanePlaybackHidden(browser, false), false);
});

test("setArtifactsPanePlaybackHidden proxies the requested hidden state", async () => {
  const browser = {
    execute: async (_fn, hidden) => Boolean(hidden),
  };
  assert.equal(await setArtifactsPanePlaybackHidden(browser, true), true);
  assert.equal(await setArtifactsPanePlaybackHidden(browser, false), false);
});

test("captureArtifactsPaneState reads the artifact pane markers", async () => {
  const browser = {
    execute: async () => ({
      hasArtifactCard: true,
      artifactCardCount: 1,
      hasVideoPreview: true,
      videoCount: 1,
      playingVideoCount: 1,
      firstVideoCurrentTime: 1.25,
      firstVideoDuration: 11.233,
      firstVideoPaused: false,
      firstVideoEnded: false,
      firstVideoControls: false,
      firstVideoLoop: false,
      hasEmptyState: false,
      rightPaneCount: 1,
      hiddenByPlayback: false,
    }),
  };
  const result = await captureArtifactsPaneState(browser);
  assert.deepEqual(result, {
    hasArtifactCard: true,
    artifactCardCount: 1,
    hasVideoPreview: true,
    videoCount: 1,
    playingVideoCount: 1,
    firstVideoCurrentTime: 1.25,
    firstVideoDuration: 11.233,
    firstVideoPaused: false,
    firstVideoEnded: false,
    firstVideoControls: false,
    firstVideoLoop: false,
    hasEmptyState: false,
    rightPaneCount: 1,
    hiddenByPlayback: false,
  });
});

test("waitForArtifactsPane resolves when the artifact pane is ready", async () => {
  let attempts = 0;
  const browser = {
    waitUntil: async (predicate) => {
      for (let i = 0; i < 3; i += 1) {
        attempts += 1;
        if (await predicate()) {
          return true;
        }
      }
      throw new Error("artifacts pane never opened");
    },
    execute: async () => ({
      hasArtifactCard: attempts >= 2,
      artifactCardCount: attempts >= 2 ? 1 : 0,
      hasVideoPreview: false,
      videoCount: 0,
      hasEmptyState: false,
      rightPaneCount: 1,
    }),
  };
  await waitForArtifactsPane(browser);
  assert.ok(attempts >= 2);
});

test("ensureWorkbenchVisible writes a browser-state artifact when selector wait fails", async () => {
  const artifactDir = mkdtempSync(path.join(tmpdir(), "demo-playback-artifacts-"));
  const browser = {
    waitUntil: async () => {
      throw new Error("selector not found");
    },
    execute: async () => ({
      href: "tauri://localhost/",
      pathname: "/",
      search: "",
      title: "ctx",
      readyState: "complete",
      bodyClassName: "launcher",
      rootSnippet: "<div class=\"launcher-shell\"></div>",
      bodySnippet: "<div id=\"root\"><div class=\"launcher-shell\"></div></div>",
    }),
  };
  try {
    await assert.rejects(() => ensureWorkbenchVisible(browser, artifactDir), /selector not found/);
    const artifactPath = path.join(artifactDir, "browser-state-on-workbench-failure.json");
    const artifact = JSON.parse(readFileSync(artifactPath, "utf8"));
    assert.equal(typeof artifact, "object");
    assert.equal(artifact.capture_error, undefined);
  } finally {
    rmSync(artifactDir, { recursive: true, force: true });
  }
});

test("ensureWorkbenchVisible accepts a ready workbench shell with the E2E bridge", async () => {
  const artifactDir = mkdtempSync(path.join(tmpdir(), "demo-playback-artifacts-"));
  let waitUntilCount = 0;
  const browser = {
    waitUntil: async (predicate) => {
      waitUntilCount += 1;
      const ready = await predicate();
      if (!ready) {
        throw new Error(`waitUntil failed at step ${waitUntilCount}`);
      }
    },
    execute: async (_fn, selector) => {
      if (typeof selector === "string") {
        return selector === ".wb-root";
      }
      return true;
    },
  };
  try {
    await assert.doesNotReject(() => ensureWorkbenchVisible(browser, artifactDir));
    assert.equal(waitUntilCount, 2);
  } finally {
    rmSync(artifactDir, { recursive: true, force: true });
  }
});
