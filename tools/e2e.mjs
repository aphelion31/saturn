// Browser check against a running, seeded Saturn: sign in through the real Plane web app, open the
// workspace and the first project's work items, then god-mode. Prints every /api or /auth call that
// failed (status >= 400), so unported endpoints the UI depends on show up.
//   BASE=http://127.0.0.1:3300 node tools/e2e.mjs
// Needs playwright: NODE_PATH=$(npm root -g)/@playwright/mcp/node_modules
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const BASE = process.env.BASE || "http://127.0.0.1:3000";
const EMAIL = process.env.EMAIL || "admin@saturn.dev";
const PASSWORD = process.env.PASSWORD || "Saturn-rocks-2026";
const SLUG = process.env.SLUG || "acme";

const failed = new Map();
const browser = await chromium.launch(process.env.CHROME ? { executablePath: process.env.CHROME } : {});
const page = await browser.newPage();
page.on("response", (r) => {
  const u = new URL(r.url());
  if ((u.pathname.startsWith("/api/") || u.pathname.startsWith("/auth/")) && r.status() >= 400) {
    const key = `${r.status()} ${r.request().method()} ${u.pathname.replace(/[0-9a-f-]{36}/g, "{id}")}`;
    failed.set(key, (failed.get(key) || 0) + 1);
  }
});
page.on("pageerror", (e) => console.log("pageerror:", e.message.slice(0, 200)));

async function step(name, fn) {
  try {
    await fn();
    console.log(`✔ ${name} -> ${new URL(page.url()).pathname}`);
  } catch (e) {
    console.log(`✖ ${name}: ${e.message.split("\n")[0]}`);
    await page.screenshot({ path: `/tmp/saturn-e2e-${name.replace(/\W+/g, "-")}.png` });
  }
}

await step("sign in", async () => {
  await page.goto(`${BASE}/`);
  await page.fill("input[name=email]", EMAIL);
  await page.keyboard.press("Enter");
  await page.waitForSelector("input[name=password]", { timeout: 15000 });
  await page.fill("input[name=password]", PASSWORD);
  await page.keyboard.press("Enter");
  await page.waitForURL(new RegExp(`/${SLUG}`), { timeout: 20000 });
});
await step("workspace home", async () => {
  await page.goto(`${BASE}/${SLUG}/`);
  await page.waitForLoadState("networkidle");
});
await step("projects", async () => {
  await page.goto(`${BASE}/${SLUG}/projects/`);
  await page.waitForLoadState("networkidle");
});
await step("work items", async () => {
  const link = await page.locator(`a[href*="/${SLUG}/projects/"][href*="/issues"]`).first();
  if (await link.count()) await link.click();
  await page.waitForLoadState("networkidle");
  await page.screenshot({ path: "/tmp/saturn-e2e-issues.png", fullPage: false });
});
await step("god-mode", async () => {
  await page.goto(`${BASE}/god-mode/`);
  await page.waitForLoadState("networkidle");
});

console.log(failed.size ? "\nfailed API calls:" : "\nno failed API calls");
for (const [k, n] of [...failed.entries()].sort()) console.log(`  ${k}  x${n}`);
await browser.close();
