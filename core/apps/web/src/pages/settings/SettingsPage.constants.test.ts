import { SECTIONS } from "./SettingsPage.constants";

describe("Settings sections", () => {
  it("keeps notifications visible in settings navigation", () => {
    const notifications = SECTIONS.find((section) => section.id === "notifications");

    expect(notifications).toBeDefined();
    expect(notifications?.navHidden).not.toBe(true);
  });

  it("keeps dictation hidden from the sidebar navigation", () => {
    const dictation = SECTIONS.find((section) => section.id === "dictation");

    expect(dictation).toBeDefined();
    expect(dictation?.navHidden).toBe(true);
  });

  it("keeps sandbox and networking visible in settings navigation without a separate sandboxing entry", () => {
    const sandboxAndNetworking = SECTIONS.find((section) => section.id === "container_network");
    const sectionIds = SECTIONS.map((section) => String(section.id));

    expect(sandboxAndNetworking).toBeDefined();
    expect(sandboxAndNetworking?.label).toBe("Sandbox & Networking");
    expect(sandboxAndNetworking?.navHidden).not.toBe(true);
    expect(sectionIds).not.toContain("sandboxing");
  });

  it("surfaces team and enterprise navigation now that the admin scaffold exists", () => {
    const teamEnterprise = SECTIONS.find((section) => section.id === "team_enterprise");

    expect(teamEnterprise).toBeDefined();
    expect(teamEnterprise?.label).toBe("Team & Enterprise");
    expect(teamEnterprise?.navHidden).not.toBe(true);
  });
});
