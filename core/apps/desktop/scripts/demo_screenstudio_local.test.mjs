import test from "node:test";
import assert from "node:assert/strict";

import {
  buildScreenStudioProjectBundle,
  buildScreenStudioRecordConfig,
  extractDotenvVariable,
  findScreenStudioCaptureWindow,
  normalizeScreenStudioCropRect,
} from "./demo_screenstudio_local.mjs";
import { parseArgs as parseScreenStudioRecordArgs } from "./demo_hn_mobile_record_screenstudio_local.mjs";

test("normalizeScreenStudioCropRect rounds bounds into Screen Studio crop format", () => {
  assert.deepEqual(
    normalizeScreenStudioCropRect({ x: 0.4, y: 33.2, width: 1129.4, height: 855.1 }),
    { x: 0, y: 33, width: 1129, height: 855, yAxis: "topBasedIncreasingDownwards" },
  );
});

test("buildScreenStudioRecordConfig builds polyrecorder input and display channels", () => {
  assert.deepEqual(
    buildScreenStudioRecordConfig({
      outputDirectory: "/tmp/recording",
      displayId: 1,
      cropRect: { x: 0, y: 33, width: 1129, height: 855 },
    }),
    {
      logLevel: "debug",
      outputDirectory: "/tmp/recording",
      channels: [
        { type: "input", captureKeyStrokes: true },
        {
          type: "display",
          displayId: 1,
          excludeFinderDesktopIcons: true,
          cropRect: {
            x: 0,
            y: 33,
            width: 1129,
            height: 855,
            yAxis: "topBasedIncreasingDownwards",
          },
        },
      ],
    },
  );
});

test("buildScreenStudioProjectBundle mirrors the recording duration into the project range and scene", () => {
  const bundle = buildScreenStudioProjectBundle({
    projectName: "ctx demo",
    durationMs: 11233,
    projectId: "projectid1",
    sceneId: "sceneid001",
    sliceId: "sliceid001",
    now: new Date("2026-04-02T07:00:00.000Z"),
    version: "3.6.0-4214",
  });
  assert.equal(bundle.project.json.name, "ctx demo");
  assert.deepEqual(bundle.project.json.config.recordingRange, [0, 11233]);
  assert.equal(bundle.project.json.scenes[0].slices[0].sourceEndMs, 11233);
  assert.equal(bundle.meta.json.version, "3.6.0-4214");
  assert.deepEqual(bundle.markers, { json: [] });
});

test("findScreenStudioCaptureWindow requires exactly one titled match", () => {
  const windowInfo = findScreenStudioCaptureWindow(
    [
      { appName: "ctx-demo", title: "hn-mobile", bounds: { x: 10, y: 20, width: 300, height: 400 } },
      { appName: "iTerm2", title: "ctx (codex)", bounds: { x: 0, y: 0, width: 100, height: 100 } },
    ],
    "hn-mobile",
  );
  assert.equal(windowInfo.appName, "ctx-demo");
  assert.deepEqual(windowInfo.bounds, {
    x: 10,
    y: 20,
    width: 300,
    height: 400,
    yAxis: "topBasedIncreasingDownwards",
  });
  assert.throws(
    () => findScreenStudioCaptureWindow([{ title: "other", bounds: { x: 0, y: 0, width: 1, height: 1 } }], "hn-mobile"),
    /expected exactly one Screen Studio capture window/,
  );
});

test("extractDotenvVariable reads quoted Infisical values", () => {
  const dotenv = "CN_API_KEY='secret-value'\nTAURI_SIGNING_PRIVATE_KEY=\"other-value\"\n";
  assert.equal(extractDotenvVariable(dotenv, "CN_API_KEY"), "secret-value");
  assert.equal(extractDotenvVariable(dotenv, "TAURI_SIGNING_PRIVATE_KEY"), "other-value");
  assert.equal(extractDotenvVariable(dotenv, "MISSING"), null);
});

test("parseScreenStudioRecordArgs forwards playback args and derives a matching raw recording title", () => {
  const options = parseScreenStudioRecordArgs([
    "--output-project",
    "/tmp/custom-demo.screenstudio",
    "--daemon-data-dir",
    "/tmp/custom-daemon",
    "--window-title",
    "custom-window",
    "--ready-preroll-ms",
    "4500",
    "--sidebar-ready-timeout-ms",
    "9000",
    "--harness-label",
    "Codex",
  ]);
  assert.equal(options.outputProjectPath, "/tmp/custom-demo.screenstudio");
  assert.equal(options.outputTitle, "custom-demo");
  assert.match(options.rawRecordingDir, /custom-demo$/);
  assert.equal(options.daemonDataDir, "/tmp/custom-daemon");
  assert.equal(options.windowTitle, "custom-window");
  assert.equal(options.readyPrerollMs, 4500);
  assert.equal(options.sidebarReadyTimeoutMs, 9000);
  assert.deepEqual(options.playbackArgs, ["--harness-label", "Codex"]);
});
