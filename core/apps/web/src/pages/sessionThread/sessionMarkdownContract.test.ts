import { describe, expect, it } from "vitest";
import { createSessionMarkdownDocument } from "./sessionMarkdownContract";

describe("sessionMarkdownContract", () => {
  it("keeps mixed inline table-cell content in one paragraph", () => {
    const document = createSessionMarkdownDocument(
      [
        "| Synthetic example | Column one | Column two |",
        "|---|---:|---:|",
        "| First sample with a long label | `sample-one`: 123 units plus a trailing note | `THREE`: 456 units |",
      ].join("\n"),
    );

    const [table] = document.blocks;
    expect(table?.kind).toBe("table");
    if (table?.kind !== "table") {
      throw new Error("expected parsed markdown table");
    }

    const awsCell = table.rows[1]?.cells[1];
    expect(awsCell?.blocks).toHaveLength(1);
    const [paragraph] = awsCell?.blocks ?? [];
    expect(paragraph?.kind).toBe("paragraph");
    if (paragraph?.kind !== "paragraph") {
      throw new Error("expected table cell paragraph");
    }

    expect(paragraph.text.plainText).toBe("sample-one: 123 units plus a trailing note");
    expect(paragraph.text.runs.map((run) => run.kind)).toEqual(["inlineCode", "text"]);
  });
});
