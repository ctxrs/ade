import { Buffer } from "node:buffer";
import { defineConfig } from "vite";

import {
  buildDemoProfileHtml,
  buildHackerNewsTargetUrl,
  HN_PROXY_PREFIX,
  rewriteHackerNewsHtml,
  rewriteHackerNewsLocation,
  rewriteHackerNewsSetCookie,
  shouldMockDemoProfilePage,
  shouldMockDemoFrontPage,
} from "./hn_proxy.js";

function shouldReadRequestBody(method) {
  return method !== "GET" && method !== "HEAD";
}

async function readRequestBody(request) {
  const chunks = [];

  for await (const chunk of request) {
    chunks.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk);
  }

  return chunks.length ? Buffer.concat(chunks) : null;
}

function buildUpstreamHeaders(request) {
  const headers = new Headers();
  const requestUserAgent = request.headers["user-agent"];
  headers.set(
    "user-agent",
    Array.isArray(requestUserAgent)
      ? requestUserAgent[0]
      : requestUserAgent || "Mozilla/5.0 (iPhone; CPU iPhone OS 18_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Mobile/15E148 Safari/604.1",
  );

  for (const headerName of ["accept", "accept-language", "content-type", "cookie"]) {
    const value = request.headers[headerName];
    if (!value) {
      continue;
    }

    if (Array.isArray(value)) {
      headers.set(headerName, value.join("; "));
      continue;
    }

    headers.set(headerName, value);
  }

  return headers;
}

function copySetCookieHeaders(upstream, response) {
  if (typeof upstream.headers.getSetCookie !== "function") {
    return;
  }

  const cookies = upstream.headers.getSetCookie().map(rewriteHackerNewsSetCookie).filter(Boolean);
  if (cookies.length) {
    response.setHeader("set-cookie", cookies);
  }
}

function createHackerNewsProxyMiddleware() {
  return async (request, response, next) => {
    if (!request.url || !request.url.startsWith(HN_PROXY_PREFIX)) {
      next();
      return;
    }

    try {
      const mockDemoFrontPage = shouldMockDemoFrontPage(request.url);
      const mockDemoProfilePage = shouldMockDemoProfilePage(request.url);
      if (mockDemoProfilePage) {
        response.statusCode = 200;
        response.setHeader("cache-control", "no-store");
        response.setHeader("content-type", "text/html; charset=utf-8");
        response.end(rewriteHackerNewsHtml(buildDemoProfileHtml()));
        return;
      }

      const upstreamUrl = buildHackerNewsTargetUrl(request.url);
      const method = String(request.method || "GET").toUpperCase();
      const body = shouldReadRequestBody(method) ? await readRequestBody(request) : null;
      const upstream = await fetch(upstreamUrl, {
        method,
        body,
        redirect: "manual",
        headers: {
          ...Object.fromEntries(buildUpstreamHeaders(request).entries()),
        },
      });

      response.statusCode = upstream.status;
      response.setHeader("cache-control", "no-store");

      copySetCookieHeaders(upstream, response);

      const location = upstream.headers.get("location");
      if (location) {
        response.setHeader("location", rewriteHackerNewsLocation(location));
      }

      if (upstream.status >= 300 && upstream.status < 400) {
        response.end();
        return;
      }

      const contentType = upstream.headers.get("content-type") || "text/plain; charset=utf-8";
      response.setHeader("content-type", contentType);

      if (contentType.includes("text/html")) {
        response.end(rewriteHackerNewsHtml(await upstream.text(), { mockDemoFrontPage }));
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
