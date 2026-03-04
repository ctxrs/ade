import { SECTIONS } from "./SettingsPage.constants";

describe("Settings sections", () => {
  it("keeps dictation hidden from the sidebar navigation", () => {
    const dictation = SECTIONS.find((section) => section.id === "dictation");

    expect(dictation).toBeDefined();
    expect(dictation?.navHidden).toBe(true);
  });
});
