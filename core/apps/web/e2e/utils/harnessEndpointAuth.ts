import { expect, type Locator, type Page } from "playwright/test";
import type { EndpointHarnessMatrixEntry } from "./harnessEndpointMatrix";

export type HarnessAuthConfigResult =
  | { ok: true; detail: string }
  | { ok: false; detail: string };

const normalizeText = (value: string | null | undefined): string => (value ?? "").replace(/\s+/g, " ").trim();

const harnessTriggerLabel = (page: Page) =>
  page
    .locator(
      ".wb-new-composer-stack .wb-switcher-harness .wb-switcher-label, .wb-new-composer-stack button[title='Harness'] .wb-switcher-label",
    )
    .first();

const isHarnessLabelSelected = (labelText: string, entry: EndpointHarnessMatrixEntry): boolean => {
  const selected = labelText.toLowerCase();
  return selected.includes(entry.menuLabel.toLowerCase()) || selected.includes(entry.providerId.toLowerCase());
};

async function openHarnessMenu(page: Page) {
  const harnessButton = page
    .locator(".wb-new-composer-stack .wb-switcher-harness, .wb-new-composer-stack button[title='Harness']")
    .first();
  await expect(harnessButton).toBeVisible({ timeout: 20_000 });
  const menu = page.locator(".wb-harness-menu");
  if (!(await menu.isVisible().catch(() => false))) {
    await harnessButton.click();
    await expect(menu).toBeVisible({ timeout: 10_000 });
  }
  return menu;
}

async function resolveHarnessMenuButton(
  menu: Locator,
  entry: EndpointHarnessMatrixEntry,
) {
  const byLabel = menu
    .locator(".wb-harness-row .wb-harness-row-main")
    .filter({ hasText: entry.menuLabel })
    .first();
  if (await byLabel.count()) return byLabel;
  const byProviderId = menu
    .locator(".wb-harness-row .wb-harness-row-main")
    .filter({ hasText: entry.providerId })
    .first();
  return byProviderId;
}

async function chooseOpenRouterPreset(page: Page, modal: Locator) {
  const providerSelect = modal.locator('[role="combobox"]').first();
  if ((await providerSelect.count()) === 0) return;
  await providerSelect.click();
  const roleOption = page.getByRole("option", { name: /OpenRouter/i }).first();
  if ((await roleOption.count()) > 0) {
    await roleOption.click();
    return;
  }
  const textOption = page.locator(".tw-z-\\[1101\\]").getByText("OpenRouter", { exact: true }).first();
  if ((await textOption.count()) > 0) {
    await textOption.click();
    return;
  }
  throw new Error("OpenRouter preset option not found in endpoint provider selector");
}

async function dismissAuthModalIfOpen(page: Page): Promise<void> {
  const modal = page.locator(".settings-harness-modal");
  if (!(await modal.isVisible().catch(() => false))) return;
  const closeButton = modal.getByRole("button", { name: "Close" }).first();
  if ((await closeButton.count()) > 0) {
    await closeButton.click().catch(() => {});
  }
  if (await modal.isVisible().catch(() => false)) {
    const backButton = modal.getByRole("button", { name: "Back" }).first();
    if ((await backButton.count()) > 0) {
      await backButton.click().catch(() => {});
    }
  }
  if (await modal.isVisible().catch(() => false)) {
    await page.locator(".modal-overlay").first().click({ position: { x: 4, y: 4 } }).catch(() => {});
  }
  if (await modal.isVisible().catch(() => false)) {
    await page.keyboard.press("Escape").catch(() => {});
  }
  await modal.waitFor({ state: "hidden", timeout: 4_000 }).catch(() => {});
}

