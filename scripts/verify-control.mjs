// Headless check of PRD journey 4 (commissioning-control task 8). Run with Windows node from the repo root:
//   node scripts/verify-control.mjs
// Drives the e2e build (WebView2 over CDP): Training mode → control panel of the simulated VFD →
// arm → run → set 30.0 Hz → read-back verified → reached target → software stop → audit log → report.
// The script never builds a frame: it reads the frame the app shows and checks it against the raw value
// and the audit log. Simulated devices only: this is not hardware evidence.
// Results: docs/verification/control-results.json.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const ROOT = resolve(import.meta.dirname, "..");
const OUT = join(ROOT, "docs", "verification");
const EXE = join(ROOT, "src-tauri", "target", "e2e", "release", "buslink-studio.exe");
const EXPORT_DIR = join(OUT, "export-control");
const PORT = 9334;
const STOP_LABEL = "Software stop. Not an emergency stop.";
mkdirSync(OUT, { recursive: true });
rmSync(EXPORT_DIR, { recursive: true, force: true });
mkdirSync(EXPORT_DIR, { recursive: true });

const results = { date: new Date().toISOString(), platform: "Windows (WebView2 in the e2e build)", journey: "PRD §5 journey 4 (simulated VFD)", steps: [] };
const step = (name, ok, detail = "") => {
  results.steps.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const waitFor = (page, fn, timeout, arg) => page.waitForFunction(fn, { timeout, polling: 200 }, arg).then(() => true, () => false);
const text = (page, sel) => page.evaluate((s) => document.querySelector(s)?.textContent ?? "", sel);
const shot = (page, name) => page.screenshot({ path: join(OUT, name), fullPage: true });

async function confirmDialog(page) {
  const open = await waitFor(page, () => !!document.querySelector("dialog[open][data-testid=confirm-dialog]"), 8000);
  const info = open
    ? await page.evaluate(() => {
        const dd = (label) => {
          const dt = [...document.querySelectorAll(".confirm-list dt")].find((d) => d.textContent === label);
          return dt?.nextElementSibling?.textContent ?? "";
        };
        return {
          label: document.querySelector("[data-testid=confirm-send]")?.textContent ?? "",
          newValue: document.querySelector("[data-testid=confirm-new]")?.textContent ?? "",
          frame: document.querySelector("[data-testid=confirm-frame]")?.textContent ?? "",
          unit: dd("Unit ID"),
          fn: dd("Function"),
          raw: dd("Raw register value"),
          register: dd("Register"),
          focus: document.activeElement?.dataset?.testid ?? "",
          fields: [...document.querySelectorAll(".confirm-list dt")].map((d) => d.textContent),
        };
      })
    : null;
  return { open, info };
}

/** The frame shown in the dialog agrees with the other dialog fields (unit, function, offset, raw value). */
function frameMatches(info) {
  const bytes = info.frame.trim().split(/\s+/).map((b) => Number.parseInt(b, 16));
  const fc = Number.parseInt(/FC(\d+)/.exec(info.fn)?.[1] ?? "-1", 10);
  const offset = Number.parseInt(/PDU offset (\d+)/.exec(info.register)?.[1] ?? "-1", 10);
  const raw = Number.parseInt(/\((\d+)\)/.exec(info.raw)?.[1] ?? "-1", 10);
  return (
    bytes.length === 8 &&
    bytes[0] === Number(info.unit) &&
    bytes[1] === fc &&
    bytes[2] * 256 + bytes[3] === offset &&
    bytes[4] * 256 + bytes[5] === raw
  );
}

async function run() {
  if (!existsSync(EXE)) return step("e2e app binary exists", false, EXE);
  const app = spawn(EXE, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`, BUSLINK_E2E_EXPORT_DIR: EXPORT_DIR },
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
    await page.evaluate(() => {
      localStorage.setItem("buslink.lang", "en");
      localStorage.setItem("buslink.user", "");
    });
    await page.reload();
    await waitFor(page, () => !!document.querySelector(".app-header"), 15000);

    await page.evaluate(() => (location.hash = "#/connect"));
    await page.waitForSelector("[data-testid=start-training]", { timeout: 10000 });
    await page.click("[data-testid=start-training]");
    const good = await waitFor(page, () => {
      const cards = [...document.querySelectorAll(".device-card")];
      return cards.length === 3 && cards.every((c) => c.querySelectorAll(".tile.q-good").length > 0);
    }, 20000);
    step("Training mode: 3 simulated devices GOOD", good);
    await page.evaluate(() => (location.hash = "#/dashboard"));
    await waitFor(page, () => document.querySelectorAll("[data-testid^=control-]").length > 0, 8000);
    const links = await page.evaluate(() => [...document.querySelectorAll("[data-testid^=control-]")].map((a) => a.getAttribute("href")));
    step("dashboard offers the control panel only for the simulated VFD and temperature controller", links.length === 2, links.join(", "));
    const vfdHref = links.find((l) => l.includes("vfd"));
    await page.evaluate((h) => (location.hash = h), vfdHref);
    await page.waitForSelector("[data-testid=arm-box]", { timeout: 10000 });
    await waitFor(page, () => !!document.querySelector("[data-testid=arm]"), 5000);

    const monitoring = await page.evaluate(() => ({
      banner: document.querySelector(".control .sim-banner")?.textContent ?? "",
      disabled: [...document.querySelectorAll(".cmd-box button")].every((b) => b.disabled),
      stopHidden: document.querySelector(".stop-box")?.hidden ?? false,
    }));
    step("starts in Monitoring: 'Simulated device' banner, commands disabled", /Simulated device/.test(monitoring.banner) && monitoring.disabled, JSON.stringify(monitoring));
    await shot(page, "20-control-monitoring.png");

    await page.click("[data-testid=arm]");
    const armErr = await waitFor(page, () => /audit/.test(document.querySelector("#arm-user-err")?.textContent ?? ""), 3000);
    step("arming without a user name is refused next to the field", armErr);
    await page.type("[data-testid=arm-user]", "trainee");
    await page.click("[data-testid=arm]");
    const armed = await waitFor(page, () => /Armed: control enabled for \d+ s/.test(document.querySelector("[data-testid=arm-text]")?.textContent ?? ""), 5000);
    const armedText = await text(page, "[data-testid=arm-text]");
    const stop = await page.evaluate(() => {
      const b = document.querySelector("[data-testid=cmd-stop]");
      return { visible: !!b && b.offsetParent !== null, label: b?.textContent ?? "" };
    });
    step("armed with a countdown", armed, armedText);
    step("software stop visible while armed, labelled 'Software stop. Not an emergency stop.'", stop.visible && stop.label === STOP_LABEL, stop.label);
    await sleep(2200);
    const counted = await text(page, "[data-testid=arm-text]");
    step("countdown runs", counted !== armedText, `${armedText} → ${counted}`);
    await shot(page, "21-control-armed.png");

    await page.click("[data-testid=cmd-run]");
    let d = await confirmDialog(page);
    step("run: confirm dialog opens with the safe choice focused", d.open && d.info.focus === "confirm-cancel", JSON.stringify(d.info));
    await page.click("[data-testid=confirm-send]");
    const runAck = await waitFor(page, () => /^Acknowledged/.test(document.querySelector("[data-testid=res-ack]")?.textContent ?? ""), 8000);
    const runFb = await waitFor(page, () => /^Reached target/.test(document.querySelector("[data-testid=res-feedback]")?.textContent ?? ""), 8000);
    step("run: acknowledged, then 'Running' bit measured", runAck && runFb, await text(page, "[data-testid=res-feedback]"));

    // Out of bounds: rejected next to the field, no dialog, nothing sent.
    await page.type("[data-testid=value-set_frequency]", "60");
    await page.click("[data-testid=cmd-set_frequency]");
    await sleep(500);
    const oob = await page.evaluate(() => ({
      err: document.querySelector("#val-set_frequency-err")?.textContent ?? "",
      dialog: !!document.querySelector("dialog[open]"),
    }));
    step("60 Hz is rejected next to the field (never clamped), no dialog", /Outside/.test(oob.err) && !oob.dialog, oob.err);

    await page.$eval("[data-testid=value-set_frequency]", (i) => {
      i.value = "";
      i.dispatchEvent(new Event("input"));
    });
    await page.type("[data-testid=value-set_frequency]", "30");
    const btnLabel = await text(page, "[data-testid=cmd-set_frequency]");
    step("button names the action and value", btnLabel === "Apply frequency setpoint 30.00 Hz", btnLabel);
    await page.click("[data-testid=cmd-set_frequency]");
    d = await confirmDialog(page);
    const fieldsOk = d.info && ["Device", "Unit ID", "Register", "Function", "Old value", "New value", "Raw register value", "Frame to send (hex)"].every((f) => d.info.fields.includes(f));
    const frame30 = d.info?.frame ?? "";
    step(
      "confirm dialog: device, unit ID, register, function, old → new, raw value, frame (hex) that matches them",
      d.open && fieldsOk && d.info.label === "Apply frequency setpoint 30.00 Hz" && d.info.newValue.includes("30.00 Hz") && /\(3000\)/.test(d.info.raw) && frameMatches(d.info),
      JSON.stringify(d.info),
    );
    await shot(page, "22-confirm-30hz.png");
    await page.click("[data-testid=confirm-send]");
    const rb = await waitFor(page, () => /^verified · 30.00 Hz/.test(document.querySelector("[data-testid=res-readback]")?.textContent ?? ""), 8000);
    const fbNow = await text(page, "[data-testid=res-feedback]");
    step("30.0 Hz: write acknowledged and read-back verified", rb, await text(page, "[data-testid=res-readback]"));
    step("process feedback shown separately (pending while the drive ramps)", /^(Pending|Reached)/.test(fbNow), fbNow);
    await shot(page, "23-ack-pending.png");
    const reached = await waitFor(page, () => /^Reached target/.test(document.querySelector("[data-testid=res-feedback]")?.textContent ?? ""), 15000);
    step("process reached target 30.0 Hz (measured output frequency)", reached, await text(page, "[data-testid=res-feedback]"));
    await shot(page, "24-reached-30hz.png");

    await page.click("[data-testid=cmd-stop]");
    d = await confirmDialog(page);
    step("software stop also needs confirmation", d.open, d.info?.label ?? "");
    await page.click("[data-testid=confirm-send]");
    const stopRb = await waitFor(page, () => /^verified/.test(document.querySelector("[data-testid=res-readback]")?.textContent ?? ""), 8000);
    const stopped = await waitFor(page, () => /^Reached target/.test(document.querySelector("[data-testid=res-feedback]")?.textContent ?? ""), 20000);
    step("software stop: read-back verified, output reached 0 Hz", stopRb && stopped, await text(page, "[data-testid=res-feedback]"));
    await shot(page, "25-stopped.png");

    await page.evaluate(() => (location.hash = "#/audit"));
    const audit = await waitFor(page, () => document.querySelectorAll("[data-testid=audit-table] tbody tr").length >= 10, 8000);
    const auditInfo = await page.evaluate((f) => {
      const rows = [...document.querySelectorAll("[data-testid=audit-table] tbody tr")].map((r) => r.textContent);
      return {
        n: rows.length,
        results: rows.filter((r) => r.includes("Result")).length,
        feedback: rows.filter((r) => r.includes("Process feedback")).length,
        frame: rows.some((r) => r.includes(`TX ${f}`)),
      };
    }, frame30);
    step("audit log lists results, process feedback and the frame shown in the dialog", audit && auditInfo.results >= 3 && auditInfo.feedback >= 3 && auditInfo.frame, JSON.stringify(auditInfo));
    await shot(page, "26-audit.png");
    await page.click("[data-testid=audit-export-csv]");
    await sleep(1500);
    const csv = readdirSync(EXPORT_DIR).find((f) => f.startsWith("buslink-audit") && f.endsWith(".csv"));
    const csvText = csv ? readFileSync(join(EXPORT_DIR, csv), "utf8") : "";
    step(
      "audit CSV export: ISO-8601 UTC times, set_frequency result verified",
      /^ts_utc,session_id,user,role,event/.test(csvText) && /\d{4}-\d\d-\d\dT[\d:.]+Z,.*result.*set_frequency.*verified/.test(csvText),
      csv ?? "no file",
    );
    await page.click("[data-testid=audit-export-xlsx]");
    await sleep(1500);
    const xlsx = readdirSync(EXPORT_DIR).find((f) => f.startsWith("buslink-audit") && f.endsWith(".xlsx"));
    step("audit XLSX export written", !!xlsx && readFileSync(join(EXPORT_DIR, xlsx)).subarray(0, 2).toString() === "PK", xlsx ?? "no file");

    await page.evaluate(() => (location.hash = "#/report"));
    const rep = await waitFor(page, () => !!document.querySelector("[data-testid=rep-commands] tbody tr"), 8000);
    const repInfo = await page.evaluate(() => ({
      devices: document.querySelectorAll("[data-testid=rep-devices] tbody tr").length,
      sim: document.querySelector(".report .sim-banner")?.textContent ?? "",
      commands: [...document.querySelectorAll("[data-testid=rep-commands] tbody tr")].map((r) => r.textContent).join(" | "),
      version: document.querySelector("[data-testid=rep-devices]")?.textContent.includes("sim.vfd-basic 1.1.0") ?? false,
    }));
    step(
      "session report: devices with profile versions, simulated note, commands with read-back and feedback",
      rep && repInfo.devices === 3 && repInfo.version && /simulated/.test(repInfo.sim) && /set_frequency.*verified.*reached/.test(repInfo.commands),
      JSON.stringify({ ...repInfo, commands: repInfo.commands.slice(0, 300) }),
    );
    await shot(page, "27-report.png");
    await page.click("[data-testid=report-save]");
    await sleep(1500);
    const html = readdirSync(EXPORT_DIR).find((f) => f.endsWith(".html"));
    const htmlText = html ? readFileSync(join(EXPORT_DIR, html), "utf8") : "";
    step("report saved as self-contained HTML (no script)", htmlText.startsWith("<!DOCTYPE html>") && !/<script/i.test(htmlText) && htmlText.includes("set_frequency"), html ?? "no file");
    const pdf = await page.pdf({ format: "A4", printBackground: true }).catch((e) => e);
    if (pdf instanceof Uint8Array) {
      writeFileSync(join(EXPORT_DIR, "report.pdf"), pdf);
      step("report prints to PDF (print media CSS, via CDP Page.printToPDF)", pdf.length > 1000, `${pdf.length} bytes`);
    } else {
      step("report prints to PDF (print media CSS, via CDP Page.printToPDF)", false, String(pdf));
    }

    // Disconnect drops the arm; nothing resumes on reconnect.
    await page.evaluate((h) => (location.hash = h), vfdHref);
    await page.waitForSelector("[data-testid=arm]", { timeout: 8000 });
    await page.click("[data-testid=arm]");
    await waitFor(page, () => /Armed/.test(document.querySelector("[data-testid=arm-text]")?.textContent ?? ""), 5000);
    await page.evaluate(() => (location.hash = "#/connect"));
    await sleep(800);
    const disc = await page.evaluateHandle(() => [...document.querySelectorAll("button")].find((b) => /^Disconnect$/.test(b.textContent ?? "")));
    if (disc.asElement()) await disc.asElement().click();
    await sleep(1500);
    await page.evaluate((h) => (location.hash = h), vfdHref);
    const dropped = await waitFor(page, () => /disconnected|stopped polling/.test(document.querySelector("[data-testid=disarm-reason]")?.textContent ?? ""), 6000);
    step("disconnect drops the arm and says so", dropped, await text(page, "[data-testid=disarm-reason]"));
    await shot(page, "28-disarmed-after-disconnect.png");
  } catch (e) {
    step("journey ran without an exception", false, String(e?.stack ?? e));
  } finally {
    await browser?.disconnect().catch(() => {});
    app.kill();
  }
}

await run();
const passed = results.steps.filter((s) => s.ok).length;
results.summary = `${passed}/${results.steps.length} passed`;
writeFileSync(join(OUT, "control-results.json"), `${JSON.stringify(results, null, 2)}\n`);
console.log(results.summary);
process.exit(passed === results.steps.length ? 0 : 1);
