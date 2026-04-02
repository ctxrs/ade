#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

function printHelp() {
  process.stdout.write(`demo_video_edit

Usage:
  node core/apps/desktop/scripts/demo_video_edit.mjs --recipe recipe.json

Recipe schema:
  {
    "input_path": "/abs/in.mp4",
    "output_path": "/abs/out.mp4",
    "operations": [
      { "type": "trim_start", "seconds": 2 },
      { "type": "trim_end", "seconds": 1.25 },
      { "type": "speed", "start_seconds": 3, "end_seconds": 4, "factor": 0.333333 },
      { "type": "hold", "at_seconds": 7.5, "seconds": 2 }
    ]
  }
`);
}

function parseArgs(argv) {
  const out = { recipePath: null };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--recipe") {
      out.recipePath = path.resolve(next);
      index += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  if (!out.recipePath) {
    throw new Error("--recipe is required");
  }
  return out;
}

function assertFinitePositive(value, label, { allowZero = false } = {}) {
  const numeric = Number(value);
  const valid = Number.isFinite(numeric) && (allowZero ? numeric >= 0 : numeric > 0);
  if (!valid) {
    throw new Error(`${label} must be ${allowZero ? "a non-negative" : "a positive"} number`);
  }
  return numeric;
}

function ffprobeDurationSeconds(inputPath) {
  const result = spawnSync(
    "ffprobe",
    [
      "-v",
      "error",
      "-show_entries",
      "format=duration",
      "-of",
      "default=noprint_wrappers=1:nokey=1",
      inputPath,
    ],
    { encoding: "utf8" },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`ffprobe failed for ${inputPath}`);
  }
  return assertFinitePositive(result.stdout.trim(), "input duration");
}

function normalizeVideoEditOperation(operation, index) {
  if (!operation || typeof operation !== "object") {
    throw new Error(`operations[${index}] must be an object`);
  }
  const type = String(operation.type || "").trim();
  if (type === "trim_start" || type === "trim_end") {
    return {
      type,
      seconds: assertFinitePositive(operation.seconds, `operations[${index}].seconds`, { allowZero: true }),
    };
  }
  if (type === "speed") {
    return {
      type,
      startSeconds: assertFinitePositive(operation.start_seconds, `operations[${index}].start_seconds`, { allowZero: true }),
      endSeconds: assertFinitePositive(operation.end_seconds, `operations[${index}].end_seconds`),
      factor: assertFinitePositive(operation.factor, `operations[${index}].factor`),
    };
  }
  if (type === "hold") {
    return {
      type,
      atSeconds: assertFinitePositive(operation.at_seconds, `operations[${index}].at_seconds`, { allowZero: true }),
      seconds: assertFinitePositive(operation.seconds, `operations[${index}].seconds`),
    };
  }
  throw new Error(`unsupported edit operation: ${type || "<missing>"}`);
}

export function normalizeVideoEditRecipe(rawRecipe, fallbackRecipePath = "") {
  if (!rawRecipe || typeof rawRecipe !== "object") {
    throw new Error("recipe must be an object");
  }
  const recipeDir = fallbackRecipePath ? path.dirname(fallbackRecipePath) : process.cwd();
  const inputPath = path.resolve(recipeDir, String(rawRecipe.input_path || "").trim());
  const outputPath = path.resolve(recipeDir, String(rawRecipe.output_path || "").trim());
  if (!existsSync(inputPath)) {
    throw new Error(`input video does not exist: ${inputPath}`);
  }
  if (!outputPath) {
    throw new Error("output_path is required");
  }
  const operations = Array.isArray(rawRecipe.operations) ? rawRecipe.operations : [];
  return {
    inputPath,
    outputPath,
    operations: operations.map((operation, index) => normalizeVideoEditOperation(operation, index)),
  };
}

