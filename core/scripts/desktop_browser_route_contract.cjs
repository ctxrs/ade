#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const API_CALL_NAMES = ["apiAny", "daemonFetchRaw", "getDaemonHttpUrl", "getDaemonWsUrl"];
const IDENT_CHAR = /[A-Za-z0-9_$]/;

const DESKTOP_BROWSER_SECRET_POLICY = Object.freeze({
  classification: "desktop_web_owner_transition",
  rationale:
    "The desktop webview uses a derived browser secret as an owner-equivalent daemon credential for /api/* so desktop web routes cannot drift into 401s.",
});

const NON_DESKTOP_PRINCIPAL_ROUTES = Object.freeze([
  {
    method: "POST",
    pattern: ["api", "mobile", "pair"],
    principal: "mobile_pairing",
  },
  {
    method: "POST",
    pattern: ["api", "mobile", "register"],
    principal: "mobile_device_registration",
  },
  {
    method: "POST",
    pattern: ["api", "mobile", "secure"],
    principal: "mobile_secure_envelope",
  },
  {
    method: "GET",
    pattern: ["api", "mobile", "secure", "workspaces", "*", "stream"],
    principal: "mobile_secure_stream",
  },
]);

function repoRootFromCwd() {
  return path.resolve(__dirname, "..", "..");
}

function routeKey(method, routePath) {
  return `${method.toUpperCase()} ${routePath}`;
}

function walkFiles(root, out = []) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "generated" || entry.name === "__fixtures__") continue;
      walkFiles(full, out);
      continue;
    }
    if (!entry.isFile()) continue;
    if (!/\.(ts|tsx)$/.test(entry.name)) continue;
    if (/\.(test|spec)\.(ts|tsx)$/.test(entry.name)) continue;
    out.push(full);
  }
  return out;
}

function skipWhitespace(source, index) {
  let i = index;
  while (i < source.length && /\s/.test(source[i])) i += 1;
  return i;
}

function skipString(source, index) {
  const quote = source[index];
  let i = index + 1;
  while (i < source.length) {
    if (source[i] === "\\") {
      i += 2;
      continue;
    }
    if (source[i] === quote) return i + 1;
    i += 1;
  }
  return source.length;
}

function skipTemplate(source, index) {
  let i = index + 1;
  while (i < source.length) {
    if (source[i] === "\\") {
      i += 2;
      continue;
    }
    if (source[i] === "`") return i + 1;
    if (source[i] === "$" && source[i + 1] === "{") {
      i = findMatching(source, i + 1, "{", "}") + 1;
      continue;
    }
    i += 1;
  }
  return source.length;
}

function findMatching(source, openIndex, openChar, closeChar) {
  let depth = 0;
  for (let i = openIndex; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "'" || ch === '"') {
      i = skipString(source, i) - 1;
      continue;
    }
    if (ch === "`") {
      i = skipTemplate(source, i) - 1;
      continue;
    }
    if (ch === openChar) depth += 1;
    if (ch === closeChar) {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return source.length - 1;
}

function skipTypeArguments(source, index) {
  if (source[index] !== "<") return index;
  let depth = 0;
  for (let i = index; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "'" || ch === '"') {
      i = skipString(source, i) - 1;
      continue;
    }
    if (ch === "`") {
      i = skipTemplate(source, i) - 1;
      continue;
    }
    if (ch === "<") depth += 1;
    if (ch === ">") {
      depth -= 1;
      if (depth === 0) return i + 1;
    }
  }
  return index;
}

function firstArgumentExpression(callSource) {
  let parens = 0;
  let braces = 0;
  let brackets = 0;
  for (let i = 0; i < callSource.length; i += 1) {
    const ch = callSource[i];
    if (ch === "'" || ch === '"') {
      i = skipString(callSource, i) - 1;
      continue;
    }
    if (ch === "`") {
      i = skipTemplate(callSource, i) - 1;
      continue;
    }
    if (ch === "(") parens += 1;
    else if (ch === ")") parens -= 1;
    else if (ch === "{") braces += 1;
    else if (ch === "}") braces -= 1;
    else if (ch === "[") brackets += 1;
    else if (ch === "]") brackets -= 1;
    else if (ch === "," && parens === 0 && braces === 0 && brackets === 0) {
      return callSource.slice(0, i).trim();
    }
  }
  return callSource.trim();
}

