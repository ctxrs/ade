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
import aiderLogo from "../assets/harness-logos/aider.png";
import cagentLogo from "../assets/harness-logos/cagent.png";
import codyLogo from "../assets/harness-logos/cody.png";
import continueLogo from "../assets/harness-logos/continue.png";
import codeAssistantLogo from "../assets/harness-logos/code-assistant.png";
import junieLogo from "../assets/harness-logos/junie.png";
import kiloLogo from "../assets/harness-logos/kilo.png";
import openhandsLogo from "../assets/harness-logos/openhands.png";
import plandexLogo from "../assets/harness-logos/plandex.png";
import sweAgentLogo from "../assets/harness-logos/swe-agent.png";
import tabbyLogo from "../assets/harness-logos/tabby.png";

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

  // Additional harnesses from specs/21_harness_providers.md
  { id: "aider", label: "Aider", logoSrc: aiderLogo },
  { id: "continue", label: "Continue", logoSrc: continueLogo },
  { id: "plandex", label: "Plandex", logoSrc: plandexLogo },
  { id: "openhands", label: "OpenHands", logoSrc: openhandsLogo },
  { id: "swe-agent", label: "SWE-agent", logoSrc: sweAgentLogo },
  { id: "tabby", label: "Tabby", logoSrc: tabbyLogo },
  { id: "cagent", label: "cagent", logoSrc: cagentLogo },
  { id: "code-assistant", label: "code-assistant", logoSrc: codeAssistantLogo },
  { id: "kilo", label: "Kilo Code", logoSrc: kiloLogo },
  { id: "cody", label: "Cody", logoSrc: codyLogo },
  { id: "junie", label: "JetBrains Junie", logoSrc: junieLogo },
];
