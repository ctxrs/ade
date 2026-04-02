#!/usr/bin/env node
import { existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

import { runChecked } from "./demo_hn_mobile_artifact_lib.mjs";

export function parseScreenBox(rawValue) {
  const parts = String(rawValue ?? "")
    .split(",")
    .map((part) => Number.parseInt(part.trim(), 10));
  if (parts.length !== 4 || parts.some((value) => !Number.isFinite(value) || value < 0)) {
    throw new Error(`invalid screen box: ${rawValue}`);
  }
  return Object.freeze({
    x: parts[0],
    y: parts[1],
    width: parts[2],
    height: parts[3],
  });
}

export function buildCompositeFfmpegArgs({ inputPath, overlayPath, maskPath, outputPath, screenBox, canvasSize, durationSeconds }) {
  const filterGraph = [
    `[0:v]scale=${screenBox.width}:${screenBox.height}:force_original_aspect_ratio=increase,crop=${screenBox.width}:${screenBox.height}[screen]`,
    `[screen]format=rgba[screen_rgba]`,
    `color=color=black@0.0:size=${canvasSize.width}x${canvasSize.height}[base]`,
    `[base][screen_rgba]overlay=${screenBox.x}:${screenBox.y}[canvas]`,
    `[canvas]format=rgba[canvas_rgba]`,
    `[1:v]format=gray[mask]`,
    `[canvas_rgba][mask]alphamerge[masked]`,
    `[masked][2:v]overlay=0:0:format=auto,format=yuv420p[out]`,
  ].join(";");

  return [
    "-y",
    "-i", inputPath,
    "-loop", "1",
    "-i", maskPath,
    "-loop", "1",
    "-i", overlayPath,
    "-filter_complex", filterGraph,
    "-map", "[out]",
    "-an",
    "-t", String(durationSeconds),
    "-c:v", "libx264",
    "-pix_fmt", "yuv420p",
    "-movflags", "+faststart",
    outputPath,
  ];
}

function parseArgs(argv) {
  const options = {
    inputPath: "",
    overlayPath: "",
    maskPath: "",
    outputPath: "",
    screenBox: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--input") {
      options.inputPath = path.resolve(next);
      index += 1;
    } else if (arg === "--overlay") {
      options.overlayPath = path.resolve(next);
      index += 1;
    } else if (arg === "--mask") {
      options.maskPath = path.resolve(next);
      index += 1;
    } else if (arg === "--output") {
      options.outputPath = path.resolve(next);
      index += 1;
    } else if (arg === "--screen-box") {
      options.screenBox = parseScreenBox(next);
      index += 1;
    }
  }

  if (!options.inputPath || !existsSync(options.inputPath)) {
    throw new Error("--input must point to an existing raw video");
  }
  if (!options.overlayPath || !existsSync(options.overlayPath)) {
    throw new Error("--overlay must point to an existing overlay image");
  }
  if (!options.maskPath || !existsSync(options.maskPath)) {
    throw new Error("--mask must point to an existing mask image");
  }
  if (!options.outputPath) {
    throw new Error("--output is required");
  }
  if (!options.screenBox) {
    throw new Error("--screen-box is required");
  }

  return options;
}

function readImageSize(imagePath) {
  const result = spawnSync("sips", ["-g", "pixelWidth", "-g", "pixelHeight", imagePath], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`sips failed while reading image size for ${imagePath}`);
  }

  const widthMatch = String(result.stdout).match(/pixelWidth:\s*(\d+)/);
  const heightMatch = String(result.stdout).match(/pixelHeight:\s*(\d+)/);
  if (!widthMatch || !heightMatch) {
    throw new Error(`unable to parse image size for ${imagePath}`);
  }

  return Object.freeze({
    width: Number.parseInt(widthMatch[1], 10),
    height: Number.parseInt(heightMatch[1], 10),
  });
}

function readVideoDuration(inputPath) {
  const result = spawnSync("ffprobe", [
    "-v", "error",
    "-show_entries", "format=duration",
    "-of", "default=noprint_wrappers=1:nokey=1",
    inputPath,
  ], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`ffprobe failed while reading video duration for ${inputPath}`);
  }

  const duration = Number.parseFloat(String(result.stdout || "").trim());
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new Error(`unable to parse video duration for ${inputPath}`);
  }
  return duration;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const canvasSize = readImageSize(options.overlayPath);
  const inputDuration = readVideoDuration(options.inputPath);
  mkdirSync(path.dirname(options.outputPath), { recursive: true });
  runChecked("ffmpeg", buildCompositeFfmpegArgs({ ...options, canvasSize, durationSeconds: inputDuration }));
  process.stdout.write(`${JSON.stringify({ status: "ok", output: options.outputPath }, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  }
}
