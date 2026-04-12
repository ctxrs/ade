import { expect, test } from "./fixtures";
import {
  measureTurnHeaderParity,
  openWorkbenchShell,
} from "./utils/pretextParity";

const TURN_HEADER_FIXTURE = [
  "- [Predicting the Popularity of Social News Posts](https://cs229.stanford.edu/proj2012/MaguireMichelson-PredictingThePopularityOfSocialNewsPosts.pdf) reports `85% accuracy`, but that was a much easier binary setup on a small old dataset: “popular” was `>100` upvotes and “unpopular” was `<50`, with domain and posting-time features. I would treat that as a classroom proof-of-concept, not strong evidence of a robust front-page predictor.",
  "- The strongest paper I found is [Popularity and Quality in Social News Aggregators](https://archives.iw3c2.org/www2015/documents/proceedings/companion/p815.pdf). On HN it gets out-of-sample `R² ≈ 0.65` for vote dynamics and finds estimated quality correlates strongly with observed score (`Spearman ≈ 0.80`). But that model uses time-series vote/position data after submission. It is not “read the title and content before posting, then predict front-page probability.”",
].join("\n");

test("workbench: expanded turn-header planner matches rendered height for URL-heavy transcript text", async ({
  page,
}) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureTurnHeaderParity(page, { plainText: TURN_HEADER_FIXTURE });

  expect(
    Math.abs(measurement.delta),
    `turn_header drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});
