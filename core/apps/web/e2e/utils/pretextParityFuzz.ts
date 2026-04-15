import type {
  AssistantParityParams,
  AssistantStreamingParityParams,
  MarkdownSample,
  MessageParityParams,
  TurnHeaderParityParams,
} from "./pretextParity";

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

const WORDS = [
  "agent",
  "browser",
  "buffer",
  "command",
  "composer",
  "context",
  "delta",
  "deterministic",
  "entry",
  "fragment",
  "header",
  "inline",
  "layout",
  "marker",
  "message",
  "padding",
  "parity",
  "pretext",
  "probe",
  "render",
  "session",
  "shell",
  "stream",
  "summary",
  "thread",
  "token",
  "turn",
  "virtualizer",
] as const;

const PATH_SEGMENTS = [
  "core",
  "apps",
  "web",
  "src",
  "pages",
  "sessionThread",
  "workbenchShell",
  "sessionMarkdownMeasurement.ts",
  "pretextVirtualizerRowLayout.ts",
  "sessionThreadDomMeasurement.tsx",
  "e2e",
  "fixtures",
  "inline-code",
  "turn-header",
  "blockquote",
  "table",
] as const;

const URL_SEGMENTS = [
  "docs",
  "virtualizer",
  "measurement",
  "parity",
  "assistant",
  "webkit",
  "chromium",
  "transcript",
  "inline-code",
  "streaming-tail",
] as const;

const EMOJI = ["🙂", "⚙️", "🧪", "📏"] as const;
const CJK = ["你好 世界", "測試 佈局", "段落 換行"] as const;

const WIDTHS = [472, 540, 620, 788] as const;

export type GeneratedMessageSample = {
  name: string;
  params: MessageParityParams;
};

export type GeneratedAssistantSample = {
  name: string;
  params: AssistantParityParams;
};

export type GeneratedAssistantStreamingSample = {
  name: string;
  params: AssistantStreamingParityParams;
  finalContent: string;
};

export type GeneratedTurnHeaderSample = {
  name: string;
  params: TurnHeaderParityParams;
};

export type GeneratedPretextParityFuzzCorpus = {
  seed: number;
  widths: readonly number[];
  markdownSamples: readonly MarkdownSample[];
  messageSamples: readonly GeneratedMessageSample[];
  assistantSamples: readonly GeneratedAssistantSample[];
  assistantStreamingSamples: readonly GeneratedAssistantStreamingSample[];
  turnHeaderSamples: readonly GeneratedTurnHeaderSample[];
};

type GeneratedBlock = {
  label: string;
  text: string;
};

class SeededRandom {
  private state: number;

  constructor(seed: number) {
    this.state = seed >>> 0;
  }

  next(): number {
    let state = this.state + 0x6d2b79f5;
    state = Math.imul(state ^ (state >>> 15), state | 1);
    state ^= state + Math.imul(state ^ (state >>> 7), state | 61);
    this.state = state ^ (state >>> 14);
    return (this.state >>> 0) / 4294967296;
  }

  int(min: number, max: number): number {
    return min + Math.floor(this.next() * (max - min + 1));
  }

  bool(probability = 0.5): boolean {
    return this.next() < probability;
  }

  pick<T>(values: readonly T[]): T {
    return values[this.int(0, values.length - 1)]!;
  }
}

function slugify(value: string): string {
  return value.replace(/[^a-z0-9]+/gi, "-").replace(/^-+|-+$/g, "").toLowerCase();
}

function capitalize(value: string): string {
  return value.length === 0 ? value : `${value[0]!.toUpperCase()}${value.slice(1)}`;
}

function repeatJoin(count: number, build: () => string, separator: string): string {
  return Array.from({ length: count }, build).join(separator);
}

function generateWords(rng: SeededRandom, min = 2, max = 5): string {
  return repeatJoin(rng.int(min, max), () => rng.pick(WORDS), " ");
}

function generatePathToken(rng: SeededRandom): string {
  return repeatJoin(rng.int(4, 7), () => rng.pick(PATH_SEGMENTS), "/");
}

