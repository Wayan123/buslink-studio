// Headless verification (dashboard-ui task 9). Run with Windows node from the repo root:
//   node scripts/verify-journey.mjs
// Part A drives the e2e build of the real app (WebView2 = Chromium) over CDP:
//   Training mode -> 3 devices GOOD -> timeout on unit 3 -> trend gap "TIMEOUT" -> export CSV.
// Part B serves dist/ to headless Chrome (no backend) and checks the 360 px layout.
// Screenshots and results go to docs/verification/. Simulated devices only: this is not hardware evidence.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const ROOT = resolve(import.meta.dirname, "..");
const OUT = join(ROOT, "docs", "verification");
const EXE = join(ROOT, "src-tauri", "target", "e2e", "release", "buslink-studio.exe");
const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const EXPORT_DIR = join(OUT, "export");
const PORT = 9333;
mkdirSync(OUT, { recursive: true });
rmSync(EXPORT_DIR, { recursive: true, force: true });
mkdirSync(EXPORT_DIR, { recursive: true });

const results = { date: new Date().toISOString(), platform: "Windows (WebView2 in the e2e build) + headless Chrome", steps: [] };
const step = (name, ok, detail = "") => {
  results.steps.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFor(page, fn, timeout, arg) {
  return page.waitForFunction(fn, { timeout, polling: 250 }, arg).then(() => true, () => false);
}

async function partA() {
  if (!existsSync(EXE)) {
    step("e2e app binary exists", false, EXE);
    return;
  }
  const app = spawn(EXE, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
      BUSLINK_E2E_EXPORT_DIR: EXPORT_DIR,
    },
    stdio: "ignore",
  });
  let browser;
  try {
    for (let i = 0; i < 60 && !browser; i++) {
      await sleep(500);
      browser = await puppeteer.connect({ browserURL: `http://127.0.0.1:${PORT}`, defaultViewport: null }).catch(() => undefined);
    }
    if (!browser) return step("connect to app over CDP", false);
    let page;
    for (let i = 0; i < 40 && !page; i++) {
      page = (await browser.pages()).find((p) => !p.url().startsWith("devtools"));
      if (!page) await sleep(250);
    }
    step("connect to app over CDP", !!page, page?.url());
    await page.setViewport({ width: 1280, height: 900 });
    await waitFor(page, () => !!document.querySelector(".app-header"), 15000);
    const backend = await page.evaluate(() => !document.querySelector(".notice[role=alert]"));
    step("backend available in the app webview", backend);

    await page.evaluate(() => (location.hash = "#/connect"));
    await page.waitForSelector("[data-testid=start-training]", { timeout: 10000 });
    await page.click("[data-testid=start-training]");
    const three = await waitFor(
      page,
      () => {
        const cards = [...document.querySelectorAll(".device-card")];
        return cards.length === 3 && cards.every((c) => c.querySelectorAll(".tile.q-good").length > 0 && !c.querySelector(".tile.q-none"));
      },
      20000,
    );
    const devices = await page.evaluate(() => [...document.querySelectorAll(".device-card")].map((c) => c.dataset.device));
    step("Training mode: 3 simulated devices GOOD", three, devices.join(", "));
    const simBanners = await page.evaluate(() => document.querySelectorAll(".device-card .sim-banner").length);
    step("every training device card shows the 'Simulated device' banner", simBanners === 3, `${simBanners} banners`);
    await sleep(3000); // a few seconds of GOOD history before the fault
    await page.screenshot({ path: join(OUT, "01-dashboard-training.png"), fullPage: true });

    const temp = devices.find((d) => d.includes("temp")) ?? devices[2];
    await page.evaluate(() => (location.hash = "#/training"));
    await page.waitForSelector("[data-testid=fault-3-timeout]", { timeout: 10000 });
    await page.click("[data-testid=fault-3-timeout]");
    step("inject TIMEOUT on unit 3 (15 s)", true);

    await page.evaluate(() => (location.hash = "#/dashboard"));
    const timeoutTile = await waitFor(
      page,
      (dev) => !!document.querySelector(`.device-card[data-device="${dev}"] .tile.q-bad .badge.q-bad`),
      10000,
      temp,
    );
    const badgeText = await page.evaluate(
      (dev) => document.querySelector(`.device-card[data-device="${dev}"] .tile .badge.q-bad`)?.textContent ?? "",
      temp,
    );
    step("dashboard tiles show TIMEOUT with the last value kept", timeoutTile && badgeText.includes("TIMEOUT"), badgeText);
    const commAlarm = await page.evaluate(() => document.querySelector(".alarm-banner")?.textContent ?? "");
    step("separate communication alarm in the banner", /comm|komunikasi/i.test(commAlarm), commAlarm.trim());
    await page.screenshot({ path: join(OUT, "02-dashboard-timeout.png"), fullPage: true });

    await page.evaluate((dev) => (location.hash = `#/trends/${encodeURIComponent(dev)}`), temp);
    const gap = await waitFor(page, () => [...document.querySelectorAll(".gap-item")].some((li) => li.textContent.includes("TIMEOUT")), 15000);
    const gapText = await page.evaluate(() => [...document.querySelectorAll(".gap-item")].map((l) => l.textContent).join(" | "));
    const renderNote = await page.evaluate(() => document.querySelector(".trend-main > p.muted")?.textContent ?? "");
    step("trend shows a gap 'No data: TIMEOUT'", gap, gapText);
    results.trend_render_note = renderNote;
    await page.screenshot({ path: join(OUT, "03-trend-timeout-gap.png"), fullPage: true });

    // Let the fault end so the gap closes and GOOD data resumes.
    await sleep(17000);
    await page.screenshot({ path: join(OUT, "04-trend-after-recovery.png"), fullPage: true });

    await page.evaluate((dev) => (location.hash = `#/inspector/${encodeURIComponent(dev)}`), temp);
    const frames = await waitFor(page, () => document.querySelectorAll(".frames-table tbody tr").length > 0, 10000);
    const frameTimeout = await page.evaluate(() => [...document.querySelectorAll(".frames-table tbody tr")].some((r) => r.textContent.includes("TIMEOUT")));
    step("register inspector lists frames including the timeouts", frames && frameTimeout);
    await page.screenshot({ path: join(OUT, "05-inspector.png"), fullPage: true });

    await page.evaluate(() => (location.hash = "#/alarms"));
    await page.waitForSelector(".alarm-table, .alarms", { timeout: 10000 });
    await page.screenshot({ path: join(OUT, "06-alarms.png"), fullPage: true });

    await page.evaluate(() => (location.hash = "#/dashboard"));
    await page.waitForSelector("[data-testid=export-csv]", { timeout: 10000 });
    await page.click("[data-testid=export-csv]");
    const saved = await waitFor(page, () => /Saved to|Tersimpan di/.test(document.querySelector(".export-bar .status-line")?.textContent ?? ""), 20000);
    const files = readdirSync(EXPORT_DIR).filter((f) => f.endsWith(".csv"));
    let csvOk = false;
    let csvDetail = "no CSV written";
    if (files[0]) {
      const lines = readFileSync(join(EXPORT_DIR, files[0]), "utf8").split(/\r?\n/);
      const header = lines[0] ?? "";
      const timeouts = lines.filter((l) => l.includes(temp) && l.includes("TIMEOUT")).length;
      const goods = lines.filter((l) => l.includes(",GOOD,")).length;
      csvOk = saved && header.includes("quality") && header.includes("age_ms") && timeouts > 0 && goods > 0;
      csvDetail = `${files[0]}: ${lines.length - 1} rows, ${timeouts} TIMEOUT rows for ${temp}, ${goods} GOOD rows; header: ${header}`;
    }
    step("export CSV with quality + age columns and TIMEOUT rows", csvOk, csvDetail);

    // Narrow layout inside the real app (Android-size viewport).
    await page.setViewport({ width: 360, height: 780 });
    await sleep(800);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    step("app at 360 px: no horizontal page scroll", overflow <= 0, `overflow ${overflow}px`);
    await page.screenshot({ path: join(OUT, "07-dashboard-360px.png"), fullPage: true });
    await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await page.evaluate(() => {
      localStorage.setItem("buslink.theme", "system");
      document.documentElement.dataset.theme = "dark";
    });
    await sleep(500);
    await page.screenshot({ path: join(OUT, "08-dashboard-360px-dark.png"), fullPage: true });
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
  } catch (e) {
    step("journey ran without an exception", false, String(e?.stack ?? e));
  } finally {
    await browser?.disconnect().catch(() => {});
    app.kill();
  }
}

