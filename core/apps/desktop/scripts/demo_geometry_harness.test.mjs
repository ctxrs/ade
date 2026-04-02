import test from "node:test";
import assert from "node:assert/strict";

import { summarizeGeometryResults } from "./demo_geometry_harness.mjs";

test("summarizeGeometryResults ranks candidates by expected hits then mean distance", () => {
  const summary = summarizeGeometryResults([
    { candidateId: "a", hitExpectedTarget: true, distance: 12 },
    { candidateId: "a", hitExpectedTarget: true, distance: 10 },
    { candidateId: "b", hitExpectedTarget: true, distance: 4 },
    { candidateId: "b", hitExpectedTarget: false, distance: 2 },
    { candidateId: "c", hitExpectedTarget: false, distance: 1 },
  ]);

  assert.deepEqual(summary, [
    { candidateId: "a", attempts: 2, expectedHits: 2, meanDistance: 11 },
    { candidateId: "b", attempts: 2, expectedHits: 1, meanDistance: 3 },
    { candidateId: "c", attempts: 1, expectedHits: 0, meanDistance: 1 },
  ]);
});
