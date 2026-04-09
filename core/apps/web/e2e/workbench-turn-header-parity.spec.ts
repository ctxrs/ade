import { expect, test, type Page } from "./fixtures";

const TURN_HEADER_FIXTURE = [
  "- [Predicting the Popularity of Social News Posts](https://cs229.stanford.edu/proj2012/MaguireMichelson-PredictingThePopularityOfSocialNewsPosts.pdf) reports `85% accuracy`, but that was a much easier binary setup on a small old dataset: “popular” was `>100` upvotes and “unpopular” was `<50`, with domain and posting-time features. I would treat that as a classroom proof-of-concept, not strong evidence of a robust front-page predictor.",
  "- The strongest paper I found is [Popularity and Quality in Social News Aggregators](https://archives.iw3c2.org/www2015/documents/proceedings/companion/p815.pdf). On HN it gets out-of-sample `R² ≈ 0.65` for vote dynamics and finds estimated quality correlates strongly with observed score (`Spearman ≈ 0.80`). But that model uses time-series vote/position data after submission. It is not “read the title and content before posting, then predict front-page probability.”",
].join("\n");

type TurnHeaderParityMeasurement = {
  planned: number;
  actual: number;
  delta: number;
};

async function openWorkbenchShell(page: Page) {
  await page.goto("/?ctxE2E=1", { waitUntil: "domcontentloaded" });
}

async function measureTurnHeaderParity(
  page: Page,
  plainText: string,
  viewportWidth = 820,
): Promise<TurnHeaderParityMeasurement> {
  return page.evaluate(async ({ plainText, viewportWidth }) => {
    const ReactModule = await import("/node_modules/.vite/deps/react.js");
    const React = ReactModule.default ?? ReactModule;
    const ReactDomClientModule = await import("/node_modules/.vite/deps/react-dom_client.js");
    const ReactDOMClient = ReactDomClientModule.default ?? ReactDomClientModule;
    const { WorkbenchTurnHeaderView } = await import("/src/pages/sessionThread/SessionThreadItemViews.tsx");
    const { getPretextVirtualizerRowLayout } = await import("/src/pages/sessionThread/pretextVirtualizerRowLayout.ts");
    const { resolveSessionThreadContentWidth } = await import("/src/pages/sessionThread/sessionThreadLayoutTokens.ts");

    const contentWidth = resolveSessionThreadContentWidth(viewportWidth);
    const host = document.createElement("div");
    host.style.position = "fixed";
    host.style.left = "-10000px";
    host.style.top = "0";
    host.style.width = `${contentWidth}px`;
    document.body.appendChild(host);

    const root = ReactDOMClient.createRoot(host);
    const header = {
      id: "turn-header-parity",
      created_at: "2026-04-09T00:00:00Z",
      content: plainText,
      plain_text: plainText,
      attachments: [],
    };
    root.render(
      React.createElement(WorkbenchTurnHeaderView, {
        header,
        plainText,
        expanded: true,
        onToggle: () => {},
      }),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 50));

    const actual = host.querySelector(".wb-turn-header")?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(
      {
        kind: "turn_header",
        id: "turn-header-turn-header-parity",
        header,
      },
      viewportWidth,
      { expandedTurnHeaders: { [header.id]: true } },
    ).height;

    root.unmount();
    host.remove();

    return {
      planned,
      actual,
      delta: planned - actual,
    };
  }, { plainText, viewportWidth });
}

test("workbench: expanded turn-header planner matches rendered height for URL-heavy transcript text", async ({
  page,
}) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureTurnHeaderParity(page, TURN_HEADER_FIXTURE);

  expect(
    Math.abs(measurement.delta),
    `turn_header drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});
