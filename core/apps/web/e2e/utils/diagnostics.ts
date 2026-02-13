import { expect } from "../fixtures";
import type { Page } from "playwright/test";

type E2EDiagnosticEvent = {
  id: number;
  ts: number;
  source: string;
  code: string;
  severity: "info" | "warning" | "error";
  message: string;
  fatal?: boolean;
  context?: Record<string, unknown>;
};

type NoUnexpectedDiagnosticsOptions = {
  includeWarnings?: boolean;
  allowedCodes?: string[];
};

type E2EWindow = Window & {
  __ctxE2E?: {
    clearDiagnostics?: () => void;
    getDiagnostics?: () => E2EDiagnosticEvent[] | unknown;
  };
};

export async function clearDiagnostics(page: Page): Promise<void> {
  await page.evaluate(() => {
    (window as E2EWindow).__ctxE2E?.clearDiagnostics?.();
  });
}

export async function getDiagnostics(page: Page): Promise<E2EDiagnosticEvent[]> {
  return page.evaluate(() => {
    const events = (window as E2EWindow).__ctxE2E?.getDiagnostics?.();
    return Array.isArray(events) ? events : [];
  });
}

export async function expectNoUnexpectedDiagnostics(
  page: Page,
  opts?: NoUnexpectedDiagnosticsOptions,
): Promise<void> {
  const events = await getDiagnostics(page);
  const allowed = new Set(opts?.allowedCodes ?? []);
  const severitySet = new Set<string>(opts?.includeWarnings ? ["error", "warning"] : ["error"]);
  const unexpected = events.filter((event) => severitySet.has(String(event.severity)) && !allowed.has(event.code));
  expect(
    unexpected,
    `Unexpected diagnostics:\n${JSON.stringify(unexpected, null, 2)}`,
  ).toEqual([]);
}
