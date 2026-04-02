import {
  attachHackerNewsEnhancer,
  buildDemoLoginFields,
  buildFeedPreviewPath,
  DEMO_LOGIN_READY_STORAGE_KEY,
} from "./hn_enhancer.js";
import { attachDemoAutoplay } from "./demo_autoplay.js";
import { resolveScreenHeight } from "./viewport.js";

export function resolveDefaultPagePath(searchValue) {
  const searchParams = new URLSearchParams(searchValue ?? "");
  const demoPath = searchParams.get("demoPath") ?? "";
  if (demoPath.startsWith("/proxy/hn/")) {
    return demoPath;
  }
  return buildFeedPreviewPath(searchValue);
}

function buildDefaultPage(searchValue) {
  return Object.freeze({
    path: resolveDefaultPagePath(searchValue),
    title: "Hacker News feed",
  });
}

function syncViewportHeight() {
  const screenHeight = resolveScreenHeight({
    screenHeight: window.screen.height,
    viewportHeight: window.visualViewport?.height,
    innerHeight: window.innerHeight,
  });
  document.documentElement.style.setProperty("--hn-screen-height", `${Math.round(screenHeight)}px`);
}

function hasDemoLoginCookie() {
  return /(?:^|;\s*)user=/.test(document.cookie);
}

function renderShell(root) {
  root.innerHTML = `
    <div class="hn-mobile-app">
      <div class="hn-mobile-loading">Opening Hacker News…</div>
    </div>
  `;
}

async function ensureDemoLoginSession(searchValue) {
  const loginFields = buildDemoLoginFields(searchValue);
  if (
    !loginFields ||
    window.sessionStorage.getItem(DEMO_LOGIN_READY_STORAGE_KEY) === "true" ||
    hasDemoLoginCookie()
  ) {
    window.sessionStorage.setItem(DEMO_LOGIN_READY_STORAGE_KEY, "true");
    return;
  }

  const response = await fetch("/proxy/hn/login", {
    method: "POST",
    headers: {
      "content-type": "application/x-www-form-urlencoded;charset=UTF-8",
    },
    body: new URLSearchParams(loginFields).toString(),
    redirect: "manual",
    credentials: "same-origin",
  });

  const completed =
    response.ok ||
    response.type === "opaqueredirect" ||
    [301, 302, 303, 307, 308].includes(response.status);
  if (!completed) {
    throw new Error(`HN demo login failed with status ${response.status}`);
  }

  window.sessionStorage.setItem(DEMO_LOGIN_READY_STORAGE_KEY, "true");
}

function attachFrame(root) {
  const defaultPage = buildDefaultPage(window.location.search);
  const shell = root.querySelector(".hn-mobile-app");
  if (!(shell instanceof HTMLDivElement)) {
    throw new Error("Missing HN mobile shell");
  }

  shell.replaceChildren();
  const frame = document.createElement("iframe");
  frame.className = "hn-page-frame";
  frame.loading = "eager";
  frame.referrerPolicy = "origin";
  frame.title = defaultPage.title;
  attachHackerNewsEnhancer(frame);
  attachDemoAutoplay(frame, window.location.search);
  frame.src = defaultPage.path;
  shell.append(frame);
}

export async function mountApp(root) {
  syncViewportHeight();
  window.addEventListener("resize", syncViewportHeight);
  window.visualViewport?.addEventListener("resize", syncViewportHeight);
  renderShell(root);
  await ensureDemoLoginSession(window.location.search);
  attachFrame(root);
}