function generateCommandToken(rng: SeededRandom): string {
  const command = rng.pick(["pnpm", "cargo", "ctx", "git"] as const);
  if (command === "pnpm") {
    return `pnpm -C core/apps/web test:e2e:pretext:${rng.pick(["parity:webkit", "parity:chromium", "corpus:webkit", "guardrail"] as const)}`;
  }
  if (command === "cargo") {
    return `cargo test -p ${rng.pick(["codex-crp", "ctx-http", "ctx-store"] as const)}`;
  }
  if (command === "git") {
    return `git ${rng.pick(["status", "diff --stat", "rev-parse HEAD"] as const)}`;
  }
  return `ctx ${rng.pick(["serve", "task list", "run start --mode sandbox"] as const)}`;
}

function generateUrl(rng: SeededRandom): string {
  return `https://example.com/${repeatJoin(rng.int(2, 4), () => rng.pick(URL_SEGMENTS), "/")}?ref=${rng.int(100, 999)}`;
}

function generateInlineCode(rng: SeededRandom): string {
  return `\`${rng.bool(0.5) ? generatePathToken(rng) : generateCommandToken(rng)}\``;
}

function generateLink(rng: SeededRandom): string {
  return `[${generateWords(rng, 1, 3)}](${generateUrl(rng)})`;
}

function generateInlineFragment(rng: SeededRandom): string {
  switch (rng.int(0, 7)) {
    case 0:
      return generateWords(rng, 2, 5);
    case 1:
      return generateInlineCode(rng);
    case 2:
      return generateLink(rng);
    case 3:
      return `*${generateWords(rng, 1, 3)}*`;
    case 4:
      return `**${generateWords(rng, 1, 3)}**`;
    case 5:
      return `~~${generateWords(rng, 1, 2)}~~`;
    case 6:
      return `${rng.pick(EMOJI)} ${rng.pick(CJK)}`;
    default:
      return generateWords(rng, 3, 6);
  }
}

function generateSentence(rng: SeededRandom, minFragments = 5, maxFragments = 9): string {
  const fragments = [capitalize(generateWords(rng, 2, 4))];
  const fragmentCount = rng.int(minFragments, maxFragments);
  for (let index = 1; index < fragmentCount; index += 1) {
    fragments.push(generateInlineFragment(rng));
  }
  return `${fragments.join(" ")}${rng.pick([".", ".", ".", ";", ":"] as const)}`;
}

function prefixLines(prefix: string, text: string): string {
  return text
    .split("\n")
    .map((line) => (line.length > 0 ? `${prefix}${line}` : prefix.trimEnd()))
    .join("\n");
}

function generateParagraphBlock(rng: SeededRandom): GeneratedBlock {
  return {
    label: "paragraph",
    text: generateSentence(rng),
  };
}

function generateHardBreakBlock(rng: SeededRandom): GeneratedBlock {
  return {
    label: "hard-break",
    text: `${generateSentence(rng, 4, 7)}\n${generateSentence(rng, 4, 7)}`,
  };
}

function generateListBlock(rng: SeededRandom): GeneratedBlock {
  const itemCount = rng.int(2, 4);
  return {
    label: "list",
    text: repeatJoin(itemCount, () => `- ${generateSentence(rng, 4, 7)}`, "\n"),
  };
}

function generateNestedListBlock(rng: SeededRandom): GeneratedBlock {
  return {
    label: "nested-list",
    text: `- ${generateSentence(rng, 4, 6)}\n  - ${generateSentence(rng, 4, 6)}\n  - ${generateSentence(rng, 4, 6)}`,
  };
}

function generateBlockquoteBlock(rng: SeededRandom): GeneratedBlock {
  const inner = `${generateSentence(rng, 4, 7)}\n\n${generateSentence(rng, 4, 7)}`;
  return {
    label: "blockquote",
    text: prefixLines("> ", inner),
  };
}

function generateFenceBlock(rng: SeededRandom): GeneratedBlock {
  const language = rng.pick(["ts", "bash", "json"] as const);
  const lines =
    language === "ts"
      ? [
          `const token = '${slugify(generateWords(rng, 2, 3))}';`,
          `console.log('${generatePathToken(rng)}', token);`,
        ]
      : language === "json"
        ? [`{`, `  "path": "${generatePathToken(rng)}",`, `  "command": "${generateCommandToken(rng)}"`, `}`]
        : [generateCommandToken(rng), generateCommandToken(rng)];
  return {
    label: "fence",
    text: `\`\`\`${language}\n${lines.join("\n")}\n\`\`\``,
  };
}

