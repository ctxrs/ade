import { test } from './fixtures';
import { openWorkbenchShell } from './utils/pretextParity';
import { generatePretextParityFuzzCorpus } from './utils/pretextParityFuzz';
import { resolveSessionThreadMessageTextWidth } from '../src/pages/sessionThread/sessionThreadLayoutTokens';

test('tmp isolate remaining list miss items', async ({ page }) => {
  await openWorkbenchShell(page);
  const corpus = generatePretextParityFuzzCorpus();
  const sample2 = corpus.messageSamples.find((s) => s.name === 'generated-message-2-generated-md-2-hard-break-paragraph-hard-break-nested-list');
  const sample8 = corpus.messageSamples.find((s) => s.name === 'generated-message-8-generated-md-8-table-nested-list-fence-paragraph');
  if (!sample2 || !sample8) throw new Error('missing samples');
  const width = resolveSessionThreadMessageTextWidth(472);
  const cases = [
    {
      name: 'm2-top-only',
      markdown: '- Agent thread token ~~message~~ `sessionThreadDomMeasurement.tsx/core/pretextVirtualizerRowLayout.ts/pages/pages/apps` **buffer delta** **composer** browser fragment composer composer shell:',
    },
    {
      name: 'm2-nested1-only',
      markdown: '- x\n  - Probe probe context command composer agent parity parity padding stream padding shell *thread* ~~entry buffer~~ ⚙️ 測試 佈局.',
    },
    {
      name: 'm2-nested2-only',
      markdown: '- x\n  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render* **delta render layout** 📏 你好 世界.',
    },
    {
      name: 'm8-top-only',
      markdown: '- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src` 🧪 測試 佈局 🙂 你好 世界:',
    },
    {
      name: 'm8-nested1-only',
      markdown: '- x\n  - Buffer buffer ~~composer~~ `core/pages/pages/pages` ~~buffer~~;',
    },
    {
      name: 'm8-nested2-only',
      markdown: '- x\n  - Deterministic parity message stream `git rev-parse HEAD` [padding entry](https://example.com/inline-code/transcript?ref=676) *message agent* *session entry header* *summary summary*:',
    },
  ] as const;
  for (const probe of cases) {
    const parity = await page.evaluate(async ({ markdown, width, name }) => {
      const win = window as any;
      return win.__ctxE2E.measureMarkdownParity([{ name, markdown }], width);
    }, { markdown: probe.markdown, width, name: probe.name });
    console.log(JSON.stringify({ kind: 'isolated-item', width, probe, parity }, null, 2));
  }
});
