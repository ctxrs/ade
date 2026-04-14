import { test } from './fixtures';
import { openWorkbenchShell } from './utils/pretextParity';
import { generatePretextParityFuzzCorpus } from './utils/pretextParityFuzz';
import { resolveSessionThreadMessageTextWidth } from '../src/pages/sessionThread/sessionThreadLayoutTokens';

test('tmp exact planner width for message8 nested1', async ({ page }) => {
  await openWorkbenchShell(page);
  const corpus = generatePretextParityFuzzCorpus();
  const sample8 = corpus.messageSamples.find((s) => s.name === 'generated-message-8-generated-md-8-table-nested-list-fence-paragraph');
  if (!sample8) throw new Error('missing sample');
  const width = resolveSessionThreadMessageTextWidth(472);
  const probe = {
    name: 'message8-block2-nested1',
    markdown: sample8.params.content.split('\n\n')[1]!,
    probeWidth: width,
    debugWidth: width - 48,
    debugTarget: 'core/pages/pages/pages',
  } as const;
  const details = await page.evaluate(async ({ markdown, probeWidth, debugWidth, debugTarget, name }) => {
    const win = window as any;
    function collectLineTexts(el: Element) {
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      const chars: Array<{ node: Node; start: number; end: number; ch: string }> = [];
      while (walker.nextNode()) {
        const node = walker.currentNode;
        const text = node.textContent ?? '';
        let offset = 0;
        for (const ch of Array.from(text)) {
          const start = offset;
          offset += ch.length;
          chars.push({ node, start, end: offset, ch });
        }
      }
      const lines: Array<{ top: number; text: string }> = [];
      for (const entry of chars) {
        const range = document.createRange();
        range.setStart(entry.node, entry.start);
        range.setEnd(entry.node, entry.end);
        const rect = range.getClientRects()[0];
        if (!rect || rect.width === 0) continue;
        const top = Math.round(rect.top * 100) / 100;
        const prev = lines[lines.length - 1];
        if (!prev || Math.abs(prev.top - top) > 0.5) lines.push({ top, text: entry.ch });
        else prev.text += entry.ch;
      }
      return lines.map((line) => line.text);
    }
    win.__ctxForceInlineCodeDebug = true;
    win.__ctxInlineCodeDebugTarget = debugTarget;
    win.__ctxInlineCodeDebugWidth = debugWidth;
    win.__ctxInlineCodeDebug = null;
    const parity = await win.__ctxE2E.measureMarkdownParity([{ name, markdown }], probeWidth);
    const planner = win.__ctxInlineCodeDebug ?? null;
    await win.__ctxE2E.installMarkdownScrollProbe(markdown, probeWidth);
    await new Promise((resolve) => window.requestAnimationFrame(() => resolve(undefined)));
    const paragraphs = Array.from(document.querySelectorAll<HTMLElement>('#markdown-scroll-probe .wb-md-list-item-body p')).map((p, index) => ({
      index,
      text: p.textContent ?? '',
      lines: collectLineTexts(p),
      codeRects: Array.from(p.querySelectorAll<HTMLElement>('code')).map((code) => ({
        text: code.textContent ?? '',
        rects: Array.from(code.getClientRects()).map((rect) => ({ top: rect.top, left: rect.left, width: rect.width, height: rect.height })),
      })),
    }));
    await win.__ctxE2E.removeMarkdownScrollProbe();
    return { parity, planner, paragraphs, debugWidth };
  }, probe);
  console.log(JSON.stringify({ kind: 'exact-planner-widths', probe, ...details }, null, 2));
});
