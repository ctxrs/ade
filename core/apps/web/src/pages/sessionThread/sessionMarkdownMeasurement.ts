import { clearSessionMarkdownMeasurementCaches as clearMarkdownMeasurementCaches } from "./sessionMarkdownMeasurementCore";
import { clearSessionPlainTextMeasurementCaches } from "./sessionPlainTextMeasurement";
import { clearSessionTextMeasurementCaches } from "./sessionTextMeasurement";

export const SESSION_TRANSCRIPT_LAYOUT_ENGINE_REVISION = "2026-04-13-1";

export function clearSessionMarkdownMeasurementCaches(): void {
  clearMarkdownMeasurementCaches();
  clearSessionPlainTextMeasurementCaches();
  clearSessionTextMeasurementCaches();
}

export { measureSessionMarkdownDocument } from "./sessionMarkdownBlockMeasurement";
