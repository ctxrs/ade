import { defineConfig } from "vite";

import { buildHackerNewsTargetUrl, HN_PROXY_PREFIX, rewriteHackerNewsHtml } from "./hn_proxy.js";

function createHackerNewsProxyMiddleware() {
  return async (request, response, next) => {
    if (!request.url || !request.url.startsWith(HN_PROXY_PREFIX)) {
      next();
      return;
    }

    try {
      const upstreamUrl = buildHackerNewsTargetUrl(request.url);
      const upstream = await fetch(upstreamUrl, {
        headers: {
          "user-agent": "ctx-hn-mobile-demo/1.0",
        },
      });
      const contentType = upstream.headers.get("content-type") || "text/plain; charset=utf-8";

      response.statusCode = upstream.status;
      response.setHeader("cache-control", "no-store");
      response.setHeader("content-type", contentType);

      if (contentType.includes("text/html")) {
        response.end(rewriteHackerNewsHtml(await upstream.text()));
        return;
      }

      response.end(Buffer.from(await upstream.arrayBuffer()));
    } catch (error) {
      next(error);
    }
  };
}

function attachHackerNewsProxy(server) {
  server.middlewares.use(createHackerNewsProxyMiddleware());
}

export default defineConfig({
  plugins: [
    {
      name: "ctx-hn-mobile-hn-proxy",
      configureServer(server) {
        attachHackerNewsProxy(server);
      },
      configurePreviewServer(server) {
        attachHackerNewsProxy(server);
      },
    },
  ],
});
