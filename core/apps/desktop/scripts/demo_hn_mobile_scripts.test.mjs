import test from "node:test";
import assert from "node:assert/strict";

import { resolveCapturePrompt } from "./demo_hn_mobile_capture.mjs";
import { inferVideoArtifactMimeType } from "./demo_video_artifacts.mjs";
import {
  buildArtifactPlaybackCompletionPlan,
  buildExpectedSidebarTaskTitles,
  buildMouseTimings,
  buildPromptCharacters,
  buildRecordStartDelayPlan,
  buildFrameSequenceFfmpegArgs,
  computeArtifactPlaybackTimeoutMs,
  ensureSidebarOpen,
  findNewTaskRecord,
  parseArgs as parsePlaybackArgs,
  pointFromWindowRectRect,
  resolvePlaybackPrompt,
  resolvePlaybackSessionArtifactPath,
  resolveTaskWorktreeRoot,
  sidebarMatchesExpectedState,
  visibleHarnessRowsNeedInstall,
} from "./demo_hn_mobile_playback.mjs";
import { selectInstallableVisibleHarnessProviderIds } from "./demo_ping_pong_playback.mjs";
import { buildPlaywrightRunner, buildWrapperHtml } from "./demo_hn_mobile_record_artifact.mjs";
import { resolveDefaultPagePath } from "../automation/fixtures/demo-workspaces/hn-mobile-muted-domains/src/app.js";
import {
  buildDemoProfileHtml,
  buildHackerNewsTargetUrl,
  HN_DEMO_FRONT_PAGE_QUERY_KEY,
  HN_PROXY_PREFIX,
  injectDemoFrontPageStories,
  rewriteHackerNewsAttribute,
  rewriteHackerNewsHtml,
  rewriteHackerNewsLocation,
  rewriteHackerNewsSetCookie,
  shouldMockDemoProfilePage,
  shouldMockDemoFrontPage,
} from "../automation/fixtures/demo-workspaces/hn-mobile-muted-domains/hn_proxy.js";
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
} from "../automation/fixtures/demo-workspaces/hn-mobile-muted-domains/src/hn_enhancer.js";
import { resolveScreenHeight } from "../automation/fixtures/demo-workspaces/hn-mobile-muted-domains/src/viewport.js";

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
  const fixture = { session_artifact_path: "demo-artifacts/hn-muted-domains.mp4" };
  assert.equal(
    resolvePlaybackSessionArtifactPath(fixturePath, fixture, null),
    "/tmp/demo-fixtures/demo-artifacts/hn-muted-domains.mp4",
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

test("selectInstallableVisibleHarnessProviderIds keeps only supported visible missing harnesses", () => {
  assert.deepEqual(
    selectInstallableVisibleHarnessProviderIds([
      {
        provider_id: "codex",
        installed: false,
        details: { install_supported: "1" },
      },
      {
        provider_id: "hidden-agent",
        installed: false,
        details: { install_supported: "1", ui_hidden: "true" },
      },
      {
        provider_id: "dependency-agent",
        installed: false,
        details: { install_supported: "1", provider_kind: "dependency" },
      },
      {
        provider_id: "cursor",
        installed: true,
        details: { install_supported: "1" },
      },
      {
        provider_id: "unsupported-agent",
        installed: false,
        details: {},
      },
    ]),
    ["codex"],
  );
});

test("computeArtifactPlaybackTimeoutMs uses video duration plus buffer", () => {
  assert.equal(computeArtifactPlaybackTimeoutMs(11.233008), 12734);
  assert.equal(computeArtifactPlaybackTimeoutMs(null), 20000);
});

test("buildArtifactPlaybackCompletionPlan includes a 1s end hold after the video finishes", () => {
  assert.deepEqual(
    buildArtifactPlaybackCompletionPlan(11.233008),
    {
      timeoutMs: 12734,
      tailDwellMs: 1000,
    },
  );
});

test("buildExpectedSidebarTaskTitles orders seeded tasks newest first by minutes_ago", () => {
  assert.deepEqual(
    buildExpectedSidebarTaskTitles({
      active_tasks: [
        { title: "Oldest", minutes_ago: 41 },
        { title: "Newest", minutes_ago: 2 },
        { title: "Middle", minutes_ago: 12 },
      ],
    }),
    ["Newest", "Middle", "Oldest"],
  );
});

test("sidebarMatchesExpectedState requires full ordering and exactly one working row", () => {
  assert.equal(
    sidebarMatchesExpectedState(
      {
        rows: [
          { title: "New task", hasWorkingSpinner: false },
          { title: "Newest", hasWorkingSpinner: true },
          { title: "Middle", hasWorkingSpinner: false },
          { title: "Oldest", hasWorkingSpinner: false },
        ],
      },
      ["Newest", "Middle", "Oldest"],
      "Newest",
    ),
    true,
  );
  assert.equal(
    sidebarMatchesExpectedState(
      {
        rows: [
          { title: "Newest", hasWorkingSpinner: true },
          { title: "Oldest", hasWorkingSpinner: true },
          { title: "Middle", hasWorkingSpinner: false },
        ],
      },
      ["Newest", "Middle", "Oldest"],
      "Newest",
    ),
    false,
  );
});

test("ensureSidebarOpen clicks the collapsed toggle until the sidebar is visible", async () => {
  let attempts = 0;
  const browser = {
    waitUntil: async (predicate) => {
      for (let index = 0; index < 3; index += 1) {
        attempts += 1;
        if (await predicate()) {
          return true;
        }
      }
      throw new Error("sidebar did not open");
    },
    execute: async () => attempts >= 2,
  };
  await assert.doesNotReject(() => ensureSidebarOpen(browser));
  assert.ok(attempts >= 2);
});

test("parseArgs defaults the HN storyboard to one curated diff file", () => {
  const options = parsePlaybackArgs([]);
  assert.equal(options.diffFilePath, "src/hn_enhancer.js");
  assert.equal(options.secondaryDiffFilePath, null);
  assert.equal(options.harnessMenuDwellMs, 3000);
  assert.equal(options.readyPrerollMs, 0);
  assert.equal(options.recordStartDelayMs, 0);
  assert.equal(options.mouseTakeoverLeadMs, 2000);
  assert.equal(options.mouseTimings.harnessTriggerMoveMs, 420);
  assert.equal(options.mouseTimings.artifactPlayMoveMs, 520);
});

test("parseArgs keeps an explicit secondary diff file opt-in", () => {
  const options = parsePlaybackArgs(["--secondary-diff-file", "hn_proxy.js"]);
  assert.equal(options.secondaryDiffFilePath, "hn_proxy.js");
});

test("parseArgs allows overriding the harness selector dwell", () => {
  const options = parsePlaybackArgs(["--harness-menu-dwell-ms", "900"]);
  assert.equal(options.harnessMenuDwellMs, 900);
});

test("parseArgs allows overriding the ready preroll dwell", () => {
  const options = parsePlaybackArgs(["--ready-preroll-ms", "2500"]);
  assert.equal(options.readyPrerollMs, 2500);
});

test("parseArgs allows overriding the pre-action record start delay", () => {
  const options = parsePlaybackArgs(["--record-start-delay-ms", "30000"]);
  assert.equal(options.recordStartDelayMs, 30000);
});

test("parseArgs allows overriding the mouse takeover lead time", () => {
  const options = parsePlaybackArgs(["--mouse-takeover-lead-ms", "2400"]);
  assert.equal(options.mouseTakeoverLeadMs, 2400);
});

test("parseArgs allows overriding mouse choreography timing", () => {
  const options = parsePlaybackArgs([
    "--mouse-harness-trigger-move-ms", "760",
    "--mouse-prompt-cps", "22",
    "--mouse-artifact-play-move-ms", "980",
  ]);
  assert.equal(options.mouseTimings.harnessTriggerMoveMs, 760);
  assert.equal(options.mouseTimings.promptTypingCps, 22);
  assert.equal(options.mouseTimings.artifactPlayMoveMs, 980);
});

test("buildMouseTimings falls back when overrides are invalid", () => {
  assert.deepEqual(
    buildMouseTimings({
      harnessTriggerMoveMs: "nope",
      promptTypingCps: 0,
      artifactPlayMoveMs: -1,
    }),
    buildMouseTimings(),
  );
});

test("buildRecordStartDelayPlan leaves most of the delay idle and reserves a short takeover window", () => {
  assert.deepEqual(
    buildRecordStartDelayPlan(30000, 2000),
    {
      idleDelayMs: 28000,
      preSequenceTakeoverDelayMs: 2000,
    },
  );
  assert.deepEqual(
    buildRecordStartDelayPlan(1500, 2000),
    {
      idleDelayMs: 0,
      preSequenceTakeoverDelayMs: 1500,
    },
  );
});

test("pointFromWindowRectRect maps DOM rects into top-based native cursor coordinates using the app inner window rect", () => {
  assert.deepEqual(
    pointFromWindowRectRect(
      { x: 0, y: 80, width: 1728, height: 962 },
      { innerWidth: 1728, innerHeight: 962, screenHeight: 1117, windowInnerPosition: { x: 0, y: 66 } },
      { left: 594.5, top: 522, width: 103.109375, height: 22 },
      0.52,
      0.5,
    ),
    { x: 648.12, y: 599 },
  );
});

test("visibleHarnessRowsNeedInstall detects whether any visible rows still need install", () => {
  assert.equal(visibleHarnessRowsNeedInstall([{ label: "Codex", hasInstallButton: false }]), false);
  assert.equal(visibleHarnessRowsNeedInstall([{ label: "Goose", hasInstallButton: true }]), true);
});

test("buildWrapperHtml renders a phone shell iframe", () => {
  const markup = buildWrapperHtml("http://127.0.0.1:4173");
  assert.match(markup, /class="phone"/);
  assert.match(markup, /iframe class="demo-phone-screen" src="http:\/\/127\.0\.0\.1:4173"/);
});

test("buildPlaywrightRunner records the muted-domains settings flow", () => {
  const runner = buildPlaywrightRunner();
  assert.match(runner, /data-muted-domains-input/);
  assert.match(runner, /ctxDemoMockFrontPage=1/);
  assert.match(runner, /ctx-muted-domains-toast/);
  assert.doesNotMatch(runner, /a\[href\*="\/proxy\/hn\/user\?id=ADE_TEST_ACCOUNT"\]/);
  assert.doesNotMatch(runner, /data-save-current-page/);
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
    rewriteHackerNewsAttribute("action", "login?goto=news"),
    `${HN_PROXY_PREFIX}/login?goto=news`,
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
    buildHackerNewsTargetUrl(`${HN_PROXY_PREFIX}/news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`).href,
    "https://news.ycombinator.com/news",
  );
});