function generateTableBlock(rng: SeededRandom): GeneratedBlock {
  const rows = Array.from({ length: rng.int(2, 3) }, () => [
    generateWords(rng, 1, 2),
    generateInlineCode(rng),
    generateWords(rng, 4, 8),
  ]);
  return {
    label: "table",
    text: [
      "| Kind | Token | Note |",
      "|---|---|---|",
      ...rows.map((row) => `| ${row[0]} | ${row[1]} | ${row[2]} |`),
    ].join("\n"),
  };
}

function generateHeadingBlock(rng: SeededRandom): GeneratedBlock {
  return {
    label: "heading",
    text: `## ${capitalize(generateWords(rng, 2, 4))}\n\n${generateSentence(rng, 4, 7)}`,
  };
}

function generateMarkdownDocument(rng: SeededRandom): GeneratedBlock[] {
  const builders = [
    generateParagraphBlock,
    generateHardBreakBlock,
    generateListBlock,
    generateNestedListBlock,
    generateBlockquoteBlock,
    generateFenceBlock,
    generateTableBlock,
    generateHeadingBlock,
  ] as const;
  const blockCount = rng.int(2, 4);
  const blocks: GeneratedBlock[] = [];
  for (let index = 0; index < blockCount; index += 1) {
    blocks.push(rng.pick(builders)(rng));
  }
  return blocks;
}

function generateMarkdownSample(rng: SeededRandom, index: number): MarkdownSample {
  const blocks = generateMarkdownDocument(rng);
  return {
    name: `generated-md-${index}-${blocks.map((block) => block.label).join("-")}`,
    markdown: blocks.map((block) => block.text).join("\n\n"),
  };
}

function uniqueSorted(values: readonly number[]): number[] {
  return [...new Set(values)]
    .filter((value) => Number.isFinite(value) && value > 0)
    .sort((left, right) => left - right);
}

