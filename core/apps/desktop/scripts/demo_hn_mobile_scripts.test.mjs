import test from "node:test";
import assert from "node:assert/strict";

import { resolveCapturePrompt } from "./demo_hn_mobile_capture.mjs";
import { inferVideoArtifactMimeType } from "./demo_video_artifacts.mjs";
import { buildCompositeFfmpegArgs, parseScreenBox } from "./demo_hn_mobile_composite_artifact.mjs";
import {
  buildRemoteRecordCommand,
  resolveLocalVideoPlan,
} from "./demo_hn_mobile_record_simulator_artifact.mjs";
import {
  buildPromptCharacters,
  buildFrameSequenceFfmpegArgs,
  findNewTaskRecord,
  resolvePlaybackPrompt,
  resolvePlaybackSessionArtifactPath,
  resolveTaskWorktreeRoot,
} from "./demo_hn_mobile_playback.mjs";
import { buildWrapperHtml } from "./demo_hn_mobile_record_artifact.mjs";
import { buildRawCaptureUrl } from "./demo_hn_mobile_record_raw_artifact.mjs";
import { resolveDefaultPagePath } from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/src/app.js";
import {
  DEFAULT_HN_DEMO_SCENARIO_ID,
  HN_DEMO_SCENARIO_QUERY_KEY,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/demo_scenarios.js";
import {
  buildDemoProfileHtml,
  buildHackerNewsTargetUrl,
  HN_PROXY_PREFIX,
  injectDemoFrontPageAccountNav,
  injectDemoFrontPageStories,
  rewriteHackerNewsAttribute,
  rewriteHackerNewsHtml,
  rewriteHackerNewsLocation,
  rewriteHackerNewsSetCookie,
  shouldMockDemoProfilePage,
  shouldMockDemoFrontPage,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/hn_proxy.js";
import {
  DEFAULT_DEMO_AUTOPLAY_ID,
  DEMO_AUTOPLAY_PRESET_QUERY_KEY,
  DEMO_AUTOPLAY_QUERY_KEY,
  resolveDemoAutoplayOptions,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/src/demo_autoplay.js";
import {
  buildDemoLoginFields,
  buildDemoLoginGoto,
  buildFeedPreviewPath,
  buildMutedDomainsToastMessage,
  buildMutedDomainsSettingsRowMarkup,
  domainsMatch,
  normalizeDomain,
  parseMutedDomains,
  hasLoginLink,
  readDemoLoginCredentials,
} from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/src/hn_enhancer.js";
import { resolveScreenHeight } from "../automation/fixtures/demo-workspaces/hn-mobile-baseline/src/viewport.js";

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
  const fixture = { session_artifact_path: "demo-artifacts/hn-muted-domains-apple-official-mock.mov" };
  assert.equal(
    resolvePlaybackSessionArtifactPath(fixturePath, fixture, null),
    "/tmp/demo-fixtures/demo-artifacts/hn-muted-domains-apple-official-mock.mov",
  );
  assert.equal(
    resolvePlaybackSessionArtifactPath(fixturePath, fixture, "/tmp/custom.mp4"),
    "/tmp/custom.mp4",
  );
});

test("inferVideoArtifactMimeType maps demo video containers to browser MIME types", () => {
  assert.equal(inferVideoArtifactMimeType("/tmp/demo.mp4"), "video/mp4");
  assert.equal(inferVideoArtifactMimeType("/tmp/demo.mov"), "video/quicktime");
  assert.equal(inferVideoArtifactMimeType("/tmp/demo.webm"), "video/webm");
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

test("buildRawCaptureUrl seeds the default app route with the requested scenario", () => {
  assert.equal(
    buildRawCaptureUrl("http://127.0.0.1:4173", DEFAULT_HN_DEMO_SCENARIO_ID),
    `http://127.0.0.1:4173/?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
});

test("resolveDemoAutoplayOptions only enables the known muted-domains autoplay mode", () => {
  assert.deepEqual(
    resolveDemoAutoplayOptions(`?${DEMO_AUTOPLAY_QUERY_KEY}=${DEFAULT_DEMO_AUTOPLAY_ID}&${DEMO_AUTOPLAY_PRESET_QUERY_KEY}=recording`),
    {
      id: DEFAULT_DEMO_AUTOPLAY_ID,
      presetId: "recording",
      mutedDomainsValue: "x.com\ntwitter.com",
      requireHiddenResume: false,
      initialDwellMs: 1400,
      settingsDwellMs: 950,
      perCharacterMs: 80,
      afterTypingMs: 800,
      afterSaveMs: 1100,
      finalDwellMs: 2700,
    },
  );
  assert.equal(resolveDemoAutoplayOptions(""), null);
  assert.throws(
    () => resolveDemoAutoplayOptions(`?${DEMO_AUTOPLAY_QUERY_KEY}=${DEFAULT_DEMO_AUTOPLAY_ID}&${DEMO_AUTOPLAY_PRESET_QUERY_KEY}=bogus`),
    /Unknown demo autoplay preset/,
  );
});

test("rewriteHackerNewsAttribute keeps HN navigation inside the local proxy", () => {
  assert.equal(
    rewriteHackerNewsAttribute("href", "item?id=47470773"),
    `${HN_PROXY_PREFIX}/item?id=47470773`,
  );
  assert.equal(
    rewriteHackerNewsAttribute("action", "login?goto=news"),
    `${HN_PROXY_PREFIX}/login?goto=news`,
  );
  assert.equal(
    rewriteHackerNewsAttribute("href", "user?id=ADE_TEST_ACCOUNT", { scenarioId: DEFAULT_HN_DEMO_SCENARIO_ID }),
    `${HN_PROXY_PREFIX}/user?id=ADE_TEST_ACCOUNT&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
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

test("rewriteHackerNewsHtml strips the upstream script, injects a mobile viewport, and rewrites internal links", () => {
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
  assert.match(html, /name="viewport"/);
  assert.doesNotMatch(html, /ctx-hn-mobile-viewport-fill/);
  assert.match(html, /href="\/proxy\/hn\/item\?id=47470773"/);
  assert.match(html, /src="https:\/\/news\.ycombinator\.com\/y18\.svg"/);

  const htmlWithScenario = rewriteHackerNewsHtml(
    `<html><head></head><body><a href="user?id=ADE_TEST_ACCOUNT">Profile</a></body></html>`,
    { scenarioId: DEFAULT_HN_DEMO_SCENARIO_ID },
  );
  assert.match(
    htmlWithScenario,
    /href="\/proxy\/hn\/user\?id=ADE_TEST_ACCOUNT&amp;?ctxDemoScenario=muted-domains|href="\/proxy\/hn\/user\?id=ADE_TEST_ACCOUNT&ctxDemoScenario=muted-domains"/,
  );
});

test("injectDemoFrontPageStories replaces the top two front-page stories", () => {
  const html = [
    '<tr class="athing submission" id="1"><td align="right" valign="top" class="title"><span class="rank">1.</span></td><td valign="top" class="votelinks"></td><td class="title"><span class="titleline"><a href="https://example.com/1">Story 1</a><span class="sitebit comhead"> (<a href="from?site=example.com"><span class="sitestr">example.com</span></a>)</span></span></td></tr><tr><td colspan="2"></td><td class="subtext"><span class="subline"><span class="score" id="score_1">10 points</span> by <a href="user?id=alice" class="hnuser">alice</a> <span class="age"><a href="item?id=1">1 hour ago</a></span> | <a href="item?id=1">1&nbsp;comments</a></span></td></tr><tr class="spacer" style="height:5px"></tr>',
    '<tr class="athing submission" id="2"><td align="right" valign="top" class="title"><span class="rank">2.</span></td><td valign="top" class="votelinks"></td><td class="title"><span class="titleline"><a href="https://example.com/2">Story 2</a><span class="sitebit comhead"> (<a href="from?site=example.com"><span class="sitestr">example.com</span></a>)</span></span></td></tr><tr><td colspan="2"></td><td class="subtext"><span class="subline"><span class="score" id="score_2">20 points</span> by <a href="user?id=bob" class="hnuser">bob</a> <span class="age"><a href="item?id=2">2 hours ago</a></span> | <a href="item?id=2">2&nbsp;comments</a></span></td></tr><tr class="spacer" style="height:5px"></tr>',
    '<tr class="athing submission" id="3"><td align="right" valign="top" class="title"><span class="rank">3.</span></td><td valign="top" class="votelinks"></td><td class="title"><span class="titleline"><a href="https://example.com/3">Story 3</a><span class="sitebit comhead"> (<a href="from?site=example.com"><span class="sitestr">example.com</span></a>)</span></span></td></tr><tr><td colspan="2"></td><td class="subtext"><span class="subline"><span class="score" id="score_3">30 points</span> by <a href="user?id=carol" class="hnuser">carol</a> <span class="age"><a href="item?id=3">3 hours ago</a></span> | <a href="item?id=3">3&nbsp;comments</a></span></td></tr><tr class="spacer" style="height:5px"></tr>',
  ].join("");

  const injected = injectDemoFrontPageStories(html);
  assert.doesNotMatch(injected, /Story 1/);
  assert.doesNotMatch(injected, /Story 2/);
  assert.match(injected, /Story 3/);
  assert.match(injected, /Claude Code's source code has been leaked via a map file in their NPM registry/);
  assert.match(injected, /You can now run a full Linux operating system inside a 6mb PDF/);
  assert.match(injected, /<span class="rank">3\.<\/span>/);
});

test("injectDemoFrontPageAccountNav swaps the login nav for the demo account", () => {
  const html = injectDemoFrontPageAccountNav(`
    <table><tr><td style="text-align:right;padding-right:4px;"><span class="pagetop"><a href="login">login</a></span></td></tr></table>
  `, DEFAULT_HN_DEMO_SCENARIO_ID);
  assert.match(html, /ADE_TEST_ACCOUNT/);
  assert.match(html, /logout/);
});

test("rewriteHackerNewsLocation keeps HN redirects inside the local proxy", () => {
  assert.equal(rewriteHackerNewsLocation("news"), `${HN_PROXY_PREFIX}/news`);
  assert.equal(
    rewriteHackerNewsLocation("https://example.com/story"),
    "https://example.com/story",
  );
});

test("rewriteHackerNewsSetCookie strips remote-domain attributes for local proxy use", () => {
  assert.equal(
    rewriteHackerNewsSetCookie("user=abc123; Domain=news.ycombinator.com; Path=/; Secure; HttpOnly"),
    "user=abc123; Path=/; HttpOnly",
  );
});

test("buildHackerNewsTargetUrl strips the local demo-front-page flag", () => {
  assert.equal(
    buildHackerNewsTargetUrl(`${HN_PROXY_PREFIX}/news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`).href,
    "https://news.ycombinator.com/news",
  );
});

test("shouldMockDemoFrontPage only targets the front-page request", () => {
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`), true);
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/news?p=2&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`), false);
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/item?id=1&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`), false);
});

test("shouldMockDemoProfilePage only targets the ADE_TEST_ACCOUNT demo profile", () => {
  assert.equal(shouldMockDemoProfilePage(`${HN_PROXY_PREFIX}/user?id=ADE_TEST_ACCOUNT&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`), true);
  assert.equal(shouldMockDemoProfilePage(`${HN_PROXY_PREFIX}/user?id=pg&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`), false);
  assert.equal(shouldMockDemoProfilePage(`${HN_PROXY_PREFIX}/user?id=ADE_TEST_ACCOUNT`), false);
});

test("buildDemoProfileHtml returns the real-profile-shaped demo markup", () => {
  const html = rewriteHackerNewsHtml(buildDemoProfileHtml());
  assert.match(html, /Profile: ADE_TEST_ACCOUNT \| Hacker News/);
  assert.match(html, /name="email"/);
  assert.match(html, /showdead:/);
  assert.match(html, /value="update"/);
  assert.match(html, /href="\/proxy\/hn\/favorites\?id=ADE_TEST_ACCOUNT"/);
  assert.match(html, /id="bigbox"/);
});

test("normalizeDomain trims protocols, paths, and www prefixes", () => {
  assert.equal(normalizeDomain("https://www.x.com/some/path"), "x.com");
  assert.equal(normalizeDomain("twitter.com"), "twitter.com");
});

test("parseMutedDomains returns unique normalized domain entries", () => {
  assert.deepEqual(
    parseMutedDomains("x.com\nwww.x.com\ntwitter.com, x.com"),
    ["x.com", "twitter.com"],
  );
});

test("domainsMatch treats x.com and twitter.com as the same muted family", () => {
  assert.equal(domainsMatch("x.com", ["twitter.com"]), true);
  assert.equal(domainsMatch("mobile.twitter.com", ["x.com"]), true);
  assert.equal(domainsMatch("github.com", ["x.com"]), false);
});

test("buildMutedDomainsSettingsRowMarkup only renders the muted domains textarea row", () => {
  const markup = buildMutedDomainsSettingsRowMarkup();
  assert.match(markup, /muted domains:/);
  assert.match(markup, /data-muted-domains-input="true"/);
  assert.doesNotMatch(markup, /Stored only on this device/);
  assert.doesNotMatch(markup, /One domain per line/);
  assert.doesNotMatch(markup, /Preview front page/);
});

test("buildFeedPreviewPath keeps the demo front-page flag on the feed link", () => {
  assert.equal(
    buildFeedPreviewPath("?demoSeedMutedStories=1"),
    `/proxy/hn/news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
  assert.equal(
    buildFeedPreviewPath(""),
    `/proxy/hn/news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
});

test("buildMutedDomainsToastMessage uses the shortened copy", () => {
  assert.equal(buildMutedDomainsToastMessage(1), "Hid 1 story from muted domains.");
  assert.equal(buildMutedDomainsToastMessage(2), "Hid 2 stories from muted domains.");
});

test("buildDemoLoginGoto preserves the demo front-page flag through login", () => {
  assert.equal(
    buildDemoLoginGoto("?demoSeedMutedStories=1"),
    `news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
  assert.equal(
    buildDemoLoginGoto(""),
    `news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
});

test("buildDemoLoginFields prepares proxied HN login fields", () => {
  assert.deepEqual(
    buildDemoLoginFields("?demoLoginUser=ADE_TEST_ACCOUNT&demoLoginPassword=secret&demoSeedMutedStories=1"),
    {
      acct: "ADE_TEST_ACCOUNT",
      pw: "secret",
      goto: `news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
    },
  );
  assert.equal(buildDemoLoginFields(""), null);
});

test("resolveScreenHeight prefers the tallest available viewport metric", () => {
  assert.equal(
    resolveScreenHeight({ screenHeight: 852, viewportHeight: 818, innerHeight: 818 }),
    852,
  );
  assert.equal(
    resolveScreenHeight({ screenHeight: 0, viewportHeight: 812.6, innerHeight: 812 }),
    812.6,
  );
  assert.equal(resolveScreenHeight({}), 0);
});

test("resolveDefaultPagePath accepts an explicit proxied demo path", () => {
  assert.equal(
    resolveDefaultPagePath(`?demoPath=${encodeURIComponent(`/proxy/hn/user?id=ADE_TEST_ACCOUNT&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`)}`),
    `/proxy/hn/user?id=ADE_TEST_ACCOUNT&${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
  assert.equal(
    resolveDefaultPagePath(`?demoPath=${encodeURIComponent("https://example.com")}`),
    `/proxy/hn/news?${HN_DEMO_SCENARIO_QUERY_KEY}=${DEFAULT_HN_DEMO_SCENARIO_ID}`,
  );
});

test("readDemoLoginCredentials returns credentials only when both query params exist", () => {
  assert.deepEqual(
    readDemoLoginCredentials("?demoLoginUser=ADE_TEST_ACCOUNT&demoLoginPassword=ADE_TEST_PASSWORD"),
    { account: "ADE_TEST_ACCOUNT", password: "ADE_TEST_PASSWORD" },
  );
  assert.equal(readDemoLoginCredentials("?demoLoginUser=ADE_TEST_ACCOUNT"), null);
});

test("hasLoginLink detects proxied HN login links", () => {
  const doc = {
    querySelectorAll() {
      return [
        {
          getAttribute(name) {
            return name === "href" ? "/proxy/hn/login?goto=news" : null;
          },
          textContent: "login",
        },
      ];
    },
  };

  assert.equal(hasLoginLink(doc), true);
  assert.equal(
    hasLoginLink({
      querySelectorAll() {
        return [];
      },
    }),
    false,
  );
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

test("parseScreenBox accepts x,y,width,height coordinates", () => {
  assert.deepEqual(parseScreenBox("196,288,1206,2623"), {
    x: 196,
    y: 288,
    width: 1206,
    height: 2623,
  });
  assert.throws(() => parseScreenBox("196,288,1206"), /invalid screen box/);
});

test("buildCompositeFfmpegArgs composes a raw artifact inside the overlay mask", () => {
  assert.deepEqual(
    buildCompositeFfmpegArgs({
      inputPath: "/tmp/raw.mp4",
      overlayPath: "/tmp/overlay.png",
      maskPath: "/tmp/mask.png",
      outputPath: "/tmp/out.mp4",
      screenBox: { x: 196, y: 288, width: 1206, height: 2623 },
      canvasSize: { width: 1600, height: 3200 },
      durationSeconds: 2.111,
    }),
    [
      "-y",
      "-i", "/tmp/raw.mp4",
      "-loop", "1",
      "-i", "/tmp/mask.png",
      "-loop", "1",
      "-i", "/tmp/overlay.png",
      "-filter_complex",
      "[0:v]scale=1206:2623:force_original_aspect_ratio=increase,crop=1206:2623[screen];[screen]format=rgba[screen_rgba];color=color=black@0.0:size=1600x3200[base];[base][screen_rgba]overlay=196:288[canvas];[canvas]format=rgba[canvas_rgba];[1:v]format=gray[mask];[canvas_rgba][mask]alphamerge[masked];[masked][2:v]overlay=0:0:format=auto,format=yuv420p[out]",
      "-map", "[out]",
      "-an",
      "-t", "2.111",
      "-c:v", "libx264",
      "-pix_fmt", "yuv420p",
      "-movflags", "+faststart",
      "/tmp/out.mp4",
    ],
  );
});

test("buildRemoteRecordCommand prewarms on-device, returns to SpringBoard, then records the warm relaunch", () => {
  const command = buildRemoteRecordCommand({
    appearance: "light",
    homeDwellMs: 1000,
    prewarmDelayMs: 6000,
    recordingDurationMs: 10_500,
    relaunchDelayMs: 800,
    remoteBundleId: "rs.ctx.hnmobile",
    remoteDeviceName: "iPhone 16",
    sessionName: "ctx-hn-mobile-record-test",
  });
  assert.match(command, /xcrun simctl ui 'iPhone 16' appearance 'light'/);
  assert.match(command, /xcrun simctl terminate 'iPhone 16' 'rs\.ctx\.hnmobile'/);
  assert.match(command, /xcrun simctl launch 'iPhone 16' 'rs\.ctx\.hnmobile' >\/tmp\/ctx-hn-mobile-record-test\.prewarm\.log/);
  assert.match(command, /sleep 6\.000/);
  assert.match(command, /xcrun simctl launch 'iPhone 16' com\.apple\.springboard >\/tmp\/ctx-hn-mobile-record-test\.home\.log/);
  assert.match(command, /xcrun simctl io 'iPhone 16' recordVideo --codec=h264 --mask=black '\/tmp\/ctx-hn-mobile-record-test\.mov'/);
  assert.match(command, /sleep 0\.800/);
  assert.match(command, /xcrun simctl launch 'iPhone 16' 'rs\.ctx\.hnmobile' >\/tmp\/ctx-hn-mobile-record-test\.launch\.log/);
});

test("resolveLocalVideoPlan accepts mov and mp4 outputs only", () => {
  assert.deepEqual(resolveLocalVideoPlan("/tmp/out.mov"), {
    finalPath: "/tmp/out.mov",
    needsTranscode: false,
  });
  assert.deepEqual(resolveLocalVideoPlan("/tmp/out.mp4"), {
    finalPath: "/tmp/out.mp4",
    needsTranscode: true,
  });
  assert.throws(() => resolveLocalVideoPlan("/tmp/out.webm"), /must end with \.mov or \.mp4/);
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
