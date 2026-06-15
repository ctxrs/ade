import { renderControlPlaneInstallScript } from "./control-plane-install-script.js";
import { renderInstallScript } from "./install-script.js";
import { renderUninstallScript } from "./uninstall-script.js";

const DOWNLOAD_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;
const ATTRIBUTION_VALUE_PATTERN = /[^A-Za-z0-9._:-]/g;
const MAX_ATTRIBUTION_VALUE_LENGTH = 120;

function shell(script, status = 200) {
  return new Response(script, {
    status,
    headers: {
      "content-type": "text/x-shellscript; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

function html(body, status = 200) {
  return new Response(body, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

const ADE_INSTALL_ROUTES = new Set(["/install", "/install.sh", "/install/ade", "/install/ade.sh"]);
const CONTROL_PLANE_INSTALL_ROUTES = new Set(["/install/control-plane", "/install/control-plane.sh"]);
const UNINSTALL_ROUTES = new Set(["/uninstall", "/uninstall.sh"]);

function normalizeDownloadId(raw) {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!DOWNLOAD_ID_PATTERN.test(trimmed)) return null;
  return trimmed;
}

function normalizeAttributionValue(raw) {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const normalized = trimmed.replace(ATTRIBUTION_VALUE_PATTERN, "_").slice(0, MAX_ATTRIBUTION_VALUE_LENGTH);
  return normalized || null;
}

function normalizeReferrerDomain(raw) {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = /^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(trimmed)
      ? new URL(trimmed)
      : new URL(`https://${trimmed}`);
    const hostname = parsed.hostname.trim().toLowerCase().replace(/\.+$/, "");
    return hostname || null;
  } catch {
    return null;
  }
}

export default {
  fetch(request) {
    const url = new URL(request.url);
    const pathname = url.pathname.replace(/\/+$/, "") || "/";

    if (request.method === "GET" && ADE_INSTALL_ROUTES.has(pathname)) {
      return shell(renderInstallScript({
        downloadId: normalizeDownloadId(url.searchParams.get("ctx_download_id")) ?? crypto.randomUUID(),
        referrerDomain: normalizeReferrerDomain(url.searchParams.get("referrer_domain"))
          ?? normalizeReferrerDomain(request.headers.get("referer")),
        utmSource: normalizeAttributionValue(url.searchParams.get("utm_source")),
        utmMedium: normalizeAttributionValue(url.searchParams.get("utm_medium")),
        utmCampaign: normalizeAttributionValue(url.searchParams.get("utm_campaign")),
      }), 200);
    }

    if (request.method === "GET" && CONTROL_PLANE_INSTALL_ROUTES.has(pathname)) {
      return shell(renderControlPlaneInstallScript(), 200);
    }

    if (request.method === "GET" && UNINSTALL_ROUTES.has(pathname)) {
      return shell(renderUninstallScript(), 200);
    }

    if (request.method === "GET" && pathname === "/") {
      return html(
        `<!doctype html><html><body><p>ctx install endpoint</p><p>ADE: <code>curl -fsSL https://ctx.rs/install | sh</code></p><p>Control plane: <code>curl -fsSL https://ctx.rs/install/control-plane | sh</code></p><p>Uninstall: <code>curl -fsSL https://ctx.rs/uninstall | sh</code></p></body></html>`,
      );
    }

    return html("<!doctype html><html><body><h1>Not Found</h1></body></html>", 404);
  },
};