function selectStreamingCutPoints(content: string): number[] {
  const length = content.length;
  if (length <= 1) {
    return [];
  }
  const candidateSet = new Set<number>();
  const addCandidate = (value: number) => {
    if (value > 0 && value < length) {
      candidateSet.add(value);
    }
  };
  for (const fraction of [0.18, 0.37, 0.61, 0.82]) {
    addCandidate(Math.round(length * fraction));
  }
  for (const match of content.matchAll(/\n\n?/g)) {
    addCandidate((match.index ?? 0) + match[0].length);
  }
  for (const match of content.matchAll(/`+/g)) {
    addCandidate((match.index ?? 0) + match[0].length);
  }
  for (const match of content.matchAll(/\|/g)) {
    addCandidate((match.index ?? 0) + 1);
  }
  const candidates = uniqueSorted([...candidateSet]);
  if (candidates.length <= 4) {
    return candidates;
  }
  const targets = [0.18, 0.37, 0.61, 0.82];
  const selected: number[] = [];
  for (const fraction of targets) {
    const target = Math.round(length * fraction);
    const next = candidates
      .filter((candidate) => !selected.includes(candidate))
      .sort((left, right) => Math.abs(left - target) - Math.abs(right - target))[0];
    if (next != null) {
      selected.push(next);
    }
  }
  return uniqueSorted(selected);
}

function splitStreamingFragments(content: string): string[] {
  const cutPoints = selectStreamingCutPoints(content);
  if (cutPoints.length === 0) {
    return [content];
  }
  const fragments: string[] = [];
  let cursor = 0;
  for (const cutPoint of [...cutPoints, content.length]) {
    const fragment = content.slice(cursor, cutPoint);
    if (fragment.length > 0) {
      fragments.push(fragment);
    }
    cursor = cutPoint;
  }
  return fragments;
}

function generateMessageSample(rng: SeededRandom, index: number): GeneratedMessageSample {
  if (index % 5 === 0) {
    const lineCount = rng.int(18, 28);
    return {
      name: `generated-message-${index}-collapsed`,
      params: {
        content: repeatJoin(
          lineCount,
          () => `${generateSentence(rng, 4, 7)} ${generateInlineCode(rng)}`,
          "\n",
        ),
        expanded: false,
      },
    };
  }
  const markdownSample = generateMarkdownSample(rng, index);
  const attachmentCount = index % 4 === 0 ? rng.int(1, 3) : 0;
  return {
    name: `generated-message-${index}-${markdownSample.name}`,
    params: {
      content: markdownSample.markdown,
      expanded: true,
      attachments:
        attachmentCount > 0
          ? Array.from({ length: attachmentCount }, (_, attachmentIndex) => ({
              kind: "image" as const,
              mime_type: "image/png",
              data_base64: IMAGE_DATA_BASE64,
              name: `generated-${index}-${attachmentIndex + 1}.png`,
            }))
          : undefined,
    },
  };
}

function generateAssistantSample(rng: SeededRandom, index: number): GeneratedAssistantSample {
  if (index % 4 === 0) {
    return {
      name: `generated-assistant-${index}-streaming`,
      params: {
        content: `- ${generateSentence(rng, 4, 7)} ${generateInlineCode(rng)}`,
        isComplete: false,
      },
    };
  }
  const markdownSample = generateMarkdownSample(rng, index + 100);
  return {
    name: `generated-assistant-${index}-${markdownSample.name}`,
    params: {
      content: markdownSample.markdown,
      isComplete: true,
    },
  };
}

function generateAssistantStreamingSample(rng: SeededRandom, index: number): GeneratedAssistantStreamingSample {
  const blocks = [
    generateParagraphBlock(rng),
    rng.bool(0.5) ? generateListBlock(rng) : generateNestedListBlock(rng),
  ];
  if (rng.bool(0.6)) {
    blocks.push(rng.bool(0.5) ? generateParagraphBlock(rng) : generateBlockquoteBlock(rng));
  }
  const finalContent = blocks.map((block) => block.text).join("\n\n");
  return {
    name: `generated-assistant-streaming-${index}-${blocks.map((block) => block.label).join("-")}`,
    params: {
      fragments: splitStreamingFragments(finalContent),
    },
    finalContent,
  };
}

function generateTurnHeaderSample(rng: SeededRandom, index: number): GeneratedTurnHeaderSample {
  const lines = [
    `${capitalize(generateWords(rng, 3, 5))} ${generateUrl(rng)} ${generatePathToken(rng)}.`,
    `${capitalize(generateWords(rng, 3, 5))} ${generateCommandToken(rng)} ${generateInlineCode(rng).replaceAll("`", "")}.`,
  ];
  if (rng.bool(0.5)) {
    lines.push(`${capitalize(generateWords(rng, 3, 5))} ${rng.pick(CJK)} ${rng.pick(EMOJI)}.`);
  }
  return {
    name: `generated-turn-header-${index}`,
    params: {
      content: lines.join("\n"),
    },
  };
}

export function generatePretextParityFuzzCorpus(options?: {
  seed?: number;
  markdownCount?: number;
  messageCount?: number;
  assistantCount?: number;
  turnHeaderCount?: number;
}): GeneratedPretextParityFuzzCorpus {
  const seed = options?.seed ?? 20260412;
  const rng = new SeededRandom(seed);
  const markdownCount = options?.markdownCount ?? 18;
  const messageCount = options?.messageCount ?? 12;
  const assistantCount = options?.assistantCount ?? 12;
  const turnHeaderCount = options?.turnHeaderCount ?? 8;

  return {
    seed,
    widths: WIDTHS,
    markdownSamples: Array.from({ length: markdownCount }, (_, index) => generateMarkdownSample(rng, index + 1)),
    messageSamples: Array.from({ length: messageCount }, (_, index) => generateMessageSample(rng, index + 1)),
    assistantSamples: Array.from({ length: assistantCount }, (_, index) => generateAssistantSample(rng, index + 1)),
    assistantStreamingSamples: Array.from({ length: assistantCount }, (_, index) =>
      generateAssistantStreamingSample(rng, index + 1),
    ),
    turnHeaderSamples: Array.from({ length: turnHeaderCount }, (_, index) => generateTurnHeaderSample(rng, index + 1)),
  };
}
