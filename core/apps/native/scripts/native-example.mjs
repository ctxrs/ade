#!/usr/bin/env node

import { connect, expect } from "./native-driver.mjs";

const app = await connect({
  httpUrl: "http://127.0.0.1:6160",
  wsUrl: "ws://127.0.0.1:6160/ws",
});

const page = app.page;

const composer = page.locator("#composer");
await composer.click();
await composer.type("hello from native automation");
await page.keyboard.press("Enter");
await expect(composer).toBeVisible();
await page.screenshot({ name: "native-example" });

await app.close();

