import test from "node:test";
import assert from "node:assert/strict";

import { resolveCapturePrompt } from "./demo_hn_mobile_capture.mjs";
import {
  buildPromptCharacters,
  buildFrameSequenceFfmpegArgs,
  findNewTaskRecord,
  resolvePlaybackPrompt,
  resolvePlaybackSessionArtifactPath,
  resolveTaskWorktreeRoot,
} from "./demo_hn_mobile_playback.mjs";
import { buildWrapperHtml } from "./demo_hn_mobile_record_artifact.mjs";
import {
  HN_PROXY_PREFIX,
  rewriteHackerNewsAttribute,
  rewriteHackerNewsHtml,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/hn_proxy.js";

test("resolveCapturePrompt prefers the capture prompt", () => {
  const fixture = {
    next_prompt: "visible prompt",
    capture_prompt: "bounded capture prompt",
  };
  assert.equal(resolveCapturePrompt(fixture, null), "bounded capture prompt");
  assert.equal(resolveCapturePrompt(fixture, "explicit prompt"), "explicit prompt");
});

test("resolvePlaybackPrompt uses the visible fixture prompt", () => {
  assert.equal(resolvePlaybackPrompt({ next_prompt: "visible prompt" }, null), "visible prompt");
  assert.equal(resolvePlaybackPrompt({ next_prompt: "visible prompt" }, "explicit prompt"), "explicit prompt");
});

test("resolvePlaybackSessionArtifactPath resolves fixture-relative artifacts", () => {
  const fixturePath = "/tmp/demo-fixtures/demo-hn-mobile-fixture.json";
  const fixture = { session_artifact_path: "demo-artifacts/hn-mobile-saved-stories.mp4" };
  assert.equal(
    resolvePlaybackSessionArtifactPath(fixturePath, fixture, null),
    "/tmp/demo-fixtures/demo-artifacts/hn-mobile-saved-stories.mp4",
  );
  assert.equal(
    resolvePlaybackSessionArtifactPath(fixturePath, fixture, "/tmp/custom.mp4"),
    "/tmp/custom.mp4",
  );
});

test("buildWrapperHtml renders a phone shell iframe", () => {
  const markup = buildWrapperHtml("http://127.0.0.1:4173");
  assert.match(markup, /class="phone"/);
  assert.match(markup, /iframe class="demo-phone-screen" src="http:\/\/127\.0\.0\.1:4173"/);
});

test("buildPromptCharacters preserves prompt text character order", () => {
  assert.deepEqual(
    buildPromptCharacters("Save it."),
    ["S", "a", "v", "e", " ", "i", "t", "."],
  );
});

test("rewriteHackerNewsAttribute keeps HN navigation inside the local proxy", () => {
  assert.equal(
    rewriteHackerNewsAttribute("href", "item?id=47470773"),
    `${HN_PROXY_PREFIX}/item?id=47470773`,
  );
  assert.equal(
    rewriteHackerNewsAttribute("src", "news.css?abc123"),
    "https://news.ycombinator.com/news.css?abc123",
  );
  assert.equal(
    rewriteHackerNewsAttribute("href", "https://example.com/story"),
    "https://example.com/story",
  );
});

test("rewriteHackerNewsHtml strips the upstream script and rewrites internal links", () => {
  const html = rewriteHackerNewsHtml(`
    <html>
      <head>
        <script src="hn.js?abc123"></script>
      </head>
      <body>
        <a href="item?id=47470773">Comments</a>
        <img src="y18.svg" />
      </body>
    </html>
  `);
  assert.doesNotMatch(html, /hn\.js/);
  assert.match(html, /href="\/proxy\/hn\/item\?id=47470773"/);
  assert.match(html, /src="https:\/\/news\.ycombinator\.com\/y18\.svg"/);
});

test("buildFrameSequenceFfmpegArgs encodes a png sequence into mp4", () => {
  assert.deepEqual(
    buildFrameSequenceFfmpegArgs("/tmp/demo-frames", 12, "/tmp/out.mp4"),
    [
      "-y",
      "-framerate", "12",
      "-i", "/tmp/demo-frames/frame-%05d.png",
      "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
      "-c:v", "libx264",
      "-pix_fmt", "yuv420p",
      "/tmp/out.mp4",
    ],
  );
});

test("resolveTaskWorktreeRoot joins the daemon worktree path", () => {
  assert.equal(
    resolveTaskWorktreeRoot("/tmp/ctx-daemon", "workspace-1", { primary_worktree_id: "worktree-9" }),
    "/tmp/ctx-daemon/worktrees/workspace-1/worktree-9",
  );
  assert.equal(resolveTaskWorktreeRoot("/tmp/ctx-daemon", "workspace-1", { primary_worktree_id: "" }), null);
  assert.equal(resolveTaskWorktreeRoot("/tmp/ctx-daemon", "workspace-1", null), null);
});

test("findNewTaskRecord ignores pre-seeded tasks when waiting for the live task", () => {
  const tasks = [
    { id: "task-old-1", title: "Older task" },
    { id: "task-old-2", title: "Another seeded task" },
    { id: "task-new", title: "Newly created task" },
  ];
  assert.deepEqual(findNewTaskRecord(tasks, ["task-old-1", "task-old-2"]), tasks[2]);
  assert.equal(findNewTaskRecord(tasks, ["task-old-1", "task-old-2", "task-new"]), null);
});
