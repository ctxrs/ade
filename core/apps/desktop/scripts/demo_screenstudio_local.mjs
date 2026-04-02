import { randomBytes } from "node:crypto";
import path from "node:path";

export const SCREEN_STUDIO_APP_PATH = "/Applications/Screen Studio.app";
export const SCREEN_STUDIO_POLYRECORDER_PATH = path.join(
  SCREEN_STUDIO_APP_PATH,
  "Contents/Resources/app.asar.unpacked/bin/prod/polyrecorder-prod",
);
export const SCREEN_STUDIO_LIST_WINDOWS_PATH = path.join(
  SCREEN_STUDIO_APP_PATH,
  "Contents/Resources/app.asar.unpacked/bin/list-windows",
);
export const SCREEN_STUDIO_DEFAULT_PROJECTS_DIR = path.join(process.env.HOME || "", "Screen Studio Projects");
export const SCREEN_STUDIO_DEFAULT_RECORDINGS_DIR = path.join(
  process.env.HOME || "",
  "Library/Application Support/Screen Studio/Screen Studio Recordings",
);
export const SCREEN_STUDIO_DEFAULT_VERSION = "3.6.0-4214";

function assertPositiveFiniteNumber(value, fieldName) {
  const numericValue = Number(value);
  if (!Number.isFinite(numericValue) || numericValue <= 0) {
    throw new Error(`${fieldName} must be a positive finite number`);
  }
  return numericValue;
}

export function createScreenStudioId() {
  return randomBytes(8).toString("base64url").replace(/[-_]/g, "").slice(0, 10);
}

export function normalizeScreenStudioCropRect(bounds) {
  if (!bounds || typeof bounds !== "object") {
    throw new Error("crop bounds are required");
  }
  const x = Math.round(Number(bounds.x));
  const y = Math.round(Number(bounds.y));
  const width = Math.round(assertPositiveFiniteNumber(bounds.width, "crop width"));
  const height = Math.round(assertPositiveFiniteNumber(bounds.height, "crop height"));
  if (!Number.isFinite(x) || !Number.isFinite(y)) {
    throw new Error("crop x and y must be finite numbers");
  }
  return {
    x,
    y,
    width,
    height,
    yAxis: "topBasedIncreasingDownwards",
  };
}

export function buildScreenStudioRecordConfig({ outputDirectory, displayId, cropRect, captureKeyStrokes = true }) {
  const normalizedDisplayId = Number(displayId);
  if (!outputDirectory) {
    throw new Error("outputDirectory is required");
  }
  if (!Number.isInteger(normalizedDisplayId) || normalizedDisplayId <= 0) {
    throw new Error("displayId must be a positive integer");
  }
  return {
    logLevel: "debug",
    outputDirectory,
    channels: [
      { type: "input", captureKeyStrokes: captureKeyStrokes !== false },
      {
        type: "display",
        displayId: normalizedDisplayId,
        excludeFinderDesktopIcons: true,
        cropRect: normalizeScreenStudioCropRect(cropRect),
      },
    ],
  };
}

function buildDefaultProjectConfig(durationMs) {
  return {
    backgroundGradient: {
      start: { x: 0, y: 0 },
      end: { x: 1, y: 1 },
      stops: [
        { color: "#1f3250", at: 0 },
        { color: "#5d7494", at: 1 },
      ],
    },
    backgroundPaddingRatio: 10,
    insetPadding: {
      bottom: 0,
      left: 0,
      right: 0,
      top: 0,
    },
    insetColor: "#000000",
    insetAlpha: 0.18,
    motionBlurAmount: 1,
    motionBlurCursorAmount: 1,
    motionBlurScreenMoveAmount: 1,
    motionBlurScreenZoomAmount: 1,
    cursorSize: 1.5,
    cursorSet: {
      id: "macos-tahoe",
      variants: {},
    },
    cursorRotateOnXMovementRatio: 0.5,
    cursorBaseRotation: 0,
    useDefaultCursorIfAppCursorHasLowResolution: true,
    alwaysUseDefaultCursor: false,
    hideNotMovingCursorAfterMs: null,
    loopCursorPositionBeforeEndMs: null,
    removeCurshorShakeTreshold: 500,
    optimizeOriginalCursorTypes: true,
    clickEffect: null,
    clickSoundEffect: null,
    backgroundColor: "#1f3250",
    backgroundImage: null,
    backgroundType: "system",
    backgroundSystemName: "macOS/sequoia-blue-orange.jpg",
    backgroundBlur: 40,
    hideCursor: false,
    mouseMovementSpring: {
      stiffness: 470,
      damping: 70,
      mass: 3,
    },
    screenMovementSpring: {
      mass: 2.25,
      stiffness: 200,
      damping: 40,
    },
    mouseClickSpring: {
      stiffness: 700,
      damping: 30,
      mass: 1,
    },
    defaultOutputAspectRatio: null,
    windowBorderRadius: 12,
    alwaysKeepZoomedIn: false,
    shadowIntensity: 0.75,
    shadowAngle: 90,
    shadowDistance: 25,
    shadowBlur: 20,
    shadowIsDirectional: false,
    disableMouseMovementSpring: false,
    hideCamera: true,
    mirrorCamera: false,
    cameraRoundness: 0.25,
    cameraSize: 0.35,
    cameraPosition: "bottom-right",
    cameraPositionPoint: {
      x: 1,
      y: 1,
    },
    cameraScaleDuringZoom: 0.7,
    cameraAspectRatio: "original",
    stopCursorMovementInLastPartMs: 0,
    showShortcuts: false,
    hiddenShortcuts: {},
    showShortcutsWithSingleLetters: false,
    showTranscript: false,
    transcriptSizeRatio: 1,
    shortcutsSizeRatio: 1,
    audioVolume: 1,
    muteMicrophone: false,
    muteSystemAudio: false,
    muteExternalDeviceAudio: false,
    improveMicrophoneAudio: false,
    backgroundAudioFileName: null,
    muteBackgroundAudio: false,
    backgroundAudioVolume: 0.05,
    clickSoundEffectVolume: 0.25,
    microphoneInStereoMode: false,
    deviceFrameKey: null,
    defaultLayout: {
      type: "screen-only",
      cameraSize: 0.35,
      cameraPositionPoint: {
        x: 1,
        y: 1,
      },
    },
    enableDeviceMockup: false,
    adjustDeviceFrameToRecordingSize: true,
    recordingRange: [0, durationMs],
    recordingCrop: {
      x: 0,
      y: 0,
      width: 1,
      height: 1,
    },
  };
}

