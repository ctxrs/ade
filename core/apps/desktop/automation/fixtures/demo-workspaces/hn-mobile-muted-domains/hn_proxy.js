export const HN_ORIGIN = "https://news.ycombinator.com";
export const HN_PROXY_PREFIX = "/proxy/hn";
export const HN_DEMO_FRONT_PAGE_QUERY_KEY = "ctxDemoMockFrontPage";
const HN_DEMO_PROFILE_ID = "ADE_TEST_ACCOUNT";

const HN_SCRIPT_PATTERN = /<script[^>]*src=["'][^"']*hn\.js[^"']*["'][^>]*><\/script>/gi;
const HN_HEAD_CLOSE_PATTERN = /<\/head>/i;
const HN_STORY_ROW_GROUP_PATTERN =
  /<tr class="athing submission"[\s\S]*?<\/tr><tr><td colspan="2"><\/td><td class="subtext">[\s\S]*?<\/tr><tr class="spacer"[^>]*><\/tr>/g;
const HN_HEAD_INJECTION = `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />`;
const DEMO_FRONT_PAGE_STORIES = Object.freeze([
  {
    id: "47584540",
    rank: 1,
    href: "https://twitter.com/Fried_rice/status/2038894956459290963",
    title: "Claude Code's source code has been leaked via a map file in their NPM registry",
    site: "twitter.com",
    points: 2007,
    user: "treexs",
    age: "1 day ago",
    comments: 992,
  },
  {
    id: "47582792",
    rank: 2,
    href: "https://twitter.com/oliviscusAI/status/2038563166431346865",
    title: "You can now run a full Linux operating system inside a 6mb PDF",
    site: "twitter.com",
    points: 22,
    user: "matthewsinclair",
    age: "1 day ago",
    comments: 2,
  },
]);
const DEMO_PROFILE_HTML = `<html lang="en" op="user"><head><meta name="referrer" content="origin"><meta name="viewport" content="width=device-width, initial-scale=1.0"><link rel="stylesheet" type="text/css" href="news.css?T7eOwTlIxzUnpE6JjSkE"><link rel="icon" href="y18.svg"><link rel="canonical" href="https://news.ycombinator.com/user?id=ADE_TEST_ACCOUNT"><title>Profile: ADE_TEST_ACCOUNT | Hacker News</title></head><body><center><table id="hnmain" border="0" cellpadding="0" cellspacing="0" width="85%" bgcolor="#f6f6ef"><tr><td bgcolor="#ff6600"><table border="0" cellpadding="0" cellspacing="0" width="100%" style="padding:2px"><tr><td style="width:18px;padding-right:4px"><a href="https://news.ycombinator.com"><img src="y18.svg" width="18" height="18" style="border:1px white solid; display:block"></a></td><td style="line-height:12pt; height:10px;"><span class="pagetop"><b class="hnname"><a href="news">Hacker News</a></b><a href="newest">new</a> | <a href="front">past</a> | <a href="newcomments">comments</a> | <a href="ask">ask</a> | <a href="show">show</a> | <a href="jobs">jobs</a> | <a href="submit" rel="nofollow">submit</a></span></td><td style="text-align:right;padding-right:4px;"><span class="pagetop"><a href="user?id=ADE_TEST_ACCOUNT">ADE_TEST_ACCOUNT</a> (1) | <a href="logout">logout</a></span></td></tr></table></td></tr><tr><td bgcolor="#ffffaa" style="font-size: 13px; color: #6b6b6b; padding: 8px 10px;">Please put a valid address in the email field, or we won't be able to send you a new password if you forget yours. Your address is only visible to you and us. Crawlers and other users can't see it.</td></tr><tr style='height:10px'></tr><tr id="bigbox"><td><form method="get" action="user"><input type="hidden" name="id" value="ADE_TEST_ACCOUNT"><input type="hidden" name="${HN_DEMO_FRONT_PAGE_QUERY_KEY}" value="1"><table border="0"><tr class="athing"><td valign="top">user:</td><td timestamp="1772659894"><a href="user?id=ADE_TEST_ACCOUNT" class="hnuser">ADE_TEST_ACCOUNT</a></td></tr><tr><td valign="top">created:</td><td><a href="front?day=2026-03-04&birth=ADE_TEST_ACCOUNT">27 days ago</a></td></tr><tr><td valign="top">karma:</td><td>1</td></tr><tr><td valign="top">about:</td><td style="overflow:hidden"><textarea name="about" rows="5" cols="60" style="width:100%;max-width:540px;"></textarea><span style="color:#999;padding-left:4px">help</span></td></tr><tr><td></td><td style="color:#828282;font-size:13px;padding-top:2px;">Only admins see your email below. To share publicly, add to the 'about' box.</td></tr><tr><td valign="top">email:</td><td><input type="text" name="email" value="" size="40"></td></tr><tr><td valign="top">showdead:</td><td><select name="showdead"><option selected>no</option><option>yes</option></select></td></tr><tr><td valign="top">noprocrast:</td><td><select name="noprocrast"><option selected>no</option><option>yes</option></select></td></tr><tr><td valign="top">maxvisit:</td><td><input type="text" name="maxvisit" value="20" size="4"></td></tr><tr><td valign="top">minaway:</td><td><input type="text" name="minaway" value="180" size="4"></td></tr><tr><td valign="top">delay:</td><td><input type="text" name="delay" value="0" size="4"></td></tr><tr><td></td><td style="padding-top:8px;"><a href="changepw"><u>change password</u></a></td></tr><tr><td></td><td><a href="submitted?id=ADE_TEST_ACCOUNT"><u>submissions</u></a></td></tr><tr><td></td><td><a href="threads?id=ADE_TEST_ACCOUNT"><u>comments</u></a></td></tr><tr><td></td><td><a href="upvoted?id=ADE_TEST_ACCOUNT"><u>upvoted submissions</u></a> / <a href="upvoted?id=ADE_TEST_ACCOUNT&comments=t"><u>comments</u></a></td></tr><tr><td></td><td><a href="favorites?id=ADE_TEST_ACCOUNT"><u>favorite submissions</u></a> / <a href="favorites?id=ADE_TEST_ACCOUNT&comments=t"><u>comments</u></a> <span style="color:#828282"><i>(publicly visible)</i></span></td></tr><tr><td></td><td style="padding-top:12px;"><input type="submit" value="update"></td></tr></table></form><br><br></td></tr></table></center></body><script type="text/javascript" src="hn.js?T7eOwTlIxzUnpE6JjSkE"></script></html>`;

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

function buildDemoFrontPageStoryGroupHtml(story) {
  return `<tr class="athing submission" id="${story.id}"><td align="right" valign="top" class="title"><span class="rank">${story.rank}.</span></td><td valign="top" class="votelinks"></td><td class="title"><span class="titleline"><a href="${story.href}">${story.title}</a><span class="sitebit comhead"> (<a href="from?site=${story.site}"><span class="sitestr">${story.site}</span></a>)</span></span></td></tr><tr><td colspan="2"></td><td class="subtext"><span class="subline"><span class="score" id="score_${story.id}">${story.points} points</span> by <a href="user?id=${story.user}" class="hnuser">${story.user}</a> <span class="age"><a href="item?id=${story.id}">${story.age}</a></span> | <a href="item?id=${story.id}">${story.comments}&nbsp;comments</a></span></td></tr><tr class="spacer" style="height:5px"></tr>`;
}

export function shouldMockDemoFrontPage(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  return (
    extractLocalHackerNewsPath(localUrl) === "/news" &&
    !localUrl.searchParams.has("p") &&
    localUrl.searchParams.get(HN_DEMO_FRONT_PAGE_QUERY_KEY) === "1"
  );
}

export function shouldMockDemoProfilePage(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  return (
    extractLocalHackerNewsPath(localUrl) === "/user" &&
    localUrl.searchParams.get("id") === HN_DEMO_PROFILE_ID &&
    localUrl.searchParams.get(HN_DEMO_FRONT_PAGE_QUERY_KEY) === "1"
  );
}

export function buildDemoProfileHtml() {
  return DEMO_PROFILE_HTML;
}

export function injectDemoFrontPageStories(html) {
  const demoStoryMarkup = DEMO_FRONT_PAGE_STORIES.map(buildDemoFrontPageStoryGroupHtml).join("");
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

export function rewriteHackerNewsAttribute(attributeName, rawValue) {
  const value = String(rawValue ?? "").trim();
  if (!value || value.startsWith("#") || value.startsWith("data:") || value.startsWith("javascript:") || value.startsWith("mailto:")) {
    return value;
  }

  const absoluteUrl = new URL(value, HN_ORIGIN);
  if (absoluteUrl.origin !== HN_ORIGIN) {
    return absoluteUrl.href;
  }
  if (attributeName === "action") {
    return rewriteHackerNewsLocation(absoluteUrl.href);
  }
  if (isAssetPath(absoluteUrl.pathname)) {
    return absoluteUrl.href;
  }
  return rewriteHackerNewsLocation(absoluteUrl.href);
}

export function rewriteHackerNewsHtml(html, options = {}) {
  const sourceHtml = options.mockDemoFrontPage ? injectDemoFrontPageStories(html) : String(html ?? "");
  return sourceHtml
    .replace(HN_SCRIPT_PATTERN, "")
    .replace(HN_HEAD_CLOSE_PATTERN, `${HN_HEAD_INJECTION}</head>`)
    .replace(/\b(href|src|action)=["']([^"']+)["']/gi, (match, attributeName, value) => {
      const nextValue = rewriteHackerNewsAttribute(attributeName.toLowerCase(), value);
      return `${attributeName}="${nextValue}"`;
    });
}

export function buildHackerNewsTargetUrl(requestUrl) {
  const localUrl = createLocalProxyUrl(requestUrl);
  localUrl.searchParams.delete(HN_DEMO_FRONT_PAGE_QUERY_KEY);
  const pathname = extractLocalHackerNewsPath(localUrl);
  return new URL(`${pathname}${localUrl.search}`, HN_ORIGIN);
}

export function rewriteHackerNewsLocation(rawLocation) {
  if (!rawLocation) {
    return "";
  }
  const absoluteUrl = new URL(rawLocation, HN_ORIGIN);
  if (absoluteUrl.origin !== HN_ORIGIN) {
    return absoluteUrl.href;
  }
  return `${HN_PROXY_PREFIX}${absoluteUrl.pathname}${absoluteUrl.search}${absoluteUrl.hash}`;
}

export function rewriteHackerNewsSetCookie(rawCookie) {
  return String(rawCookie ?? "")
    .split(";")
    .map((segment) => segment.trim())
    .filter((segment) => segment && !/^domain=/i.test(segment) && !/^secure$/i.test(segment))
    .join("; ");
}
