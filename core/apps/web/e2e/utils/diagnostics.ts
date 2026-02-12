import { expect } from "../fixtures";

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

export async function clearDiagnostics(page: any): Promise<void> {
  await page.evaluate(() => {
    (window as any).__ctxE2E?.clearDiagnostics?.();
  });
}

export async function getDiagnostics(page: any): Promise<E2EDiagnosticEvent[]> {
  return page.evaluate(() => {
    const events = (window as any).__ctxE2E?.getDiagnostics?.();
    return Array.isArray(events) ? events : [];
  });
}

export async function expectNoUnexpectedDiagnostics(
  page: any,
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
