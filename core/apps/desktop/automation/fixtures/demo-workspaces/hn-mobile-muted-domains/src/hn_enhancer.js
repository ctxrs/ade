import { HN_DEMO_FRONT_PAGE_QUERY_KEY } from "../hn_proxy.js";

const MUTED_DOMAINS_STORAGE_KEY = "hn-mobile.muted-domains";
const SETTINGS_SECTION_ID = "ctx-muted-domains-settings";
const TOAST_ID = "ctx-muted-domains-toast";
const INJECTED_STYLE_ID = "ctx-muted-domains-style";
const DEMO_LOGIN_ATTEMPT_STORAGE_KEY = "hn-mobile.demo-login-attempted";
export const DEMO_LOGIN_READY_STORAGE_KEY = "hn-mobile.demo-login-ready";
const FEED_ANIMATION_DELAY_MS = 180;
const FEED_HIDE_DELAY_MS = 520;
const FEED_PREVIEW_PATH = "/proxy/hn/news";

const X_DOMAIN_FAMILY = new Set(["x.com", "twitter.com"]);

const INJECTED_CSS = `
#${TOAST_ID} {
  position: fixed;
  top: 104px;
  left: 12px;
  right: 12px;
  z-index: 9999;
  padding: 10px 12px;
  border-radius: 14px;
  background: rgba(17, 18, 21, 0.92);
  color: #f7f7f7;
  box-shadow: 0 18px 38px rgba(0, 0, 0, 0.26);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.35;
}

.ctx-muted-domains-input {
  width: min(320px, 100%);
  min-height: 88px;
  padding: 8px 10px;
  border: 1px solid #c8c8c8;
  border-radius: 8px;
  background: #fff;
  color: #111;
  font: 16px/1.4 Menlo, Monaco, monospace;
  resize: vertical;
}

tr.ctx-muted-story-row {
  transition:
    opacity 240ms ease,
    filter 240ms ease,
    transform 240ms ease;
}

tr.ctx-muted-story-row.ctx-muted-story-hiding {
  opacity: 0;
  filter: blur(1.5px);
  transform: translateY(-8px);
}

tr.ctx-muted-story-row.ctx-muted-story-hidden {
  display: none;
}
`;

export function normalizeDomain(rawValue) {
  let value = String(rawValue ?? "").trim().toLowerCase();
  if (!value) {
    return "";
  }

  value = value.replace(/^@+/, "");
  try {
    const parsed = new URL(value.includes("://") ? value : `https://${value}`);
    value = parsed.hostname;
  } catch {
    value = value.split("/")[0]?.split("?")[0]?.split("#")[0] ?? "";
  }

  return value.replace(/^www\./, "").replace(/\.+$/, "");
}

export function parseMutedDomains(rawValue) {
  const seen = new Set();
  const domains = [];

  for (const part of String(rawValue ?? "").split(/[\s,]+/)) {
    const domain = normalizeDomain(part);
    if (!domain || seen.has(domain)) {
      continue;
    }
    seen.add(domain);
    domains.push(domain);
  }

  return domains;
}

export function domainsMatch(candidateDomain, mutedDomains) {
  const normalizedCandidate = normalizeDomain(candidateDomain);
  if (!normalizedCandidate) {
    return false;
  }

  return mutedDomains.some((mutedDomain) => {
    const normalizedMutedDomain = normalizeDomain(mutedDomain);
    if (!normalizedMutedDomain) {
      return false;
    }

    if (normalizedCandidate === normalizedMutedDomain || normalizedCandidate.endsWith(`.${normalizedMutedDomain}`)) {
      return true;
    }

    return isXDomainFamilyMember(normalizedCandidate) && isXDomainFamilyMember(normalizedMutedDomain);
  });
}

function isXDomainFamilyMember(domain) {
  return Array.from(X_DOMAIN_FAMILY).some((familyDomain) => domain === familyDomain || domain.endsWith(`.${familyDomain}`));
}

export function readDemoLoginCredentials(searchValue) {
  let searchParams;
  try {
    searchParams = new URLSearchParams(searchValue ?? "");
  } catch {
    return null;
  }

  const account = searchParams.get("demoLoginUser")?.trim() ?? "";
  const password = searchParams.get("demoLoginPassword") ?? "";
  if (!account || !password) {
    return null;
  }

  return { account, password };
}

export function hasLoginLink(doc) {
  return Array.from(doc.querySelectorAll("a")).some((link) => {
    const href = String(link.getAttribute("href") ?? "");
    if (!href || !/\/proxy\/hn\/login(?:\?|$)|(^|\/)login(?:\?|$)/.test(href)) {
      return false;
    }

    return /login/i.test(String(link.textContent ?? ""));
  });
}

function hasDemoQueryFlag(searchValue, key) {
  try {
    return new URLSearchParams(searchValue ?? "").get(key) === "1";
  } catch {
    return false;
  }
}

