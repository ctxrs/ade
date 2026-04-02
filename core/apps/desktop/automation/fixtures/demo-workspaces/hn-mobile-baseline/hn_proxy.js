import {
  DEFAULT_HN_DEMO_SCENARIO_ID,
  getDemoScenario,
  HN_DEMO_SCENARIO_QUERY_KEY,
  resolveDemoScenarioId,
} from "./demo_scenarios.js";

export const HN_ORIGIN = "https://news.ycombinator.com";
export const HN_PROXY_PREFIX = "/proxy/hn";
export { DEFAULT_HN_DEMO_SCENARIO_ID, HN_DEMO_SCENARIO_QUERY_KEY };

const HN_SCRIPT_PATTERN = /<script[^>]*src=["'][^"']*hn\.js[^"']*["'][^>]*><\/script>/gi;
const HN_HEAD_CLOSE_PATTERN = /<\/head>/i;
const HN_STORY_ROW_GROUP_PATTERN =
  /<tr class="athing submission"[\s\S]*?<\/tr><tr><td colspan="2"><\/td><td class="subtext">[\s\S]*?<\/tr><tr class="spacer"[^>]*><\/tr>/g;
const HN_HEAD_INJECTION = `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />`;
const HN_ACCOUNT_NAV_PATTERN =
  /(<td[^>]*style="[^"]*text-align:right[^"]*"[^>]*>\s*<span class="pagetop">)([\s\S]*?)(<\/span>\s*<\/td>)/i;

function isAssetPath(pathname) {
  return /\.(css|gif|ico|jpe?g|js|png|svg)$/i.test(pathname);
}

function createLocalProxyUrl(requestUrl) {
  return new URL(requestUrl, "http://127.0.0.1");
}

function extractLocalHackerNewsPath(localUrl) {
  const suffix = localUrl.pathname.startsWith(HN_PROXY_PREFIX)
    ? localUrl.pathname.slice(HN_PROXY_PREFIX.length)
    : localUrl.pathname;
  return suffix || "/news";
}

function readRequestScenarioId(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  const scenarioId = localUrl.searchParams.get(HN_DEMO_SCENARIO_QUERY_KEY);
  return scenarioId ? resolveDemoScenarioId(scenarioId) : null;
}

function buildDemoFrontPageStoryGroupHtml(story) {
  return `<tr class="athing submission" id="${story.id}"><td align="right" valign="top" class="title"><span class="rank">${story.rank}.</span></td><td valign="top" class="votelinks"></td><td class="title"><span class="titleline"><a href="${story.href}">${story.title}</a><span class="sitebit comhead"> (<a href="from?site=${story.site}"><span class="sitestr">${story.site}</span></a>)</span></span></td></tr><tr><td colspan="2"></td><td class="subtext"><span class="subline"><span class="score" id="score_${story.id}">${story.points} points</span> by <a href="user?id=${story.user}" class="hnuser">${story.user}</a> <span class="age"><a href="item?id=${story.id}">${story.age}</a></span> | <a href="item?id=${story.id}">${story.comments}&nbsp;comments</a></span></td></tr><tr class="spacer" style="height:5px"></tr>`;
}

function buildDemoAccountNavHtml(rawScenarioId) {
  const scenario = getDemoScenario(rawScenarioId);
  return `<a href="user?id=${scenario.accountId}">${scenario.accountId}</a> (1) | <a href="logout">logout</a>`;
}

function buildDemoProfileMarkup(rawScenarioId) {
  const scenario = getDemoScenario(rawScenarioId);
  const accountHref = `user?id=${encodeURIComponent(scenario.accountId)}`;
  const favoritesHref = `favorites?id=${encodeURIComponent(scenario.accountId)}`;
  const favoriteCommentsHref = `favorites?id=${encodeURIComponent(scenario.accountId)}&comments=t`;
  const submittedHref = `submitted?id=${encodeURIComponent(scenario.accountId)}`;
  const threadsHref = `threads?id=${encodeURIComponent(scenario.accountId)}`;
  const upvotedHref = `upvoted?id=${encodeURIComponent(scenario.accountId)}`;
  const upvotedCommentsHref = `upvoted?id=${encodeURIComponent(scenario.accountId)}&comments=t`;
  const createdHref = `front?day=2026-03-04&birth=${encodeURIComponent(scenario.accountId)}`;

  return `
    <html lang="en" op="user">
      <head>
        <meta name="referrer" content="origin">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <link rel="stylesheet" type="text/css" href="news.css?T7eOwTlIxzUnpE6JjSkE">
        <link rel="icon" href="y18.svg">
        <link rel="canonical" href="https://news.ycombinator.com/user?id=${scenario.accountId}">
        <title>Profile: ${scenario.accountId} | Hacker News</title>
      </head>
      <body>
        <center>
          <table id="hnmain" border="0" cellpadding="0" cellspacing="0" width="85%" bgcolor="#f6f6ef">
            <tr>
              <td bgcolor="#ff6600">
                <table border="0" cellpadding="0" cellspacing="0" width="100%" style="padding:2px">
                  <tr>
                    <td style="width:18px;padding-right:4px">
                      <a href="news"><img src="y18.svg" width="18" height="18" style="border:1px white solid; display:block"></a>
                    </td>
                    <td style="line-height:12pt; height:10px;">
                      <span class="pagetop">
                        <b class="hnname"><a href="news">Hacker News</a></b>
                        <a href="newest">new</a> |
                        <a href="front">past</a> |
                        <a href="newcomments">comments</a> |
                        <a href="ask">ask</a> |
                        <a href="show">show</a> |
                        <a href="jobs">jobs</a> |
                        <a href="submit" rel="nofollow">submit</a>
                      </span>
                    </td>
                    <td style="text-align:right;padding-right:4px;">
                      <span class="pagetop">
                        <a href="${accountHref}">${scenario.accountId}</a> (1) |
                        <a href="logout">logout</a>
                      </span>
                    </td>
                  </tr>
                </table>
              </td>
            </tr>
            <tr style="height:10px"></tr>
            <tr id="bigbox">
              <td>
                <form method="get" action="user">
                  <input type="hidden" name="id" value="${scenario.accountId}">
                  <input type="hidden" name="${HN_DEMO_SCENARIO_QUERY_KEY}" value="${scenario.id}">
                  <table border="0">
                    <tr class="athing">
                      <td valign="top">user:</td>
                      <td timestamp="1772659894"><a href="${accountHref}" class="hnuser">${scenario.accountId}</a></td>
                    </tr>
                    <tr>
                      <td valign="top">created:</td>
                      <td><a href="${createdHref}">27 days ago</a></td>
                    </tr>
                    <tr>
                      <td valign="top">karma:</td>
                      <td>1</td>
                    </tr>
                    <tr>
                      <td valign="top">about:</td>
                      <td style="overflow:hidden"><textarea name="about" rows="5" cols="60" style="width:100%;max-width:540px;"></textarea><span style="color:#999;padding-left:4px">help</span></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td style="color:#828282;font-size:13px;padding-top:2px;">Only admins see your email below. To share publicly, add to the 'about' box.</td>
                    </tr>
                    <tr>
                      <td valign="top">email:</td>
                      <td><input type="text" name="email" value="${scenario.emailAddress ?? ""}" size="40"></td>
                    </tr>
                    <tr>
                      <td valign="top">showdead:</td>
                      <td><select name="showdead"><option selected>no</option><option>yes</option></select></td>
                    </tr>
                    <tr>
                      <td valign="top">noprocrast:</td>
                      <td><select name="noprocrast"><option selected>no</option><option>yes</option></select></td>
                    </tr>
                    <tr>
                      <td valign="top">maxvisit:</td>
                      <td><input type="text" name="maxvisit" value="20" size="4"></td>
                    </tr>
                    <tr>
                      <td valign="top">minaway:</td>
                      <td><input type="text" name="minaway" value="180" size="4"></td>
                    </tr>
                    <tr>
                      <td valign="top">delay:</td>
                      <td><input type="text" name="delay" value="0" size="4"></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td style="padding-top:8px;"><a href="changepw"><u>change password</u></a></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td><a href="${submittedHref}"><u>submissions</u></a></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td><a href="${threadsHref}"><u>comments</u></a></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td><a href="${upvotedHref}"><u>upvoted submissions</u></a> / <a href="${upvotedCommentsHref}"><u>comments</u></a></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td><a href="${favoritesHref}"><u>favorite submissions</u></a> / <a href="${favoriteCommentsHref}"><u>comments</u></a> <span style="color:#828282"><i>(publicly visible)</i></span></td>
                    </tr>
                    <tr>
                      <td></td>
                      <td style="padding-top:12px;"><input type="submit" value="update"></td>
                    </tr>
                  </table>
                </form>
                <br><br>
              </td>
            </tr>
          </table>
        </center>
      </body>
      <script type="text/javascript" src="hn.js?T7eOwTlIxzUnpE6JjSkE"></script>
    </html>
  `;
}

export function shouldMockDemoFrontPage(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  const scenarioId = localUrl.searchParams.get(HN_DEMO_SCENARIO_QUERY_KEY);
  return (
    extractLocalHackerNewsPath(localUrl) === "/news" &&
    !localUrl.searchParams.has("p") &&
    !!scenarioId
  );
}

export function shouldMockDemoProfilePage(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  const scenarioId = localUrl.searchParams.get(HN_DEMO_SCENARIO_QUERY_KEY);
  if (extractLocalHackerNewsPath(localUrl) !== "/user" || !scenarioId) {
    return false;
  }
  return localUrl.searchParams.get("id") === getDemoScenario(scenarioId).accountId;
}

export function buildDemoProfileHtml(rawScenarioId = DEFAULT_HN_DEMO_SCENARIO_ID) {
  return buildDemoProfileMarkup(rawScenarioId);
}

export function injectDemoFrontPageStories(html, rawScenarioId = DEFAULT_HN_DEMO_SCENARIO_ID) {
  const scenario = getDemoScenario(rawScenarioId);
  const demoStoryMarkup = scenario.frontPageStories.map(buildDemoFrontPageStoryGroupHtml).join("");
  let replacedGroupCount = 0;

  return String(html ?? "").replace(HN_STORY_ROW_GROUP_PATTERN, (rowGroup) => {
    replacedGroupCount += 1;
    if (replacedGroupCount === 1) {
      return demoStoryMarkup;
    }
    if (replacedGroupCount === 2) {
      return "";
    }
    return rowGroup;
  });
}

export function injectDemoFrontPageAccountNav(html, rawScenarioId = DEFAULT_HN_DEMO_SCENARIO_ID) {
  return String(html ?? "").replace(HN_ACCOUNT_NAV_PATTERN, (_match, prefix, _content, suffix) => {
    return `${prefix}${buildDemoAccountNavHtml(rawScenarioId)}${suffix}`;
  });
}

export function rewriteHackerNewsAttribute(attributeName, rawValue, options = {}) {
  const value = String(rawValue ?? "").trim();
  if (!value || value.startsWith("#") || value.startsWith("data:") || value.startsWith("javascript:") || value.startsWith("mailto:")) {
    return value;
  }

  const absoluteUrl = new URL(value, HN_ORIGIN);
  if (absoluteUrl.origin !== HN_ORIGIN) {
    return absoluteUrl.href;
  }
  if (attributeName === "action") {
    return rewriteHackerNewsLocation(absoluteUrl.href, options);
  }
  if (isAssetPath(absoluteUrl.pathname)) {
    return absoluteUrl.href;
  }
  return rewriteHackerNewsLocation(absoluteUrl.href, options);
}

export function rewriteHackerNewsHtml(html, options = {}) {
  const scenarioId = options.scenarioId ? resolveDemoScenarioId(options.scenarioId) : null;
  let sourceHtml = String(html ?? "");
  if (options.mockDemoFrontPage && scenarioId) {
    sourceHtml = injectDemoFrontPageAccountNav(injectDemoFrontPageStories(sourceHtml, scenarioId), scenarioId);
  }

  return sourceHtml
    .replace(HN_SCRIPT_PATTERN, "")
    .replace(HN_HEAD_CLOSE_PATTERN, `${HN_HEAD_INJECTION}</head>`)
    .replace(/\b(href|src|action)=["']([^"']+)["']/gi, (_match, attributeName, value) => {
      const nextValue = rewriteHackerNewsAttribute(attributeName.toLowerCase(), value, { scenarioId });
      return `${attributeName}="${nextValue}"`;
    });
}

export function buildHackerNewsTargetUrl(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  localUrl.searchParams.delete(HN_DEMO_SCENARIO_QUERY_KEY);
  const pathname = extractLocalHackerNewsPath(localUrl);
  return new URL(`${pathname}${localUrl.search}`, HN_ORIGIN);
}

export function rewriteHackerNewsLocation(rawLocation, options = {}) {
  if (!rawLocation) {
    return "";
  }
  const absoluteUrl = new URL(rawLocation, HN_ORIGIN);
  if (absoluteUrl.origin !== HN_ORIGIN) {
    return absoluteUrl.href;
  }

  const localUrl = new URL(`${HN_PROXY_PREFIX}${absoluteUrl.pathname}${absoluteUrl.search}${absoluteUrl.hash}`, "http://127.0.0.1");
  if (options.scenarioId) {
    localUrl.searchParams.set(HN_DEMO_SCENARIO_QUERY_KEY, resolveDemoScenarioId(options.scenarioId));
  }
  return `${localUrl.pathname}${localUrl.search}${localUrl.hash}`;
}

export function rewriteHackerNewsSetCookie(rawCookie) {
  return String(rawCookie ?? "")
    .split(";")
    .map((segment) => segment.trim())
    .filter((segment) => segment && !/^domain=/i.test(segment) && !/^secure$/i.test(segment))
    .join("; ");
}

export function resolveRequestScenarioId(requestUrl) {
  return readRequestScenarioId(requestUrl);
}