export async function configureHarnessEndpointAuthViaModal(
  page: Page,
  entry: EndpointHarnessMatrixEntry,
  apiKey: string,
  baseUrl: string,
  modelOverride = "",
): Promise<HarnessAuthConfigResult> {
  await dismissAuthModalIfOpen(page);
  const menu = await openHarnessMenu(page);
  await menu.getByLabel("Search agents").fill(entry.searchTerm);
  const rowButton = await resolveHarnessMenuButton(menu, entry);
  if ((await rowButton.count()) === 0) {
    return { ok: false, detail: "harness menu row not found" };
  }
  await rowButton.click();

  const modal = page.locator(".settings-harness-modal");
  try {
    await expect(modal).toBeVisible({ timeout: 10_000 });
  } catch {
    return { ok: false, detail: "auth modal did not open (provider may already be configured)" };
  }

  await modal.getByRole("button", { name: "API Key" }).click();
  await chooseOpenRouterPreset(page, modal);

  const passwordInput = modal.locator("input[type='password']").first();
  await expect(passwordInput).toBeVisible({ timeout: 10_000 });
  await passwordInput.fill(apiKey);

  const endpointName = `${entry.providerId}-openrouter`;
  const nameInput = modal
    .locator("label.settings-harness-modal-label")
    .filter({ hasText: "Name (optional)" })
    .locator("input")
    .first();
  if ((await nameInput.count()) > 0) {
    await nameInput.fill(endpointName);
  }

  const baseUrlInput = modal
    .locator("label.settings-harness-modal-label")
    .filter({ hasText: "Base URL" })
    .locator("input")
    .first();
  if ((await baseUrlInput.count()) > 0) {
    await baseUrlInput.fill(baseUrl);
  }

  const targetModel = modelOverride.trim();
  if (targetModel) {
    const modelOverrideInput = modal
      .locator("label.settings-harness-modal-label")
      .filter({ hasText: "Model override" })
      .locator("input")
      .first();
    if ((await modelOverrideInput.count()) > 0) {
      await modelOverrideInput.fill(targetModel);
    } else {
      const modelInput = modal
        .locator("label.settings-harness-modal-label")
        .filter({ hasText: "Model" })
        .locator("input")
        .first();
      if ((await modelInput.count()) > 0) {
        await modelInput.fill(targetModel);
      }
    }
  }

  await modal.getByRole("button", { name: "Add API key" }).click();
  const providerError = page.locator(".settings-banner.settings-banner-error").first();
  const startedAt = Date.now();
  const timeoutMs = 20_000;
  while (Date.now() - startedAt < timeoutMs) {
    const modalVisible = await modal.isVisible().catch(() => false);
    if (!modalVisible) {
      return { ok: true, detail: "endpoint auth saved via modal" };
    }

    const errorText = normalizeText(await providerError.textContent().catch(() => ""));
    if (errorText) {
      if (errorText.toLowerCase().includes("endpoint verification failed")) {
        await dismissAuthModalIfOpen(page);
        return { ok: true, detail: `endpoint auth saved with verify warning: ${errorText}` };
      }
      await dismissAuthModalIfOpen(page).catch(() => {});
      return { ok: false, detail: errorText };
    }

    await page.waitForTimeout(200);
  }

  await dismissAuthModalIfOpen(page).catch(() => {});
  return { ok: false, detail: "auth modal did not close after Add API key" };
}

export async function selectHarnessForComposer(
  page: Page,
  entry: EndpointHarnessMatrixEntry,
): Promise<HarnessAuthConfigResult> {
  const triggerLabel = harnessTriggerLabel(page);
  await expect(triggerLabel).toBeVisible({ timeout: 10_000 });
  const currentLabel = ((await triggerLabel.textContent()) ?? "").trim();
  if (isHarnessLabelSelected(currentLabel, entry)) {
    return { ok: true, detail: `selected harness '${currentLabel}'` };
  }

  const menu = await openHarnessMenu(page);
  await menu.getByLabel("Search agents").fill(entry.searchTerm);
  const rowButton = await resolveHarnessMenuButton(menu, entry);
  if ((await rowButton.count()) === 0) {
    return { ok: false, detail: "harness menu row not found" };
  }
  await rowButton.click();

  await expect
    .poll(
      async () => {
        const label = ((await triggerLabel.textContent()) ?? "").trim();
        return isHarnessLabelSelected(label, entry);
      },
      { timeout: 10_000, intervals: [200, 400, 800] },
    )
    .toBe(true);

  const selected = ((await triggerLabel.textContent()) ?? "").trim();
  return { ok: true, detail: `selected harness '${selected}'` };
}