function decodeQuotedLiteral(expr) {
  const quote = expr[0];
  let out = "";
  for (let i = 1; i < expr.length; i += 1) {
    const ch = expr[i];
    if (ch === "\\") {
      out += expr[i + 1] ?? "";
      i += 1;
      continue;
    }
    if (ch === quote) return out;
    out += ch;
  }
  return null;
}

function decodeTemplateLiteral(expr) {
  let out = "";
  for (let i = 1; i < expr.length; i += 1) {
    const ch = expr[i];
    if (ch === "\\") {
      out += expr[i + 1] ?? "";
      i += 1;
      continue;
    }
    if (ch === "`") return out;
    if (ch === "$" && expr[i + 1] === "{") {
      out += "*";
      i = findMatching(expr, i + 1, "{", "}");
      continue;
    }
    out += ch;
  }
  return null;
}

function expressionToRawPath(expr) {
  const trimmed = expr.trim();
  const helperMatch = /^withInstallTargetParam\s*\(([\s\S]*)\)$/.exec(trimmed);
  if (helperMatch) {
    return expressionToRawPath(firstArgumentExpression(helperMatch[1]));
  }
  if (trimmed[0] === "'" || trimmed[0] === '"') return decodeQuotedLiteral(trimmed);
  if (trimmed[0] === "`") return decodeTemplateLiteral(trimmed);
  return null;
}

function normalizeRoutePath(rawPath) {
  if (!rawPath) return null;
  const apiIndex = rawPath.indexOf("/api/");
  if (apiIndex < 0) return null;
  let value = rawPath.slice(apiIndex);
  const queryIndex = value.indexOf("?");
  if (queryIndex >= 0) value = value.slice(0, queryIndex);
  while (value.endsWith("*") && !value.endsWith("/*")) value = value.slice(0, -1);
  value = value.replace(/\/+/g, "/").replace(/\/+$/, "");
  return value || null;
}

function routeSegments(routePath) {
  return routePath
    .trim()
    .replace(/^\/+|\/+$/g, "")
    .split("/")
    .filter(Boolean);
}

function routePatternMatches(routePath, pattern) {
  const segments = routeSegments(routePath);
  return (
    segments.length === pattern.length &&
    segments.every((segment, index) => pattern[index] === "*" || pattern[index] === segment)
  );
}

function nonDesktopPrincipalRequirement(method, routePath) {
  const upperMethod = method.toUpperCase();
  return NON_DESKTOP_PRINCIPAL_ROUTES.find(
    (route) => route.method === upperMethod && routePatternMatches(routePath, route.pattern),
  ) ?? null;
}

