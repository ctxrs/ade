export const HN_DEMO_SCENARIO_QUERY_KEY = "ctxDemoScenario";
export const DEFAULT_HN_DEMO_SCENARIO_ID = "muted-domains";

const DEMO_SCENARIOS = Object.freeze({
  [DEFAULT_HN_DEMO_SCENARIO_ID]: Object.freeze({
    id: DEFAULT_HN_DEMO_SCENARIO_ID,
    accountId: "ADE_TEST_ACCOUNT",
    emailAddress: "contact-21599a40845f@fixture.example.test",
    frontPageStories: Object.freeze([
      Object.freeze({
        id: "47584540",
        rank: 1,
        href: "https://twitter.com/Fried_rice/status/2038894956459290963",
        title: "Claude Code's source code has been leaked via a map file in their NPM registry",
        site: "twitter.com",
        points: 2007,
        user: "treexs",
        age: "1 day ago",
        comments: 992,
      }),
      Object.freeze({
        id: "47582792",
        rank: 2,
        href: "https://twitter.com/oliviscusAI/status/2038563166431346865",
        title: "You can now run a full Linux operating system inside a 6mb PDF",
        site: "twitter.com",
        points: 22,
        user: "matthewsinclair",
        age: "1 day ago",
        comments: 2,
      }),
    ]),
  }),
});

export function resolveDemoScenarioId(rawScenarioId) {
  const scenarioId = String(rawScenarioId ?? "").trim();
  return DEMO_SCENARIOS[scenarioId] ? scenarioId : DEFAULT_HN_DEMO_SCENARIO_ID;
}

export function getDemoScenario(rawScenarioId) {
  return DEMO_SCENARIOS[resolveDemoScenarioId(rawScenarioId)];
}

export function readDemoScenarioIdFromSearch(searchValue) {
  try {
    const searchParams = new URLSearchParams(searchValue ?? "");
    const scenarioId = searchParams.get(HN_DEMO_SCENARIO_QUERY_KEY);
    return scenarioId ? resolveDemoScenarioId(scenarioId) : DEFAULT_HN_DEMO_SCENARIO_ID;
  } catch {
    return DEFAULT_HN_DEMO_SCENARIO_ID;
  }
}

export function appendDemoScenarioToLocalPath(pathname, rawScenarioId) {
  const localUrl = new URL(pathname, "http://127.0.0.1");
  localUrl.searchParams.set(HN_DEMO_SCENARIO_QUERY_KEY, resolveDemoScenarioId(rawScenarioId));
  return `${localUrl.pathname}${localUrl.search}${localUrl.hash}`;
}