test("shouldMockDemoFrontPage only targets the front-page request", () => {
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`), true);
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/news?p=2&${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`), false);
  assert.equal(shouldMockDemoFrontPage(`${HN_PROXY_PREFIX}/item?id=1&${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`), false);
});

test("shouldMockDemoProfilePage only targets the ADE_TEST_ACCOUNT demo profile", () => {
  assert.equal(shouldMockDemoProfilePage(`${HN_PROXY_PREFIX}/user?id=ADE_TEST_ACCOUNT&${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`), true);
  assert.equal(shouldMockDemoProfilePage(`${HN_PROXY_PREFIX}/user?id=pg&${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`), false);
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
  assert.equal(buildFeedPreviewPath("?demoSeedMutedStories=1"), `/proxy/hn/news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`);
  assert.equal(buildFeedPreviewPath(""), `/proxy/hn/news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`);
});

test("buildMutedDomainsToastMessage uses the shortened copy", () => {
  assert.equal(buildMutedDomainsToastMessage(1), "Hid 1 story from muted domains.");
  assert.equal(buildMutedDomainsToastMessage(2), "Hid 2 stories from muted domains.");
});

test("buildDemoLoginGoto preserves the demo front-page flag through login", () => {
  assert.equal(buildDemoLoginGoto("?demoSeedMutedStories=1"), `news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`);
  assert.equal(buildDemoLoginGoto(""), `news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`);
});

test("buildDemoLoginFields prepares proxied HN login fields", () => {
  assert.deepEqual(
    buildDemoLoginFields("?demoLoginUser=ADE_TEST_ACCOUNT&demoLoginPassword=secret&demoSeedMutedStories=1"),
    {
      acct: "ADE_TEST_ACCOUNT",
      pw: "secret",
      goto: `news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`,
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
    resolveDefaultPagePath(`?demoPath=${encodeURIComponent("/proxy/hn/user?id=ADE_TEST_ACCOUNT&ctxDemoMockFrontPage=1")}`),
    "/proxy/hn/user?id=ADE_TEST_ACCOUNT&ctxDemoMockFrontPage=1",
  );
  assert.equal(
    resolveDefaultPagePath(`?demoPath=${encodeURIComponent("https://example.com")}`),
    `/proxy/hn/news?${HN_DEMO_FRONT_PAGE_QUERY_KEY}=1`,
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
