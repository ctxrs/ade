import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { tmpdir } from "node:os";

import {
  buildVideoEditFfmpegArgs,
  buildVideoEditTimeline,
  normalizeVideoEditRecipe,
} from "./demo_video_edit.mjs";

test("normalizeVideoEditRecipe resolves relative paths and operations", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "demo-video-edit-"));
  try {
    const recipePath = path.join(dir, "recipe.json");
    const inputPath = path.join(dir, "input.mp4");
    writeFileSync(inputPath, "fake", "utf8");
    const recipe = normalizeVideoEditRecipe(
      {
        input_path: "./input.mp4",
        output_path: "./out.mp4",
        operations: [
          { type: "trim_start", seconds: 2 },
          { type: "hold", at_seconds: 4, seconds: 1.5 },
        ],
      },
      recipePath,
    );
    assert.equal(recipe.inputPath, inputPath);
    assert.equal(recipe.outputPath, path.join(dir, "out.mp4"));
    assert.equal(recipe.operations.length, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("buildVideoEditTimeline applies trim, speed, and hold operations", () => {
  const timeline = buildVideoEditTimeline(
    {
      operations: [
        { type: "trim_start", seconds: 2 },
        { type: "trim_end", seconds: 1 },
        { type: "speed", startSeconds: 3, endSeconds: 5, factor: 0.5 },
        { type: "hold", atSeconds: 6, seconds: 2 },
      ],
    },
    10,
  );
  assert.deepEqual(
    timeline.segments,
    [
      { type: "clip", startSeconds: 2, endSeconds: 3, factor: 1 },
      { type: "clip", startSeconds: 3, endSeconds: 5, factor: 0.5 },
      { type: "clip", startSeconds: 5, endSeconds: 6, factor: 1 },
      { type: "hold", atSeconds: 6, seconds: 2 },
      { type: "clip", startSeconds: 6, endSeconds: 9, factor: 1 },
    ],
  );
});

test("buildVideoEditFfmpegArgs builds a concat pipeline", () => {
  const recipe = {
    inputPath: "/tmp/in.mp4",
    outputPath: "/tmp/out.mp4",
  };
  const args = buildVideoEditFfmpegArgs(recipe, {
    segments: [
      { type: "clip", startSeconds: 0, endSeconds: 2, factor: 1 },
      { type: "hold", atSeconds: 2, seconds: 1.25 },
      { type: "clip", startSeconds: 2, endSeconds: 4, factor: 2 },
    ],
  });
  assert.deepEqual(args.slice(0, 4), ["-y", "-i", "/tmp/in.mp4", "-filter_complex"]);
  assert.match(args[4], /trim=start=0:end=2/);
  assert.match(args[4], /tpad=stop_mode=clone:stop_duration=1.25/);
  assert.match(args[4], /setpts=\(PTS-STARTPTS\)\/2/);
  assert.deepEqual(args.slice(-4), ["-an", "-movflags", "+faststart", "/tmp/out.mp4"]);
});
