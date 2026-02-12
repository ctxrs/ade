import { expect } from "../fixtures";

export async function clearOpenedWebSocketUrls(page: any): Promise<void> {
  await page.evaluate(() => {
    (window as any).__ctxE2E?.clearOpenedWebSocketUrls?.();
  });
}

export async function getOpenedWebSocketUrls(page: any): Promise<string[]> {
  return page.evaluate(() => {
    const urls = (window as any).__ctxE2E?.getOpenedWebSocketUrls?.();
    return Array.isArray(urls) ? urls.map((value) => String(value)) : [];
  });
}

const readCanonicalWsBaseUrl = async (page: any): Promise<string | null> => {
  return page.evaluate(() => {
    try {
      const raw = window.sessionStorage.getItem("ctxDaemonConnectionV1");
      if (!raw) return null;
      const parsed = JSON.parse(raw) as { wsBaseUrl?: unknown };
      const value = typeof parsed.wsBaseUrl === "string" ? parsed.wsBaseUrl.trim() : "";
      return value || null;
    } catch {
      return null;
    }
  });
};

export async function expectWsPathOnCanonicalOrigin(page: any, pathFragment: string): Promise<void> {
  const canonicalWsBaseUrl = await readCanonicalWsBaseUrl(page);
  expect(canonicalWsBaseUrl, "Missing canonical ws base URL in session storage.").toBeTruthy();
  const expectedOrigin = new URL(String(canonicalWsBaseUrl)).origin;
  const urls = await getOpenedWebSocketUrls(page);
  let matching = urls.filter((url) => url.includes(pathFragment));
  if (matching.length === 0 && pathFragment.includes("/api/workspaces/")) {
    const workspaceStreamUrl = await page.evaluate(() => {
      return (window as any).__ctxE2E?.workspaceStream?.getCanonicalUrl?.() ?? null;
    });
    if (typeof workspaceStreamUrl === "string" && workspaceStreamUrl.includes(pathFragment)) {
      matching = [workspaceStreamUrl];
    }
  }
  expect(matching.length, `Expected at least one websocket URL containing ${pathFragment}. URLs: ${JSON.stringify(urls, null, 2)}`)
    .toBeGreaterThan(0);
  for (const url of matching) {
    expect(new URL(url).origin).toBe(expectedOrigin);
  }
}