export function buildVideoEditTimeline(recipe, inputDurationSeconds) {
  const duration = assertFinitePositive(inputDurationSeconds, "input duration");
  let trimStartSeconds = 0;
  let trimEndSeconds = 0;
  const inserts = [];
  const speedRanges = [];
  for (const operation of recipe.operations) {
    if (operation.type === "trim_start") {
      trimStartSeconds += operation.seconds;
    } else if (operation.type === "trim_end") {
      trimEndSeconds += operation.seconds;
    } else if (operation.type === "hold") {
      inserts.push({
        type: "hold",
        atSeconds: operation.atSeconds,
        seconds: operation.seconds,
      });
    } else if (operation.type === "speed") {
      if (operation.endSeconds <= operation.startSeconds) {
        throw new Error("speed operation end_seconds must be greater than start_seconds");
      }
      speedRanges.push({
        startSeconds: operation.startSeconds,
        endSeconds: operation.endSeconds,
        factor: operation.factor,
      });
    }
  }
  if (trimStartSeconds + trimEndSeconds >= duration) {
    throw new Error("trim_start + trim_end removes the entire video");
  }
  const workingStart = trimStartSeconds;
  const workingEnd = duration - trimEndSeconds;
  const boundaries = new Set([workingStart, workingEnd]);
  for (const speedRange of speedRanges) {
    if (speedRange.startSeconds < workingStart || speedRange.endSeconds > workingEnd) {
      throw new Error("speed operation falls outside the retained video range");
    }
    boundaries.add(speedRange.startSeconds);
    boundaries.add(speedRange.endSeconds);
  }
  for (const insert of inserts) {
    if (insert.atSeconds < workingStart || insert.atSeconds > workingEnd) {
      throw new Error("hold operation falls outside the retained video range");
    }
    boundaries.add(insert.atSeconds);
  }
  const sortedBoundaries = [...boundaries].sort((left, right) => left - right);
  const segments = [];
  for (let index = 0; index < sortedBoundaries.length - 1; index += 1) {
    const startSeconds = sortedBoundaries[index];
    const endSeconds = sortedBoundaries[index + 1];
    if (endSeconds <= startSeconds) continue;
    const activeSpeed = speedRanges.find(
      (range) => startSeconds >= range.startSeconds && endSeconds <= range.endSeconds,
    );
    segments.push({
      type: "clip",
      startSeconds,
      endSeconds,
      factor: activeSpeed?.factor ?? 1,
    });
    for (const insert of inserts) {
      if (Math.abs(insert.atSeconds - endSeconds) < 0.000001) {
        segments.push({
          type: "hold",
          atSeconds: insert.atSeconds,
          seconds: insert.seconds,
        });
      }
    }
  }
  return {
    duration,
    workingStart,
    workingEnd,
    segments,
  };
}

export function buildVideoEditFfmpegArgs(recipe, timeline) {
  const filterParts = [];
  const concatInputs = [];
  let outputIndex = 0;
  for (const segment of timeline.segments) {
    if (segment.type === "clip") {
      const label = `v${outputIndex}`;
      filterParts.push(
        `[0:v]trim=start=${segment.startSeconds}:end=${segment.endSeconds},setpts=(PTS-STARTPTS)/${segment.factor}[${label}]`,
      );
      concatInputs.push(`[${label}]`);
      outputIndex += 1;
      continue;
    }
    const label = `v${outputIndex}`;
    filterParts.push(
      `[0:v]trim=start=${segment.atSeconds}:end=${segment.atSeconds + 0.001},setpts=PTS-STARTPTS,tpad=stop_mode=clone:stop_duration=${segment.seconds}[${label}]`,
    );
    concatInputs.push(`[${label}]`);
    outputIndex += 1;
  }
  if (concatInputs.length === 0) {
    throw new Error("recipe produced no output segments");
  }
  filterParts.push(`${concatInputs.join("")}concat=n=${concatInputs.length}:v=1:a=0[vout]`);
  return [
    "-y",
    "-i",
    recipe.inputPath,
    "-filter_complex",
    filterParts.join(";"),
    "-map",
    "[vout]",
    "-an",
    "-movflags",
    "+faststart",
    recipe.outputPath,
  ];
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const rawRecipe = JSON.parse(readFileSync(options.recipePath, "utf8"));
  const recipe = normalizeVideoEditRecipe(rawRecipe, options.recipePath);
  const timeline = buildVideoEditTimeline(recipe, ffprobeDurationSeconds(recipe.inputPath));
  mkdirSync(path.dirname(recipe.outputPath), { recursive: true });
  const ffmpegArgs = buildVideoEditFfmpegArgs(recipe, timeline);
  const result = spawnSync("ffmpeg", ffmpegArgs, { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`ffmpeg failed with status ${result.status}`);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  }
}
