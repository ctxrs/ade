export const storySections = [
  {
    id: "top",
    label: "Top",
    description: "Front page highlights for the demo build.",
    stories: [
      {
        id: 101,
        title: "Tauri 2 makes small cross-platform tools feel viable again",
        domain: "notes.ctx.rs",
        points: 284,
        commentsCount: 41,
        author: "riverloop",
        age: "2h",
        url: "https://notes.ctx.rs/tauri-small-tools",
        summary:
          "A short post about why lightweight native shells are a good fit for focused developer products.",
        comments: [
          {
            id: 1011,
            author: "jdoe",
            age: "1h",
            text: "The nice part is shipping something that still feels native without a giant runtime tax.",
          },
          {
            id: 1012,
            author: "mlb",
            age: "58m",
            text: "For tooling apps, startup time and battery use matter more than maximal UI abstraction.",
          },
        ],
      },
      {
        id: 102,
        title: "Shipping deterministic demos for AI products",
        domain: "buildsystems.dev",
        points: 197,
        commentsCount: 28,
        author: "smeaton",
        age: "3h",
        url: "https://buildsystems.dev/deterministic-demos",
        summary:
          "A walkthrough of replay-based demo systems that keep the real app and real outputs in the loop.",
        comments: [
          {
            id: 1021,
            author: "ragged",
            age: "2h",
            text: "The trick is choosing where to be real and where to be deterministic.",
          },
        ],
      },
      {
        id: 103,
        title: "Designing mobile readers for text-heavy communities",
        domain: "productnotes.fm",
        points: 163,
        commentsCount: 19,
        author: "astra",
        age: "5h",
        url: "https://productnotes.fm/mobile-readers",
        summary:
          "Why calm spacing, fast navigation, and remembered reading state matter more than ornamental chrome.",
        comments: [],
      },
    ],
  },
  {
    id: "new",
    label: "New",
    description: "Fresh submissions and quick takes.",
    stories: [
      {
        id: 201,
        title: "A tiny SQLite toolchain for app-side search indexing",
        domain: "microstack.dev",
        points: 34,
        commentsCount: 7,
        author: "huxley",
        age: "18m",
        url: "https://microstack.dev/sqlite-search-indexing",
        summary:
          "An experiment in keeping on-device search fast without dragging in a heavyweight backend.",
        comments: [
          {
            id: 2011,
            author: "vgr",
            age: "12m",
            text: "The ergonomics are better than I expected if you control your write path.",
          },
        ],
      },
      {
        id: 202,
        title: "Recording simulator output with repeatable framing",
        domain: "appsignal.blog",
        points: 22,
        commentsCount: 3,
        author: "linen",
        age: "31m",
        url: "https://appsignal.blog/simulator-framing",
        summary:
          "A short note on keeping demo captures polished without making the runtime path itself flaky.",
        comments: [],
      },
    ],
  },
  {
    id: "ask",
    label: "Ask",
    description: "Questions and practical operator threads.",
    stories: [
      {
        id: 301,
        title: "Ask HN: what makes an engineering homepage demo actually convincing?",
        domain: "news.ycombinator.com",
        points: 119,
        commentsCount: 62,
        author: "trace",
        age: "7h",
        url: "https://news.ycombinator.com/item?id=301",
        summary:
          "People discuss whether product demos should show model thinking, diffs, outputs, or some mix of all three.",
        comments: [
          {
            id: 3011,
            author: "sdb",
            age: "6h",
            text: "If it looks fake, engineers discount it immediately. Show the diff and the output.",
          },
          {
            id: 3012,
            author: "pearl",
            age: "4h",
            text: "Autoplay loops need one idea. Everything else is a follow-up click.",
          },
        ],
      },
    ],
  },
];
