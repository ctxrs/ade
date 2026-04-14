import { test } from './fixtures';
import { openWorkbenchShell } from './utils/pretextParity';
import { resolveSessionThreadMessageTextWidth } from '../src/pages/sessionThread/sessionThreadLayoutTokens';

test('tmp isolate exact frontier for remaining list misses', async ({ page }) => {
  await openWorkbenchShell(page);
  const width = resolveSessionThreadMessageTextWidth(472);
  const cases = [
    {
      kind: 'm2',
      samples: [
        { name: 'm2-1', markdown: '- x\n  - Entry browser **inline stream pretext**' },
        { name: 'm2-2', markdown: '- x\n  - Entry browser **inline stream pretext** 🙂 你好 世界' },
        { name: 'm2-3', markdown: '- x\n  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render*' },
        { name: 'm2-4', markdown: '- x\n  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render* **delta render layout**' },
        { name: 'm2-5', markdown: '- x\n  - Entry browser **inline stream pretext** 🙂 你好 世界 *render session render* **delta render layout** 📏 你好 世界.' },
      ],
    },
    {
      kind: 'm8',
      samples: [
        { name: 'm8-1', markdown: '- Thread stream parity fragment' },
        { name: 'm8-2', markdown: '- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src`' },
        { name: 'm8-3', markdown: '- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src` 🧪 測試 佈局' },
        { name: 'm8-4', markdown: '- Thread stream parity fragment `sessionThreadDomMeasurement.tsx/inline-code/turn-header/web/turn-header/src` 🧪 測試 佈局 🙂 你好 世界:' },
      ],
    },
  ] as const;
  for (const itemCase of cases) {
    const results = await page.evaluate(async ({ samples, width }) => {
      const win = window as any;
      return win.__ctxE2E.measureMarkdownParity(samples, width);
    }, { samples: itemCase.samples, width });
    console.log(JSON.stringify({ kind: 'isolated-frontier', case: itemCase.kind, width, results }, null, 2));
  }
});