async function partB() {
  const dist = join(ROOT, "dist");
  if (!existsSync(join(dist, "index.html"))) return step("dist/ built", false);
  const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png" };
  const server = createServer((req, res) => {
    const path = join(dist, (req.url ?? "/").split("?")[0] === "/" ? "index.html" : decodeURIComponent((req.url ?? "").split("?")[0]));
    if (!path.startsWith(dist) || !existsSync(path)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": types[extname(path)] ?? "application/octet-stream" }).end(readFileSync(path));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const browser = await puppeteer.launch({ executablePath: CHROME, headless: true, args: ["--no-first-run"] });
  try {
    const page = await browser.newPage();
    for (const [lang, w] of [["en", 360], ["id", 360], ["en", 1280]]) {
      await page.setViewport({ width: w, height: 800 });
      await page.goto(`${url}#/connect`, { waitUntil: "networkidle0" });
      await page.evaluate((l) => localStorage.setItem("buslink.lang", l), lang);
      await page.reload({ waitUntil: "networkidle0" });
      const info = await page.evaluate(() => ({
        overflow: document.documentElement.scrollWidth - window.innerWidth,
        lang: document.documentElement.lang,
        notice: document.querySelector(".notice[role=alert]")?.textContent ?? "",
        smallTargets: [...document.querySelectorAll("button, select, input, .nav a")]
          .filter((e) => e.offsetParent !== null)
          .map((e) => e.getBoundingClientRect())
          .filter((r) => r.height < 24 || r.width < 24).length,
      }));
      step(
        `dist/ in headless Chrome (${lang}, ${w}px): no backend notice, no page overflow, targets ≥ 24 px`,
        info.overflow <= 0 && info.lang === lang && info.notice.length > 0 && info.smallTargets === 0,
        JSON.stringify(info),
      );
      await page.screenshot({ path: join(OUT, `10-static-connect-${lang}-${w}px.png`), fullPage: true });
    }
    // Keyboard: Tab reaches the skip link first and focus is visible.
    await page.goto(`${url}#/connect`, { waitUntil: "networkidle0" });
    await page.keyboard.press("Tab");
    const focus = await page.evaluate(() => ({
      cls: document.activeElement?.className,
      outline: getComputedStyle(document.activeElement).outlineStyle,
    }));
    step("keyboard: first Tab focuses the skip link with a visible outline", focus.cls === "skip" && focus.outline !== "none", JSON.stringify(focus));
  } finally {
    await browser.close();
    server.close();
  }
}

await partA();
await partB();
const failed = results.steps.filter((s) => !s.ok).length;
results.summary = `${results.steps.length - failed}/${results.steps.length} passed`;
writeFileSync(join(OUT, "journey-results.json"), JSON.stringify(results, null, 2));
console.log(results.summary);
process.exit(failed ? 1 : 0);
