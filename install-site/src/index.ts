import { renderInstallScript } from "./install-script.js";

function shell(script: string, status = 200): Response {
  return new Response(script, {
    status,
    headers: {
      "content-type": "text/x-shellscript; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

function html(body: string, status = 200): Response {
  return new Response(body, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

const INSTALL_ROUTES = new Set(["/install", "/install.sh"]);

export default {
  fetch(request: Request): Response {
    const url = new URL(request.url);
    const pathname = url.pathname.replace(/\/+$/, "") || "/";

    if (request.method === "GET" && INSTALL_ROUTES.has(pathname)) {
      return shell(renderInstallScript(), 200);
    }

    if (request.method === "GET" && pathname === "/") {
      return html(
        `<!doctype html><html><body><p>ctx install endpoint</p><p>Use <code>curl -fsSL https://ctx.rs/install | sh</code></p></body></html>`,
      );
    }

    return html("<!doctype html><html><body><h1>Not Found</h1></body></html>", 404);
  },
};
