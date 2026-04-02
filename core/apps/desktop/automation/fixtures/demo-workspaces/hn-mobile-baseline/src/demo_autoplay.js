export const DEMO_AUTOPLAY_QUERY_KEY = "demoAutoplay";
export const DEMO_AUTOPLAY_PRESET_QUERY_KEY = "demoAutoplayPreset";
export const DEFAULT_DEMO_AUTOPLAY_ID = "muted-domains";

const DEFAULT_AUTOPLAY_PRESET = "recording";
const MUTED_DOMAINS_AUTOPLAY_VALUE = "x.com\ntwitter.com";

const AUTOPLAY_PRESETS = Object.freeze({
  recording: Object.freeze({
    requireHiddenResume: false,
    initialDwellMs: 1400,
    settingsDwellMs: 950,
    perCharacterMs: 80,
    afterTypingMs: 800,
    afterSaveMs: 1100,
    finalDwellMs: 2700,
  }),
});

function sleep(delayMs) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, delayMs);
  });
}

function buildFrameUrl(frame) {
  try {
    return new URL(frame.contentWindow?.location?.href ?? "");
  } catch {
    return null;
  }
}

function isFeedUrl(frameUrl) {
  return !!frameUrl && frameUrl.pathname === "/proxy/hn/news";
}

function isSettingsUrl(frameUrl) {
  return !!frameUrl && frameUrl.pathname === "/proxy/hn/user";
}

function queryTopRightProfileLink(doc) {
  return doc.querySelector('a[href*="/proxy/hn/user?id="]');
}

function queryHomeLink(doc) {
  return doc.querySelector("b.hnname a");
}

function queryMutedDomainsInput(doc) {
  return doc.querySelector('[data-muted-domains-input="true"]');
}

function queryUpdateButton(doc) {
  return Array.from(doc.querySelectorAll("input[type='submit'], button[type='submit']")).find((element) => {
    const value = "value" in element ? element.value : element.textContent;
    return /update/i.test(String(value ?? ""));
  }) ?? null;
}

async function waitForFrameDocument(frame, predicate, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const doc = frame.contentDocument;
    const frameUrl = buildFrameUrl(frame);
    if (doc?.head && doc.body && predicate(doc, frameUrl)) {
      return { doc, frameUrl };
    }
    await sleep(80);
  }

  throw new Error("Timed out waiting for demo autoplay frame state");
}

async function waitForHiddenResume(frame, timeoutMs = 30_000) {
  const ownerDocument = frame.ownerDocument;
  const deadline = Date.now() + timeoutMs;
  let sawHidden = ownerDocument.visibilityState === "hidden";

  while (Date.now() < deadline) {
    const visibilityState = ownerDocument.visibilityState;
    if (!sawHidden && visibilityState === "hidden") {
      sawHidden = true;
    } else if (sawHidden && visibilityState === "visible") {
      return;
    }
    await sleep(80);
  }

  throw new Error("Timed out waiting for demo autoplay resume");
}

function clickElement(element) {
  element.dispatchEvent(new element.ownerDocument.defaultView.MouseEvent("click", {
    bubbles: true,
    cancelable: true,
    view: element.ownerDocument.defaultView,
  }));
}

function navigateFrame(frame, href) {
  const location = frame.contentWindow?.location;
  if (!location || !href) {
    throw new Error("Unable to navigate demo autoplay frame");
  }
  location.href = href;
}

async function typeIntoTextarea(textarea, value, perCharacterMs) {
  const doc = textarea.ownerDocument;
  const inputEvent = () => new doc.defaultView.Event("input", { bubbles: true, cancelable: false });
  textarea.focus();
  textarea.value = "";
  textarea.dispatchEvent(inputEvent());

  for (const character of value) {
    textarea.value += character;
    textarea.dispatchEvent(inputEvent());
    await sleep(perCharacterMs);
  }
}

export function resolveDemoAutoplayOptions(searchValue) {
  let searchParams;
  try {
    searchParams = new URLSearchParams(searchValue ?? "");
  } catch {
    return null;
  }

  const autoplayId = searchParams.get(DEMO_AUTOPLAY_QUERY_KEY)?.trim() ?? "";
  if (!autoplayId) {
    return null;
  }

  const presetId = searchParams.get(DEMO_AUTOPLAY_PRESET_QUERY_KEY)?.trim() || DEFAULT_AUTOPLAY_PRESET;
  const preset = AUTOPLAY_PRESETS[presetId];
  if (!preset) {
    throw new Error(`Unknown demo autoplay preset: ${presetId}`);
  }

  return Object.freeze({
    id: autoplayId,
    presetId,
    mutedDomainsValue: MUTED_DOMAINS_AUTOPLAY_VALUE,
    ...preset,
  });
}

async function runMutedDomainsAutoplay(frame, options) {
  let state = await waitForFrameDocument(frame, (_doc, frameUrl) => isFeedUrl(frameUrl));
  if (options.requireHiddenResume) {
    await waitForHiddenResume(frame);
  }
  await sleep(options.initialDwellMs);

  const settingsLink = queryTopRightProfileLink(state.doc);
  if (!(settingsLink instanceof state.doc.defaultView.HTMLAnchorElement)) {
    throw new Error("Missing demo autoplay settings link");
  }
  navigateFrame(frame, settingsLink.href);

  state = await waitForFrameDocument(frame, (_doc, frameUrl) => isSettingsUrl(frameUrl));
  await sleep(options.settingsDwellMs);

  const mutedDomainsInput = queryMutedDomainsInput(state.doc);
  if (!(mutedDomainsInput instanceof state.doc.defaultView.HTMLTextAreaElement)) {
    throw new Error("Missing demo autoplay muted-domains input");
  }
  await typeIntoTextarea(mutedDomainsInput, options.mutedDomainsValue, options.perCharacterMs);
  await sleep(options.afterTypingMs);

  const updateButton = queryUpdateButton(state.doc);
  if (!(updateButton instanceof state.doc.defaultView.HTMLElement)) {
    throw new Error("Missing demo autoplay update button");
  }
  const form = mutedDomainsInput.form;
  if (!(form instanceof state.doc.defaultView.HTMLFormElement)) {
    throw new Error("Missing demo autoplay settings form");
  }
  form.requestSubmit(updateButton);

  state = await waitForFrameDocument(
    frame,
    (doc, frameUrl) => isSettingsUrl(frameUrl) && doc !== state.doc,
  );
  await sleep(options.afterSaveMs);

  const homeLink = queryHomeLink(state.doc);
  if (!(homeLink instanceof state.doc.defaultView.HTMLAnchorElement)) {
    throw new Error("Missing demo autoplay home link");
  }
  navigateFrame(frame, homeLink.href);

  await waitForFrameDocument(frame, (_doc, frameUrl) => isFeedUrl(frameUrl));
  await sleep(options.finalDwellMs);
}

export function attachDemoAutoplay(frame, searchValue) {
  const options = resolveDemoAutoplayOptions(searchValue);
  if (!options) {
    return;
  }

  let started = false;
  const maybeStart = () => {
    if (started) {
      return;
    }

    const frameUrl = buildFrameUrl(frame);
    if (!isFeedUrl(frameUrl) || !frame.contentDocument?.body) {
      return;
    }

    started = true;
    runMutedDomainsAutoplay(frame, options).catch((error) => {
      frame.ownerDocument.defaultView?.console?.error("HN demo autoplay failed", error);
    });
  };

  frame.addEventListener("load", maybeStart);
  maybeStart();
}
