import { expect, test, type Page } from "./fixtures";

const EXACT_USER_MESSAGE = [
  "here is another neutral layout idea for a deterministic fixture",
  "",
  "we could ask the model to summarize a sample article and then write ten synthetic review notes.",
  "",
  "then we can compare whether the notes use the same broad style as the reference examples",
  "",
  "for example, a short positive note or a careful critical note can exercise different wrapping without carrying real conversation text.",
  "",
  "what other neutral variations should this layout test include",
].join("\n");

const COLLAPSED_LONG_MESSAGE = Array.from(
  { length: 24 },
  (_, index) => `line ${index + 1} with enough words to wrap a little bit`,
).join("\n");

const ASSISTANT_MARKDOWN = [
  "# Title",
  "",
  "- bullet one",
  "- bullet two",
  "",
  "```ts",
  "const x = 1;",
  "```",
].join("\n");

const IMAGE_DATA_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2VzJ8AAAAASUVORK5CYII=";

type RowParityMeasurement = {
  planned: number;
  actual: number;
  delta: number;
};

async function openWorkbenchShell(page: Page) {
  await page.goto("/?ctxE2E=1", { waitUntil: "domcontentloaded" });
}

async function measureMessageParity(
  page: Page,
  params: {
    content: string;
    expanded: boolean;
    attachments?: Array<{ kind: "image"; mime_type: string; data_base64: string; name?: string }>;
  },
): Promise<RowParityMeasurement> {
  return page.evaluate(async ({ content, expanded, attachments }) => {
    const ReactModule = await import("/node_modules/.vite/deps/react.js");
    const React = ReactModule.default ?? ReactModule;
    const ReactDomClientModule = await import("/node_modules/.vite/deps/react-dom_client.js");
    const ReactDOMClient = ReactDomClientModule.default ?? ReactDomClientModule;
    const { ThreadItemView } = await import("/src/pages/sessionThread/SessionThreadItemViews.tsx");
    const { getPretextVirtualizerRowLayout } = await import("/src/pages/sessionThread/pretextVirtualizerRowLayout.ts");
    const {
      SESSION_THREAD_LAYOUT_STYLE,
      resolveSessionThreadContentWidth,
    } = await import("/src/pages/sessionThread/sessionThreadLayoutTokens.ts");

    const viewportWidth = 820;
    const contentWidth = resolveSessionThreadContentWidth(viewportWidth);
    const host = document.createElement("div");
    host.style.position = "fixed";
    host.style.left = "-10000px";
    host.style.top = "0";
    host.style.width = `${contentWidth}px`;
    host.style.margin = "0";
    host.style.padding = "0";
    host.style.border = "0";
    host.style.boxSizing = "border-box";
    for (const [key, value] of Object.entries(SESSION_THREAD_LAYOUT_STYLE)) {
      host.style.setProperty(key, String(value));
    }
    document.body.appendChild(host);

    const item = {
      kind: "message" as const,
      id: "message-parity",
      role: "user" as const,
      content,
      attachments: attachments ?? [],
      created_at: "2026-04-09T00:00:00Z",
    };

    const root = ReactDOMClient.createRoot(host);
    root.render(
      React.createElement(
        "div",
        { "data-thread-item-id": item.id },
        React.createElement(
          "div",
          { className: "wb-thread-indent" },
          React.createElement(ThreadItemView, {
            item,
            worktreeId: null,
            onFileOpenError: () => {},
            messageExpanded: expanded,
            onToggleMessageExpanded: () => {},
          }),
        ),
      ),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 75));

    const actual = host.querySelector<HTMLElement>(`[data-thread-item-id="${item.id}"]`)?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(item, viewportWidth, {
      expandedMessageById: { [item.id]: expanded },
    }).height;

    root.unmount();
    host.remove();

    return {
      planned,
      actual,
      delta: planned - actual,
    };
  }, params);
}

async function measureAssistantParity(page: Page, content: string): Promise<RowParityMeasurement> {
  return page.evaluate(async ({ content }) => {
    const ReactModule = await import("/node_modules/.vite/deps/react.js");
    const React = ReactModule.default ?? ReactModule;
    const ReactDomClientModule = await import("/node_modules/.vite/deps/react-dom_client.js");
    const ReactDOMClient = ReactDomClientModule.default ?? ReactDomClientModule;
    const { AssistantEntry } = await import("/src/pages/sessionThread/SessionThreadItemViews.tsx");
    const { getPretextVirtualizerRowLayout } = await import("/src/pages/sessionThread/pretextVirtualizerRowLayout.ts");
    const {
      SESSION_THREAD_LAYOUT_STYLE,
      resolveSessionThreadContentWidth,
    } = await import("/src/pages/sessionThread/sessionThreadLayoutTokens.ts");

    const viewportWidth = 820;
    const contentWidth = resolveSessionThreadContentWidth(viewportWidth);
    const host = document.createElement("div");
    host.style.position = "fixed";
    host.style.left = "-10000px";
    host.style.top = "0";
    host.style.width = `${contentWidth}px`;
    host.style.margin = "0";
    host.style.padding = "0";
    host.style.border = "0";
    host.style.boxSizing = "border-box";
    for (const [key, value] of Object.entries(SESSION_THREAD_LAYOUT_STYLE)) {
      host.style.setProperty(key, String(value));
    }
    document.body.appendChild(host);

    const item = {
      kind: "assistant" as const,
      id: "assistant-parity",
      turn_id: "turn-1",
      created_at: "2026-04-09T00:00:00Z",
      content,
      thought: "",
      is_complete: true,
    };

    const root = ReactDOMClient.createRoot(host);
    root.render(
      React.createElement(
        "div",
        { "data-thread-item-id": item.id },
        React.createElement(
          "div",
          { className: "wb-thread-indent" },
          React.createElement(AssistantEntry, {
            content,
            worktreeId: null,
            onFileOpenError: () => {},
          }),
        ),
      ),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 75));

    const actual = host.querySelector<HTMLElement>(`[data-thread-item-id="${item.id}"]`)?.getBoundingClientRect().height ?? 0;
    const planned = getPretextVirtualizerRowLayout(item, viewportWidth, {}).height;

    root.unmount();
    host.remove();

    return {
      planned,
      actual,
      delta: planned - actual,
    };
  }, { content });
}

test("workbench: exact multi-paragraph user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: EXACT_USER_MESSAGE,
    expanded: true,
  });

  expect(
    Math.abs(measurement.delta),
    `message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: collapsed toggleable user message planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: COLLAPSED_LONG_MESSAGE,
    expanded: false,
  });

  expect(
    Math.abs(measurement.delta),
    `collapsed message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: image attachments stay in parity for message rows", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureMessageParity(page, {
    content: "two inline screenshots",
    expanded: true,
    attachments: [
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "one.png" },
      { kind: "image", mime_type: "image/png", data_base64: IMAGE_DATA_BASE64, name: "two.png" },
    ],
  });

  expect(
    Math.abs(measurement.delta),
    `attachment message drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});

test("workbench: assistant markdown planner matches rendered height", async ({ page }) => {
  test.setTimeout(120000);
  await openWorkbenchShell(page);

  const measurement = await measureAssistantParity(page, ASSISTANT_MARKDOWN);

  expect(
    Math.abs(measurement.delta),
    `assistant drifted by ${measurement.delta}px (planned ${measurement.planned}, actual ${measurement.actual})`,
  ).toBeLessThanOrEqual(1);
});