export function loadMutedDomains(storage = globalThis.localStorage) {
  try {
    const rawValue = storage?.getItem(MUTED_DOMAINS_STORAGE_KEY);
    const parsed = rawValue ? JSON.parse(rawValue) : [];
    return Array.isArray(parsed) ? parsed.map(normalizeDomain).filter(Boolean) : [];
  } catch {
    return [];
  }
}

export function saveMutedDomains(domains, storage = globalThis.localStorage) {
  storage?.setItem(MUTED_DOMAINS_STORAGE_KEY, JSON.stringify(parseMutedDomains(domains.join("\n"))));
}

function ensureInjectedStyles(doc) {
  if (doc.getElementById(INJECTED_STYLE_ID)) {
    return;
  }

  const style = doc.createElement("style");
  style.id = INJECTED_STYLE_ID;
  style.textContent = INJECTED_CSS;
  doc.head.append(style);
}

function removeToast(doc) {
  doc.getElementById(TOAST_ID)?.remove();
}

function showToast(doc, message) {
  removeToast(doc);
  const toast = doc.createElement("div");
  toast.id = TOAST_ID;
  toast.textContent = message;
  doc.body.append(toast);
}

function findSettingsForm(doc) {
  const updateButton = Array.from(doc.querySelectorAll("input[type='submit'], button[type='submit']")).find((element) => {
    const value = "value" in element ? element.value : element.textContent;
    return /update/i.test(String(value ?? ""));
  });
  return updateButton?.closest("form") ?? doc.querySelector("form");
}

function findSettingsInsertionRow(doc, settingsTable) {
  const rows = Array.from(settingsTable?.querySelectorAll("tr") ?? []);
  return rows.find((candidateRow) => {
    const labelCell = candidateRow.querySelector("td");
    return /showdead:/i.test(String(labelCell?.textContent ?? ""));
  }) ?? null;
}

export function buildDemoLoginGoto(searchValue) {
  const upstreamUrl = new URL("/news", "https://news.ycombinator.com");
  upstreamUrl.searchParams.set(HN_DEMO_FRONT_PAGE_QUERY_KEY, "1");
  return `${upstreamUrl.pathname.replace(/^\//, "")}${upstreamUrl.search}`;
}

export function buildDemoLoginFields(searchValue) {
  const credentials = readDemoLoginCredentials(searchValue);
  if (!credentials) {
    return null;
  }

  return {
    acct: credentials.account,
    pw: credentials.password,
    goto: buildDemoLoginGoto(searchValue),
  };
}

function submitDemoLoginForm(doc, credentials, searchValue) {
  const form = doc.createElement("form");
  form.method = "POST";
  form.action = "/proxy/hn/login";
  form.style.display = "none";

  for (const [name, value] of Object.entries(buildDemoLoginFields(searchValue) ?? {
    acct: credentials.account,
    pw: credentials.password,
    goto: buildDemoLoginGoto(searchValue),
  })) {
    const input = doc.createElement("input");
    input.type = "hidden";
    input.name = name;
    input.value = value;
    form.append(input);
  }

  doc.body.append(form);
  form.submit();
}

function maybeAutoLogin(doc) {
  const frameWindow = doc.defaultView;
  const topSearch = frameWindow?.top?.location?.search ?? "";
  const credentials = readDemoLoginCredentials(topSearch);
  if (!credentials || !hasLoginLink(doc)) {
    return;
  }

  const storage = frameWindow.sessionStorage;
  if (storage?.getItem(DEMO_LOGIN_ATTEMPT_STORAGE_KEY) === "true") {
    return;
  }

  storage?.setItem(DEMO_LOGIN_ATTEMPT_STORAGE_KEY, "true");
  submitDemoLoginForm(doc, credentials, topSearch);
}

export function buildMutedDomainsSettingsRowMarkup() {
  return `
    <td valign="top">muted domains:</td>
    <td>
      <textarea class="ctx-muted-domains-input" data-muted-domains-input="true" spellcheck="false"></textarea>
    </td>
  `;
}

export function buildFeedPreviewPath(searchValue) {
  const previewUrl = new URL(FEED_PREVIEW_PATH, "http://127.0.0.1");
  previewUrl.searchParams.set(HN_DEMO_FRONT_PAGE_QUERY_KEY, "1");
  return `${previewUrl.pathname}${previewUrl.search}`;
}

export function buildMutedDomainsToastMessage(storyCount) {
  const noun = storyCount === 1 ? "story" : "stories";
  return `Hid ${storyCount} ${noun} from muted domains.`;
}

