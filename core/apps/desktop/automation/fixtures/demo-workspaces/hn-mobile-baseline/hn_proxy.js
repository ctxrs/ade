export const HN_ORIGIN = "https://news.ycombinator.com";
export const HN_PROXY_PREFIX = "/proxy/hn";

const HN_SCRIPT_PATTERN = /<script[^>]*src=["'][^"']*hn\.js[^"']*["'][^>]*><\/script>/gi;

function isAssetPath(pathname) {
  return /\.(css|gif|ico|jpe?g|js|png|svg)$/i.test(pathname);
}

export function rewriteHackerNewsAttribute(attributeName, rawValue) {
  const value = String(rawValue ?? "").trim();
  if (!value || value.startsWith("#") || value.startsWith("data:") || value.startsWith("javascript:") || value.startsWith("mailto:")) {
    return value;
  }

  const absoluteUrl = new URL(value, HN_ORIGIN);
  if (attributeName === "action") {
    return absoluteUrl.href;
  }
  if (absoluteUrl.origin !== HN_ORIGIN) {
    return absoluteUrl.href;
  }
  if (isAssetPath(absoluteUrl.pathname)) {
    return absoluteUrl.href;
  }
  return `${HN_PROXY_PREFIX}${absoluteUrl.pathname}${absoluteUrl.search}${absoluteUrl.hash}`;
}

export function rewriteHackerNewsHtml(html) {
  return String(html ?? "")
    .replace(HN_SCRIPT_PATTERN, "")
    .replace(/\b(href|src|action)=["']([^"']+)["']/gi, (match, attributeName, value) => {
      const nextValue = rewriteHackerNewsAttribute(attributeName.toLowerCase(), value);
      return `${attributeName}="${nextValue}"`;
    });
}

export function buildHackerNewsTargetUrl(requestUrl) {
  const localUrl = new URL(requestUrl, "http://127.0.0.1");
  const suffix = localUrl.pathname.startsWith(HN_PROXY_PREFIX)
    ? localUrl.pathname.slice(HN_PROXY_PREFIX.length)
    : localUrl.pathname;
  const pathname = suffix || "/news";
  return new URL(`${pathname}${localUrl.search}`, HN_ORIGIN);
}
