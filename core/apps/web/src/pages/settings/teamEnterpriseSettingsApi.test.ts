import { describe, expect, it } from "vitest";
import {
  fetchEntitlementsSnapshot,
  fetchTeamEnterpriseCloudState,
  invokeTeamEnterpriseAdminAction,
  startTeamBillingCheckout,
} from "./teamEnterpriseSettingsApi";

const unavailableMessage =
  "This settings action is unavailable in the source build/local ADE.";

describe("teamEnterpriseSettingsApi", () => {
  it("fails admin actions explicitly in the source build", async () => {
    await expect(
      invokeTeamEnterpriseAdminAction(null, { action: "create_organization", name: "Example" }),
    ).rejects.toThrow(unavailableMessage);
  });

  it("fails state refresh explicitly in the source build", async () => {
    await expect(
      fetchTeamEnterpriseCloudState({ client: null, requestedActiveOrgId: null }),
    ).rejects.toThrow(unavailableMessage);
  });

  it("fails entitlement refresh explicitly in the source build", async () => {
    await expect(fetchEntitlementsSnapshot({ client: null, activeOrgId: null })).rejects.toThrow(
      unavailableMessage,
    );
  });

  it("fails checkout explicitly in the source build", async () => {
    await expect(
      startTeamBillingCheckout({
        client: null,
        organizationId: "org_local",
        billingSubjectId: "subject_local",
        interval: "month",
        returnPath: "/settings#general",
      }),
    ).rejects.toThrow(unavailableMessage);
  });
});