function enhanceSettingsPage(doc) {
  if (doc.getElementById(SETTINGS_SECTION_ID)) {
    return;
  }

  const form = findSettingsForm(doc);
  const settingsTable = form?.querySelector("table");
  const row = doc.createElement("tr");
  row.id = SETTINGS_SECTION_ID;
  row.innerHTML = buildMutedDomainsSettingsRowMarkup();
  let input = null;

  if (form && settingsTable) {
    const insertionRow = findSettingsInsertionRow(doc, settingsTable);
    if (insertionRow) {
      settingsTable.insertBefore(row, insertionRow);
    } else {
      settingsTable.append(row);
    }
    input = row.querySelector("[data-muted-domains-input='true']");
    if (!(input instanceof doc.defaultView.HTMLTextAreaElement)) {
      return;
    }

    input.value = loadMutedDomains(doc.defaultView.localStorage).join("\n");
    const persistMutedDomains = () => {
      saveMutedDomains(parseMutedDomains(input.value), doc.defaultView.localStorage);
    };
    input.addEventListener("input", persistMutedDomains);

    if (form.dataset.ctxMutedDomainsBound === "true") {
      return;
    }

    form.dataset.ctxMutedDomainsBound = "true";
    form.addEventListener("submit", () => {
      persistMutedDomains();
    });
    return;
  }

  let frameUrl;
  try {
    frameUrl = new URL(doc.defaultView.location.href);
  } catch {
    return;
  }
  if (frameUrl.pathname !== "/proxy/hn/user") {
    return;
  }

  const profileCell = doc.querySelector("#bigbox > td");
  if (!(profileCell instanceof doc.defaultView.HTMLTableCellElement)) {
    return;
  }

  const localSettingsTable = doc.createElement("table");
  localSettingsTable.border = "0";
  localSettingsTable.style.marginTop = "8px";
  localSettingsTable.append(row);
  profileCell.append(localSettingsTable);

  input = row.querySelector("[data-muted-domains-input='true']");
  if (!(input instanceof doc.defaultView.HTMLTextAreaElement)) {
    return;
  }

  input.value = loadMutedDomains(doc.defaultView.localStorage).join("\n");
  input.addEventListener("input", () => {
    saveMutedDomains(parseMutedDomains(input.value), doc.defaultView.localStorage);
  });
}

function collectStoryRowGroup(storyRow) {
  const rowGroup = [storyRow];
  let cursor = storyRow.nextElementSibling;

  for (let index = 0; index < 2 && cursor; index += 1) {
    rowGroup.push(cursor);
    cursor = cursor.nextElementSibling;
  }

  return rowGroup.filter(Boolean);
}

function findStoryDomain(storyRow) {
  return normalizeDomain(storyRow.querySelector(".sitestr")?.textContent ?? "");
}

function hideRowGroup(rowGroup) {
  rowGroup.forEach((row) => {
    row.classList.add("ctx-muted-story-row");
  });

  window.setTimeout(() => {
    rowGroup.forEach((row) => {
      row.classList.add("ctx-muted-story-hiding");
    });
  }, FEED_ANIMATION_DELAY_MS);

  window.setTimeout(() => {
    rowGroup.forEach((row) => {
      row.classList.add("ctx-muted-story-hidden");
    });
  }, FEED_HIDE_DELAY_MS);
}

function enhanceFeedPage(doc) {
  const mutedDomains = loadMutedDomains(doc.defaultView.localStorage);
  removeToast(doc);

  if (!mutedDomains.length) {
    return;
  }

  const matchingRows = Array.from(doc.querySelectorAll("tr.athing")).filter((storyRow) => domainsMatch(findStoryDomain(storyRow), mutedDomains));
  if (!matchingRows.length) {
    return;
  }

  doc.defaultView?.scrollTo(0, 0);

  matchingRows.forEach((storyRow) => {
    hideRowGroup(collectStoryRowGroup(storyRow));
  });

  window.setTimeout(() => {
    doc.defaultView?.scrollTo(0, 0);
  }, FEED_HIDE_DELAY_MS + 40);

  showToast(doc, buildMutedDomainsToastMessage(matchingRows.length));
}

function enhanceHackerNewsDocument(doc) {
  let frameUrl;
  try {
    frameUrl = new URL(doc.defaultView.location.href);
  } catch {
    return;
  }

  if (!frameUrl.pathname.startsWith("/proxy/hn")) {
    return;
  }

  maybeAutoLogin(doc);
  ensureInjectedStyles(doc);
  enhanceSettingsPage(doc);
  enhanceFeedPage(doc);
}

export function attachHackerNewsEnhancer(frame) {
  const applyEnhancements = () => {
    try {
      const doc = frame.contentDocument;
      if (!doc?.head || !doc.body) {
        return;
      }
      enhanceHackerNewsDocument(doc);
    } catch {
      // The frame may have navigated cross-origin to an external story page.
    }
  };

  frame.addEventListener("load", applyEnhancements);
  applyEnhancements();
}
