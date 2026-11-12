// Headless verification of PRD journey 3 (lan-mqtt task 9). Run with Windows node from the repo root:
//   node scripts/verify-lan.mjs
// The e2e build of the real app runs Training mode (simulated devices) and shares the read-only LAN
// view on a private address of this PC. A SECOND, separate headless Chrome (its own profile, no
// access to the app) plays the colleague's browser. It must see live values with quality, trends and
// the read-only notice, and nothing that connects, arms, controls or exports.
// Screenshots and results go to docs/verification/. The token and the QR code are blurred in the
// screenshots of the app (safety rule 14). Simulated devices only: this is not hardware evidence.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const ROOT = resolve(import.meta.dirname, "..");
const OUT = join(ROOT, "docs", "verification");
const EXE = join(ROOT, "src-tauri", "target", "e2e", "release", "buslink-studio.exe");
const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9334;
mkdirSync(OUT, { recursive: true });

const results = { date: new Date().toISOString(), platform: "Windows (WebView2 in the e2e build) + a second headless Chrome as the LAN browser", steps: [] };
const step = (name, ok, detail = "") => {
  results.steps.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFor(page, fn, timeout, arg) {
  return page.waitForFunction(fn, { timeout, polling: 250 }, arg).then(() => true, () => false);
}
/** Blur the secret link and the QR code before any screenshot of the host app. */
const MASK = "[data-testid=share-url], canvas.qr { filter: blur(14px) !important; }";

async function readUrl(app) {
  return app.$eval("[data-testid=share-url]", (el) => el.value);
}

async function main() {
  if (!existsSync(EXE)) return step("e2e app binary exists", false, EXE);
  const appProc = spawn(EXE, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, BUSLINK_E2E_EXPORT_DIR: tmpdir() },
    stdio: "ignore",
  });
  let appBrowser;
  let lanBrowser;
  try {
    for (let i = 0; i < 60 && !appBrowser; i++) {
      await sleep(500);
      appBrowser = await puppeteer.connect({ browserURL: `http://127.0.0.1:${PORT}`, defaultViewport: null }).catch(() => undefined);
    }
    if (!appBrowser) return step("connect to the app over CDP", false);
    let app;
    for (let i = 0; i < 40 && !app; i++) {
      app = (await appBrowser.pages()).find((p) => !p.url().startsWith("devtools"));
      if (!app) await sleep(250);
    }
    step("connect to the app over CDP", !!app);
    await app.setViewport({ width: 1280, height: 900 });
    await waitFor(app, () => !!document.querySelector(".app-header"), 15000);

    // --- Training mode: three simulated devices ---
    await app.evaluate(() => (location.hash = "#/connect"));
    await app.waitForSelector("[data-testid=start-training]", { timeout: 10000 });
    await app.click("[data-testid=start-training]");
    const three = await waitFor(
      app,
      () => {
        const cards = [...document.querySelectorAll(".device-card")];
        return cards.length === 3 && cards.every((c) => c.querySelectorAll(".tile.q-good").length > 0);
      },
      20000,
    );
    const devices = await app.evaluate(() => [...document.querySelectorAll(".device-card")].map((c) => c.dataset.device));
    step("Training mode: 3 simulated devices GOOD in the app", three, devices.join(", "));
    const temp = devices.find((d) => d.includes("temp")) ?? devices[2];

    // --- sharing is off at start ---
    const indOff = await app.evaluate(() => document.querySelectorAll("[data-testid=ind-lan], [data-testid=ind-mqtt]").length);
    await app.evaluate(() => (location.hash = "#/share"));
    await app.waitForSelector("[data-testid=share-start]", { timeout: 10000 });
    const urlHidden = await app.evaluate(() => document.querySelector("#share-url")?.closest("section")?.hidden ?? true);
    step("sharing is off at start: no link, no header indicator", urlHidden && indOff === 0);
    await app.addStyleTag({ content: MASK });
    await app.screenshot({ path: join(OUT, "30-share-off.png"), fullPage: true });

    // --- start sharing on a private address ---
    const options = await app.$$eval("#share-iface option", (os) => os.map((o) => ({ ip: o.value, label: o.textContent })));
    results.interfaces = options.map((o) => o.label);
    const pick = options.find((o) => !o.ip.startsWith("127.")) ?? options[0];
    step("interface list offers private addresses only", options.length > 0 && options.every((o) => /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|127\.)/.test(o.ip)), options.map((o) => o.ip).join(", "));
    if (!pick) return step("an interface to share on", false);
    await app.select("#share-iface", pick.ip);
    const defaultPort = await app.$eval("#share-port", (el) => el.value);
    step("default port is 8443", defaultPort === "8443", defaultPort);
    await app.click("[data-testid=share-start]");
    await app.waitForSelector("[data-testid=share-url]", { visible: true, timeout: 10000 });
    const url = await readUrl(app);
    const urlRx = new RegExp(`^https://${pick.ip.replaceAll(".", "\\.")}:8443/\\?t=[0-9a-f]{32}$`);
    step("link is https://<ip>:8443/?t=<128-bit hex token>", urlRx.test(url), url.replace(/t=.*/, "t=<token>"));
    const qrOk = await app.$eval("[data-testid=share-qr]", (c) => {
      const ctx = c.getContext("2d");
      const d = ctx.getImageData(0, 0, c.width, c.height).data;
      let dark = 0;
      for (let i = 0; i < d.length; i += 4) if (d[i] < 64) dark++;
      return c.width > 100 && dark > 500;
    });
    step("QR code is drawn", qrOk);
    const offVisible = await app.evaluate(() => {
      const f = document.querySelector("#share-off-h")?.closest("form");
      return !!f && f.getBoundingClientRect().height > 0;
    });
    step("the 'Sharing is off' form is not visible while sharing is on", !offVisible);
    const fp = await app.$eval("#share-on-h", (h) => h.closest("section").querySelector("code")?.textContent ?? "");
    step("certificate fingerprint is shown (SHA-256)", /^([0-9A-F]{2}:){31}[0-9A-F]{2}$/.test(fp), fp);
    const indOn = await waitFor(app, () => !!document.querySelector("[data-testid=ind-lan]"), 8000);
    step("header shows that the network view is on", indOn);
    await sleep(500);
    await app.screenshot({ path: join(OUT, "31-share-on.png"), fullPage: true });

    // --- the colleague's browser: separate Chrome, own profile ---
    lanBrowser = await puppeteer.launch({ executablePath: CHROME, headless: true, acceptInsecureCerts: true, args: ["--no-sandbox"] });
    const lan = await lanBrowser.newPage();
    const responses = [];
    lan.on("response", (r) => responses.push({ url: r.url().replace(/t=[0-9a-f]+/, "t=<token>"), status: r.status(), method: r.request().method() }));
    await lan.setViewport({ width: 1280, height: 900 });
    await lan.goto(url, { waitUntil: "load", timeout: 20000 });
    await waitFor(lan, () => !!document.querySelector(".app-header"), 15000);
    const mode = await lan.evaluate(() => ({
      meta: document.querySelector('meta[name="buslink-mode"]')?.getAttribute("content"),
      body: document.body.classList.contains("lan"),
      title: document.title,
    }));
    step("server selects the read-only build (meta buslink-mode=lan)", mode.meta === "lan" && mode.body, JSON.stringify(mode));

    const cardsOk = await waitFor(
      lan,
      () => {
        const cards = [...document.querySelectorAll(".device-card")];
        return cards.length === 3 && cards.every((c) => c.querySelectorAll(".tile.q-good").length > 0);
      },
      20000,
    );
    step("LAN browser shows the 3 devices with GOOD tiles", cardsOk);
    const live = await waitFor(lan, () => /connected|tersambung/i.test(document.querySelector("[data-testid=lan-stream]")?.textContent ?? "") && !/lost|terputus/i.test(document.querySelector("[data-testid=lan-stream]")?.textContent ?? ""), 10000);
    step("live stream connected", live, await lan.$eval("[data-testid=lan-stream]", (e) => e.textContent).catch(() => ""));
    const ro = await lan.$eval("[data-testid=lan-readonly]", (e) => e.textContent).catch(() => "");
    step("read-only notice is shown", /read-only|hanya-baca/i.test(ro), ro);
    const simBanners = await lan.evaluate(() => document.querySelectorAll(".device-card .sim-banner").length);
    step("every device card still says 'Simulated device'", simBanners === 3, `${simBanners} banners`);
    const tileText = await lan.evaluate((dev) => document.querySelector(`.device-card[data-device="${dev}"] .tile`)?.textContent ?? "", temp);
    step("tiles carry value, unit, quality and age", /GOOD|good|baik/i.test(tileText) || /\d/.test(tileText), tileText.replace(/\s+/g, " ").slice(0, 120));
    await lan.screenshot({ path: join(OUT, "32-lan-dashboard.png"), fullPage: true });

    // --- nothing that connects, arms, controls or exports ---
    const ui = await lan.evaluate(() => ({
      nav: [...document.querySelectorAll("nav.nav a")].map((a) => a.getAttribute("href")),
      buttons: [...document.querySelectorAll("button")].map((b) => b.textContent.trim()),
      forbidden: [...document.querySelectorAll("[data-testid^=export], [data-testid^=control], [data-testid=start-training], a[href^='#/connect'], a[href^='#/control'], a[href^='#/commission'], a[href^='#/audit'], a[href^='#/report'], a[href^='#/alarms'], a[href^='#/inspector']")].length,
    }));
    step("navigation is overview + trends only", JSON.stringify(ui.nav) === JSON.stringify(["#/dashboard", "#/trends"]), ui.nav.join(", "));
    const bad = ui.buttons.filter((b) => /export|ekspor|connect|sambung|arm\b|apply|terapkan|stop|run\b|setpoint|write|tulis|csv|xlsx/i.test(b));
    step("no connect, arm, write or export controls in the DOM", ui.forbidden === 0 && bad.length === 0, `buttons: ${ui.buttons.join(" | ")}`);
    results.lan_buttons = ui.buttons;

    // --- cookie and routes ---
    const cookie = (await lan.cookies()).find((c) => c.name === "bl_t");
    step("session cookie is Secure, HttpOnly, SameSite=Strict", !!cookie && cookie.secure && cookie.httpOnly && cookie.sameSite === "Strict");
    const probes = await lan.evaluate(async () => {
      const out = {};
      const tryIt = async (label, path, init) => {
        const r = await fetch(path, init).catch((e) => ({ status: `error ${e}` }));
        out[label] = r.status;
      };
      await tryIt("GET /api/snapshot (cookie only)", "/api/snapshot");
      await tryIt("POST /api/snapshot", "/api/snapshot", { method: "POST" });
      await tryIt("POST /api/write", "/api/write", { method: "POST", body: "{}" });
      await tryIt("PUT /api/stream", "/api/stream", { method: "PUT" });
      await tryIt("DELETE /", "/", { method: "DELETE" });
      await tryIt("GET /api/command", "/api/command");
      return out;
    });
    results.probes = probes;
    step(
      "browser probes: GET with cookie 200; POST/PUT/DELETE never 2xx; unknown routes 404",
      probes["GET /api/snapshot (cookie only)"] === 200 &&
        [probes["POST /api/snapshot"], probes["POST /api/write"], probes["PUT /api/stream"], probes["DELETE /"]].every((s) => s === 405 || s === 400) &&
        probes["GET /api/command"] === 404,
      JSON.stringify(probes),
    );
    // A browser without the token (fresh context, no cookie) is refused.
    const ctx = await lanBrowser.createBrowserContext();
    const anon = await ctx.newPage();
    const base = url.replace(/\?t=.*/, "");
    const r1 = await anon.goto(base, { waitUntil: "load" });
    const r2 = await anon.goto(`${base}?t=${"0".repeat(32)}`, { waitUntil: "load" });
    step("no token / wrong token: 401", r1.status() === 401 && r2.status() === 401, `${r1.status()}, ${r2.status()}`);
    await anon.screenshot({ path: join(OUT, "33-lan-no-token.png") });
    await ctx.close();

    // --- the viewer shows up in the app ---
    await app.bringToFront();
    const viewer = await waitFor(app, () => document.querySelectorAll(".share table tbody tr").length >= 1, 8000);
    step("the app lists the connected viewer", viewer);
    await app.screenshot({ path: join(OUT, "34-share-viewer.png"), fullPage: true });

    // --- a fault on the host reaches the browser, with its quality ---
    await app.evaluate(() => (location.hash = "#/training"));
    await app.waitForSelector("[data-testid=fault-3-timeout]", { timeout: 10000 });
    await app.click("[data-testid=fault-3-timeout]");
    const lanBad = await waitFor(lan, (dev) => !!document.querySelector(`.device-card[data-device="${dev}"] .tile.q-bad .badge.q-bad`), 12000, temp);
    const badge = await lan.evaluate((dev) => document.querySelector(`.device-card[data-device="${dev}"] .tile .badge.q-bad`)?.textContent ?? "", temp);
    step("TIMEOUT on the host shows in the browser with the quality badge", lanBad && badge.includes("TIMEOUT"), badge);
    const comm = await lan.evaluate(() => document.querySelector(".alarm-banner")?.textContent ?? "");
    step("communication alarm count is in the browser banner (display only)", /comm|komunikasi|1/.test(comm), comm.trim());
    const bannerIsLink = await lan.evaluate(() => document.querySelector(".alarm-banner")?.tagName);
    step("the banner is not a link to the alarm list", bannerIsLink === "DIV", String(bannerIsLink));
    await lan.screenshot({ path: join(OUT, "35-lan-timeout.png"), fullPage: true });

    // --- trends in the browser: live gap and server-side history ---
    await lan.evaluate((dev) => (location.hash = `#/trends/${encodeURIComponent(dev)}`), temp);
    const gap = await waitFor(lan, () => [...document.querySelectorAll(".gap-item")].some((li) => li.textContent.includes("TIMEOUT")), 15000);
    step("browser trend shows the gap 'No data: TIMEOUT'", gap, await lan.evaluate(() => [...document.querySelectorAll(".gap-item")].map((l) => l.textContent).join(" | ")));
    await lan.click("[data-range='1h']");
    const hist = await waitFor(lan, () => !!document.querySelector(".charts canvas"), 8000);
    await sleep(800);
    const trendReq = responses.find((r) => r.url.includes("/api/trend") && r.status === 200 && r.method === "GET");
    step("1 h history comes from GET /api/trend (decimated on the host)", hist && !!trendReq, trendReq ? trendReq.url : "no /api/trend response");
    await lan.screenshot({ path: join(OUT, "36-lan-trend-gap.png"), fullPage: true });

    // --- narrow layout ---
    await lan.evaluate(() => (location.hash = "#/dashboard"));
    await lan.setViewport({ width: 360, height: 780 });
    await sleep(700);
    const overflow = await lan.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    step("LAN view at 360 px: no horizontal page scroll", overflow <= 0, `overflow ${overflow}px`);
    await lan.screenshot({ path: join(OUT, "37-lan-360px.png"), fullPage: true });
    await lan.setViewport({ width: 1280, height: 900 });

    // --- stop closes everything; a restart is a new secret ---
    await app.bringToFront();
    await app.evaluate(() => (location.hash = "#/share"));
    await app.waitForSelector("[data-testid=share-stop]", { visible: true, timeout: 10000 });
    await app.click("[data-testid=share-stop]");
    const dropped = await waitFor(lan, () => /lost|terputus|reconnecting|menyambung ulang/i.test(document.querySelector("[data-testid=lan-stream]")?.textContent ?? ""), 10000);
    step("Stop sharing closes the browser's stream (it says 'reconnecting')", dropped);
    await lan.screenshot({ path: join(OUT, "38-lan-after-stop.png"), fullPage: true });
    const probe = await lanBrowser.newPage();
    const refused = await probe.goto(url, { timeout: 8000 }).then(() => false, () => true);
    step("the port refuses new connections after Stop", refused);
    await probe.close();
    const offAgain = await waitFor(app, () => document.querySelector("#share-off-h")?.closest("form")?.hidden === false, 8000);
    const indGone = await app.evaluate(() => document.querySelectorAll("[data-testid=ind-lan]").length === 0);
    step("the app shows sharing off again and the header indicator is gone", offAgain && (await waitFor(app, () => document.querySelectorAll("[data-testid=ind-lan]").length === 0, 8000)) && indGone !== undefined);

    await app.select("#share-iface", pick.ip);
    await app.click("[data-testid=share-start]");
    await app.waitForSelector("[data-testid=share-url]", { visible: true, timeout: 10000 });
    const url2 = await readUrl(app);
    step("restart issues a new token", url2 !== url && urlRx.test(url2));
    const stale = await lanBrowser.newPage();
    const old = await stale.goto(url, { waitUntil: "load", timeout: 10000 }).catch(() => null);
    step("the old link answers 401 after the restart", old?.status() === 401, String(old?.status()));
    await stale.close();
    await app.click("[data-testid=share-stop]");
    await sleep(500);
  } catch (e) {
    step("journey ran without an exception", false, String(e?.stack ?? e));
  } finally {
    await lanBrowser?.close().catch(() => {});
    await appBrowser?.disconnect().catch(() => {});
    appProc.kill();
  }
}

await main();
const failed = results.steps.filter((s) => !s.ok);
results.summary = { passed: results.steps.length - failed.length, failed: failed.length };
writeFileSync(join(OUT, "lan-results.json"), JSON.stringify(results, null, 2));
console.log(`\n${results.summary.passed}/${results.steps.length} passed`);
process.exit(failed.length ? 1 : 0);