function extractMethod(callSource) {
  const match = /\bmethod\s*:\s*["'`](GET|POST|PUT|DELETE|PATCH|HEAD)["'`]/i.exec(callSource);
  return match ? match[1].toUpperCase() : "GET";
}

function findApiCallsInSource(source, filePath) {
  const calls = [];
  for (const callName of API_CALL_NAMES) {
    let index = source.indexOf(callName);
    while (index >= 0) {
      const before = index > 0 ? source[index - 1] : "";
      const afterName = source[index + callName.length] ?? "";
      if (IDENT_CHAR.test(before) || IDENT_CHAR.test(afterName)) {
        index = source.indexOf(callName, index + callName.length);
        continue;
      }
      let cursor = skipWhitespace(source, index + callName.length);
      if (source[cursor] === "<") cursor = skipWhitespace(source, skipTypeArguments(source, cursor));
      if (source[cursor] !== "(") {
        index = source.indexOf(callName, index + callName.length);
        continue;
      }
      const end = findMatching(source, cursor, "(", ")");
      const callSource = source.slice(cursor + 1, end);
      const routePath = normalizeRoutePath(expressionToRawPath(firstArgumentExpression(callSource)));
      if (routePath) {
        calls.push({ filePath, callName, method: extractMethod(callSource), path: routePath });
      }
      index = source.indexOf(callName, Math.max(end + 1, index + callName.length));
    }
  }
  return calls;
}

function collectDesktopWebApiCalls(repoRoot) {
  const srcRoot = path.join(repoRoot, "core", "apps", "web", "src");
  const calls = [];
  for (const filePath of walkFiles(srcRoot)) {
    calls.push(...findApiCallsInSource(fs.readFileSync(filePath, "utf8"), path.relative(repoRoot, filePath)));
  }
  calls.sort((a, b) => routeKey(a.method, a.path).localeCompare(routeKey(b.method, b.path)) || a.filePath.localeCompare(b.filePath));
  return calls;
}

function rustFunctionBody(source, fnName) {
  const fnIndex = source.indexOf(`fn ${fnName}`);
  if (fnIndex < 0) return null;
  const openIndex = source.indexOf("{", fnIndex);
  if (openIndex < 0) return null;
  const closeIndex = findMatching(source, openIndex, "{", "}");
  if (closeIndex <= openIndex) return null;
  return source.slice(openIndex + 1, closeIndex).trim();
}

function classifyBrowserSecretPolicySource(source) {
  const body = rustFunctionBody(source, "browser_query_secret_bearer_route_allowed");
  const ownerWide = body?.replace(/\s+/g, " ") === 'req.uri().path().starts_with("/api/")';
  return {
    mode: ownerWide ? "desktop_web_owner" : "unknown",
    classification: ownerWide ? DESKTOP_BROWSER_SECRET_POLICY : null,
  };
}

function loadBrowserSecretPolicy(repoRoot) {
  const sourcePath = path.join(repoRoot, "core", "crates", "ctx-http", "src", "api", "auth", "browser.rs");
  return classifyBrowserSecretPolicySource(fs.readFileSync(sourcePath, "utf8"));
}

function browserSecretAllowsRoute(method, routePath, policy) {
  return (
    policy.mode === "desktop_web_owner" &&
    routePath.startsWith("/api/") &&
    nonDesktopPrincipalRequirement(method, routePath) === null
  );
}

function auditDesktopBrowserRouteContract(options = {}) {
  const repoRoot = options.repoRoot ?? repoRootFromCwd();
  const policy = options.policy ?? loadBrowserSecretPolicy(repoRoot);
  const calls = collectDesktopWebApiCalls(repoRoot);
  const routeMap = new Map();
  for (const call of calls) {
    const key = routeKey(call.method, call.path);
    const existing = routeMap.get(key);
    if (existing) existing.calls.push(call);
    else routeMap.set(key, {
      method: call.method,
      path: call.path,
      allowed: browserSecretAllowsRoute(call.method, call.path, policy),
      nonDesktopPrincipal: nonDesktopPrincipalRequirement(call.method, call.path)?.principal ?? null,
      calls: [call],
    });
  }
  const routes = [...routeMap.values()].sort((a, b) => routeKey(a.method, a.path).localeCompare(routeKey(b.method, b.path)));
  const blocked = routes.filter((route) => !route.allowed);
  const invalidPolicy = policy.mode !== "desktop_web_owner" || !policy.classification?.rationale;
  return { calls, routes, blocked, policy, invalidPolicy };
}

function formatRoute(route) {
  const locations = route.calls.map((call) => `  - ${call.filePath}`).join("\n");
  const principal = route.nonDesktopPrincipal ? `\nrequires principal: ${route.nonDesktopPrincipal}` : "";
  return `${routeKey(route.method, route.path)}${principal}\n${locations}`;
}

function main() {
  const result = auditDesktopBrowserRouteContract();
  if (!result.invalidPolicy && result.blocked.length === 0) {
    console.log(`desktop browser route contract ok (${result.routes.length} routes, owner-wide transition policy)`);
    return;
  }
  if (result.invalidPolicy) {
    console.error("Desktop browser secret policy is not the expected owner-wide transition policy.");
  }
  if (result.blocked.length > 0) {
    console.error("Desktop web routes are not authorized by the browser secret:");
    console.error(result.blocked.map(formatRoute).join("\n\n"));
  }
  process.exitCode = 1;
}

if (require.main === module) main();

module.exports = {
  DESKTOP_BROWSER_SECRET_POLICY,
  auditDesktopBrowserRouteContract,
  browserSecretAllowsRoute,
  classifyBrowserSecretPolicySource,
  collectDesktopWebApiCalls,
  expressionToRawPath,
  findApiCallsInSource,
  loadBrowserSecretPolicy,
  nonDesktopPrincipalRequirement,
  normalizeRoutePath,
  routeKey,
};
