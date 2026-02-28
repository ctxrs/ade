import ampLogo from "../assets/emdash-logos/ampcode.png";
import augmentLogo from "../assets/emdash-logos/augmentcode.png";
import claudeLogo from "../assets/emdash-logos/claude.png";
import clineLogo from "../assets/emdash-logos/cline.png";
import cursorLogo from "../assets/emdash-logos/cursorlogo.png";
import droidLogo from "../assets/emdash-logos/factorydroid.png";
import geminiLogo from "../assets/emdash-logos/gemini.png";
import copilotLogo from "../assets/emdash-logos/ghcopilot.png";
import gooseLogo from "../assets/emdash-logos/goose.png";
import kimiLogo from "../assets/emdash-logos/kimi.png";
import mistralLogo from "../assets/emdash-logos/mistral.png";
import codexLogo from "../assets/emdash-logos/openai.png";
import opencodeLogo from "../assets/emdash-logos/opencode.png";
import qwenLogo from "../assets/emdash-logos/qwen.png";
import openhandsLogo from "../assets/harness-logos/openhands.png";
import piLogo from "../assets/harness-logos/pi.svg";

export type HarnessCatalogEntry = {
  id: string;
  label: string;
  logoSrc: string;
  invertInDark?: boolean;
  invertInLight?: boolean;
};

// Curated list of popular harnesses shown in the selector. Entries without a logo use the fallback slot.
export const HARNESS_CATALOG: HarnessCatalogEntry[] = [
  { id: "claude-crp", label: "Claude Code", logoSrc: claudeLogo },
  { id: "codex", label: "Codex", logoSrc: codexLogo, invertInDark: true },
  { id: "qwen", label: "Qwen Code", logoSrc: qwenLogo },
  { id: "cursor", label: "Cursor", logoSrc: cursorLogo, invertInDark: true },
  { id: "pi", label: "Pi", logoSrc: piLogo, invertInLight: true },
  { id: "amp", label: "Amp", logoSrc: ampLogo },
  { id: "droid", label: "Droid", logoSrc: droidLogo, invertInDark: true },
  { id: "gemini", label: "Gemini", logoSrc: geminiLogo },
  { id: "copilot", label: "Copilot", logoSrc: copilotLogo, invertInDark: true },
  { id: "opencode", label: "OpenCode", logoSrc: opencodeLogo, invertInDark: true },
  { id: "cline", label: "Cline", logoSrc: clineLogo },
  { id: "mistral", label: "Mistral Vibe", logoSrc: mistralLogo },
  { id: "auggie", label: "Auggie", logoSrc: augmentLogo, invertInDark: true },
  { id: "goose", label: "Goose", logoSrc: gooseLogo },
  { id: "kimi", label: "Kimi", logoSrc: kimiLogo },

  // Additional harnesses from specs/21_harness_providers.md
  { id: "openhands", label: "OpenHands", logoSrc: openhandsLogo },
];

export const UNSUPPORTED_HARNESS_IDS = new Set(["codebuff", "charm", "aider", "kilo", "junie"]);

export const HARNESS_LOGO_SRCS = Array.from(
  new Set(HARNESS_CATALOG.map((entry) => entry.logoSrc).filter(Boolean)),
);

let harnessLogosPreloaded = false;

export function preloadHarnessLogos() {
  if (harnessLogosPreloaded || typeof Image === "undefined") return;
  harnessLogosPreloaded = true;
  HARNESS_LOGO_SRCS.forEach((src) => {
    const img = new Image();
    img.src = src;
  });
}