export function buildScreenStudioProjectBundle({
  projectName,
  durationMs,
  projectId = createScreenStudioId(),
  sceneId = createScreenStudioId(),
  sliceId = createScreenStudioId(),
  now = new Date(),
  version = SCREEN_STUDIO_DEFAULT_VERSION,
}) {
  const normalizedDurationMs = assertPositiveFiniteNumber(durationMs, "durationMs");
  const isoNow = new Date(now).toISOString();
  const project = {
    json: {
      id: projectId,
      name: projectName,
      createdAt: isoNow,
      updatedAt: isoNow,
      lastSavedAt: isoNow,
      config: buildDefaultProjectConfig(normalizedDurationMs),
      meta: {
        recordingFlags: [],
      },
      scenes: [
        {
          id: sceneId,
          name: "Default",
          zoomRanges: [],
          type: "recording",
          sessionIndex: 0,
          slices: [
            {
              id: sliceId,
              timeScale: 1,
              sourceStartMs: 0,
              sourceEndMs: normalizedDurationMs,
              volume: 1,
              systemAudioVolume: 1,
              hideCursor: false,
              disableSmoothMouseMovement: false,
              externalDeviceAudioVolume: 1,
            },
          ],
          layouts: [],
          masks: [],
          resolvedTypingSpeedIncreaseSuggestions: [],
        },
      ],
    },
    meta: {
      values: {
        createdAt: ["Date"],
        updatedAt: ["Date"],
        lastSavedAt: ["Date"],
      },
    },
  };
  const meta = {
    json: {
      version,
      requiredVersion: "2.4.0-beta",
      createdAt: isoNow,
    },
    meta: {
      values: {
        createdAt: ["Date"],
      },
    },
  };
  const markers = {
    json: [],
  };
  return { project, meta, markers };
}

export function findScreenStudioCaptureWindow(windows, expectedTitle) {
  if (!Array.isArray(windows)) {
    throw new Error("windows must be an array");
  }
  const normalizedTitle = String(expectedTitle || "").trim().toLowerCase();
  if (!normalizedTitle) {
    throw new Error("expectedTitle is required");
  }
  const matches = windows.filter((windowInfo) => {
    const title = String(windowInfo?.title || "").trim().toLowerCase();
    return title === normalizedTitle;
  });
  if (matches.length !== 1) {
    throw new Error(`expected exactly one Screen Studio capture window titled "${expectedTitle}", found ${matches.length}`);
  }
  const match = matches[0];
  return {
    title: String(match.title || ""),
    appName: String(match.appName || ""),
    bounds: normalizeScreenStudioCropRect(match.bounds),
  };
}

export function extractDotenvVariable(source, key) {
  const pattern = new RegExp(`^${key}=([^\\n\\r]+)$`, "m");
  const match = String(source || "").match(pattern);
  if (!match) {
    return null;
  }
  const rawValue = match[1].trim();
  if (
    (rawValue.startsWith("'") && rawValue.endsWith("'"))
    || (rawValue.startsWith("\"") && rawValue.endsWith("\""))
  ) {
    return rawValue.slice(1, -1);
  }
  return rawValue;
}
