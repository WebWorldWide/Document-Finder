// Records the homepage product walkthrough (site/video/brief.yaml).
//
//   pnpm dev                                   # app UI + dev mock backend on :1420
//   npm i --no-save playwright && npx playwright install chromium
//   node site/video/capture.mjs                # → site/video/raw/capture.webm + beats.json
//   site/video/master.sh                       # → site/assets/demo.{mp4,webm}, poster.jpg
//
// Follows the devrel-video capture rules: real UI, deliberate pauses after each
// state change, eased scrolling, a fallback for every locator, and beat times
// recorded from the capture itself (they drive the on-site chapter captions).
import { chromium } from "playwright";
import { mkdirSync, writeFileSync, readdirSync, renameSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = process.env.VIDEO_DIR ?? dirname(fileURLToPath(import.meta.url));
const out = join(here, "raw");
mkdirSync(out, { recursive: true });
const URL = process.env.DEMO_URL ?? "http://localhost:1420";
const W = 1440;
const H = 900;

const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: W, height: H },
  deviceScaleFactor: 1,
  colorScheme: "light",
  recordVideo: { dir: out, size: { width: W, height: H } },
});
const page = await ctx.newPage();
const t0 = Date.now();
const beats = [];
const beat = (label) => beats.push({ label, t: (Date.now() - t0) / 1000 });
const wait = (ms) => page.waitForTimeout(ms);
const scrollMain = (y, dur) =>
  page.evaluate(
    ({ y, dur }) =>
      new Promise((res) => {
        const el = document.getElementById("main-content") ?? document.scrollingElement;
        const start = el.scrollTop;
        const delta = y - start;
        const s = performance.now();
        const step = (now) => {
          const p = Math.min(1, (now - s) / dur);
          const e = p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2;
          el.scrollTop = start + delta * e;
          if (p < 1) requestAnimationFrame(step);
          else res();
        };
        requestAnimationFrame(step);
      }),
    { y, dur },
  );

await page.addInitScript(() => localStorage.setItem("df-theme-v3", "light"));
await page.goto(URL);
await page.waitForSelector("#df-q", { timeout: 20000 });
await wait(1200);

// Beat 1 — ask in plain language
beat("Ask in plain language");
await page.locator("#df-q").click();
await page.keyboard.type("effects of sleep on memory in teenagers", { delay: 55 });
await wait(700);
const find = page.getByRole("button", { name: /find documents/i });
if (await find.count()) await find.first().click();
else await page.keyboard.press("Enter");

// Beat 2 — downloads start in seconds
await page.waitForSelector(".df-tiles", { timeout: 20000 });
beat("Downloads start in seconds");
await wait(6500);

// Beat 3 — every source, filed
beat("Every source, filed");
await scrollMain(560, 2600);
await wait(2200);
await scrollMain(0, 1400);
await page.waitForFunction(() => /Finished/.test(document.querySelector(".df-status")?.textContent ?? ""), null, {
  timeout: 30000,
});
await wait(1500);

// Beat 4 — search the full text
beat("Search the full text");
const lib = page.getByRole("button", { name: /open in library/i });
if (await lib.count()) await lib.first().click();
else await page.locator('.df-nav-item[aria-label="Library"]').click();
await page.waitForSelector(".df-lib-toolbar input", { timeout: 10000 });
await wait(900);
await page.locator(".df-lib-toolbar input").click();
await page.keyboard.type("memory", { delay: 90 });
await wait(1600);
await page.locator(".df-doc-row").nth(1).click();
await wait(3200);
beat("end");

await ctx.close();
await browser.close();

const webm = readdirSync(out).find((f) => f.endsWith(".webm") && f !== "capture.webm");
if (webm) renameSync(join(out, webm), join(out, "capture.webm"));
writeFileSync(join(out, "beats.json"), JSON.stringify(beats, null, 2));
console.log("beats", beats);
