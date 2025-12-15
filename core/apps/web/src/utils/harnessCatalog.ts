import ampLogo from "../assets/emdash-logos/ampcode.png";
import atlassianLogo from "../assets/emdash-logos/atlassian.png";
import augmentLogo from "../assets/emdash-logos/augmentcode.png";
import charmLogo from "../assets/emdash-logos/charm.png";
import claudeLogo from "../assets/emdash-logos/claude.png";
import clineLogo from "../assets/emdash-logos/cline.png";
import codebuffLogo from "../assets/emdash-logos/codebuff.png";
import cursorLogo from "../assets/emdash-logos/cursorlogo.png";
import droidLogo from "../assets/emdash-logos/factorydroid.png";
import geminiLogo from "../assets/emdash-logos/gemini.png";
import copilotLogo from "../assets/emdash-logos/ghcopilot.png";
import gooseLogo from "../assets/emdash-logos/goose.png";
import kimiLogo from "../assets/emdash-logos/kimi.png";
import kiroLogo from "../assets/emdash-logos/kiro.png";
import mistralLogo from "../assets/emdash-logos/mistral.png";
import codexLogo from "../assets/emdash-logos/openai.png";
import opencodeLogo from "../assets/emdash-logos/opencode.png";
import qwenLogo from "../assets/emdash-logos/qwen.png";

export type HarnessCatalogEntry = {
  id: string;
  label: string;
  logoSrc: string;
  invertInDark?: boolean;
};

// Mirrors Emdash provider list so we can reuse the same icons and naming.
export const HARNESS_CATALOG: HarnessCatalogEntry[] = [
  { id: "claude", label: "Claude Code", logoSrc: claudeLogo },
  { id: "codex", label: "Codex", logoSrc: codexLogo, invertInDark: true },
  { id: "qwen", label: "Qwen Code", logoSrc: qwenLogo },
  { id: "cursor", label: "Cursor", logoSrc: cursorLogo, invertInDark: true },
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
  { id: "kiro", label: "Kiro", logoSrc: kiroLogo },
  { id: "codebuff", label: "Codebuff", logoSrc: codebuffLogo },
  { id: "charm", label: "Charm", logoSrc: charmLogo },
  { id: "rovo", label: "Rovo Dev", logoSrc: atlassianLogo },
];

