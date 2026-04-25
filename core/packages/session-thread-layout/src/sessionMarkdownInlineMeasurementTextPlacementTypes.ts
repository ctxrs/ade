import type { SessionMarkdownInlineCodeContinuationDecision, SessionMarkdownInlineCodeSegmentSeamAdjustment } from "./sessionMarkdownInlineMeasurementDebug";
import type { InlineMeasurementLineState } from "./sessionMarkdownInlineMeasurementState";

export type InlineTextPlacementDebug = {
  enabled: boolean;
  continuationDecisions: SessionMarkdownInlineCodeContinuationDecision[];
  segmentSeamAdjustments: SessionMarkdownInlineCodeSegmentSeamAdjustment[];
  appendLineText: (text: string) => void;
};

export type InlineTextPlacementResult = {
  action: "continue" | "break";
  state: InlineMeasurementLineState;
};
