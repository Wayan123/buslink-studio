// Session report (commissioning-control Req 5, PRD R19): devices, profile versions, quality statistics,
// alarms, commands with read-back and process feedback, commissioning runs. Built with text nodes only;
// "Save HTML" serializes that DOM (text is escaped by the serializer); "Print / save as PDF" uses the
// WebView print dialog.
import { api, errorText, hasBackend } from "../api.ts";
import { clear, h } from "../dom.ts";
import { fmtLocalDateTime, fmtNumber, fmtUtc } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { commandRuns } from "../control.ts";
import type { ReportData } from "../types.ts";
import { statusLine, type View } from "./view.ts";

const QUALITIES = ["GOOD", "TIMEOUT", "CRC_ERROR", "EXCEPTION", "STALE", "DECODE_ERROR", "DISCONNECTED"];

/** Plain-text cell helper: every value becomes a text node. */
function table(head: string[], rows: (string | number)[][], testid?: string): HTMLElement {
  return h(
    "table",
    { class: "report-table", "data-testid": testid },
    h("thead", null, h("tr", null, ...head.map((c) => h("th", { scope: "col" }, c)))),
    h("tbody", null, ...rows.map((r) => h("tr", null, ...r.map((c) => h("td", null, String(c)))))),
  );
}

export function renderReport(d: ReportData): HTMLElement {
  const L = lang();
  const n = (v: number | null | undefined, dec?: number) => (v === null || v === undefined ? "—" : fmtNumber(v, L, dec));
  const when = (ms: number) => `${fmtLocalDateTime(ms, L)} (${fmtUtc(ms)})`;
  const anySim = d.devices.some((x) => x.simulated_only);
  const runs = commandRuns(d.audit);
  const wizard = d.audit.filter((a) => a.event === "journal" || a.event === "verify");
  return h(
    "article",
    { class: "report", "data-testid": "report" },
    h("h1", null, t("rep.title")),
    h(
      "dl",
      { class: "report-meta" },
      h("dt", null, t("rep.session")),
      h("dd", null, d.session.id),
      h("dt", null, t("rep.started")),
      h("dd", null, when(d.session.started_ms)),
      h("dt", null, t("rep.generated")),
      h("dd", null, when(d.generated_ms)),
      h("dt", null, t("rep.app")),
      h("dd", null, `BusLink Studio ${d.app_version} · ${d.session.platform}`),
    ),
    anySim ? h("p", { class: "sim-banner", role: "note" }, t("rep.simulated_note")) : null,
    h("p", { class: "muted small" }, t("rep.no_claims")),
    h("h2", null, t("rep.devices")),
    table(
      [t("rep.col.device"), t("rep.col.unit"), t("rep.col.link"), t("rep.col.profile"), t("rep.col.support")],
      d.devices.map((x) => [
        x.name,
        x.unit_id,
        x.link,
        `${x.profile_id} ${x.profile_version}`,
        `${x.support_level ? t(`support.${x.support_level}`) : "—"}${x.simulated_only ? ` · ${t("sim.banner")}` : ""}`,
      ]),
      "rep-devices",
    ),
    h("h2", null, t("rep.quality")),
    table(
      [t("rep.col.device"), ...QUALITIES, t("rep.col.requests"), t("rep.col.timeouts"), t("rep.col.crc"), t("rep.col.exceptions"), t("rep.col.cycle")],
      d.devices.map((x) => [
        x.name,
        ...QUALITIES.map((q) => n(x.quality[q] ?? 0)),
        n(x.counters?.requests),
        n(x.counters?.timeouts),
        n(x.counters?.crc_errors),
        n(x.counters?.exceptions),
        x.counters?.cycle_ms !== null && x.counters?.cycle_ms !== undefined ? `${n(x.counters.cycle_ms)} ms` : "—",
      ]),
      "rep-quality",
    ),
    h("p", { class: "muted small" }, t("rep.quality.note")),
    h("h2", null, t("rep.alarms")),
    h(
      "p",
      null,
      t("rep.alarms.summary", {
        p1: d.alarms.activations[0],
        p2: d.alarms.activations[1],
        p3: d.alarms.activations[2],
        active: d.alarms.summary.counts.reduce((a, b) => a + b, 0),
        unacked: d.alarms.summary.unacked,
      }),
    ),
    d.alarms.events.length
      ? table(
          [t("rep.col.time"), t("rep.col.device"), t("rep.col.priority"), t("rep.col.text")],
          d.alarms.events.slice(-200).map((e) => [when(e.ts_ms), e.device ?? "—", e.severity, e.text]),
          "rep-alarms",
        )
      : h("p", { class: "muted" }, t("rep.none")),
    h("h2", null, t("rep.commands")),
    runs.length
      ? table(
          [t("rep.col.time"), t("rep.col.device"), t("rep.col.command"), t("rep.col.values"), t("rep.col.ack"), t("rep.col.readback"), t("rep.col.feedback"), t("rep.col.user")],
          runs.map(({ result: r, feedback: f }) => [
            when(r.ts_ms),
            r.device_id,
            r.command_id ?? r.register_id ?? "—",
            `${n(r.old_value, 3)} → ${n(r.requested_value, 3)}`,
            r.ack ?? "—",
            `${r.readback ?? "—"}${r.readback_value !== null ? ` (${n(r.readback_value, 3)})` : ""}`,
            f ? `${f.readback ?? "—"}${f.readback_value !== null ? ` (${n(f.readback_value, 3)})` : ""}` : t("rep.fb.none"),
            `${r.user} (${r.role})`,
          ]),
          "rep-commands",
        )
      : h("p", { class: "muted" }, t("rep.none")),
    h("h2", null, t("rep.commissioning")),
    wizard.length
      ? table(
          [t("rep.col.time"), t("rep.col.device"), t("rep.col.event"), t("rep.col.result"), t("rep.col.text")],
          wizard.map((a) => [when(a.ts_ms), a.device_id, t(`audit.event.${a.event ?? "none"}`), a.readback ?? "—", a.note ?? ""]),
          "rep-commissioning",
        )
      : h("p", { class: "muted" }, t("rep.none")),
    h("h2", null, t("rep.events")),
    d.events.length
      ? table([t("rep.col.time"), t("rep.col.text")], d.events.slice(-200).map((e) => [when(e.ts_ms), e.text]))
      : h("p", { class: "muted" }, t("rep.none")),
  );
}

/** Self-contained HTML document of the report: inline print styles, no script, DOM-serialized text. */
export function reportDocument(report: HTMLElement): string {
  const doc = document.implementation.createHTMLDocument(t("rep.title"));
  doc.documentElement.lang = lang();
  const meta = doc.createElement("meta");
  meta.setAttribute("charset", "utf-8");
  const style = doc.createElement("style");
  style.textContent = REPORT_CSS;
  doc.head.append(meta, style);
  doc.body.append(doc.importNode(report, true));
  return `<!DOCTYPE html>\n${new XMLSerializer().serializeToString(doc.documentElement)}`;
}

const REPORT_CSS = `
body{font:13px/1.4 system-ui,"Segoe UI",Roboto,sans-serif;color:#1f1f1f;background:#fff;margin:1.5rem}
h1{font-size:1.4rem}h2{font-size:1.1rem;margin-top:1.4rem;border-bottom:1px solid #6b6b6b}
table{border-collapse:collapse;width:100%;margin:.4rem 0}th,td{border:1px solid #9a9a9a;padding:.2rem .4rem;text-align:left;vertical-align:top}
thead th{background:#ececec}.muted{color:#4d4d4d}.small{font-size:.85em}
.sim-banner{border:2px dashed #1f1f1f;padding:.3rem .6rem;font-weight:600}
dl.report-meta{display:grid;grid-template-columns:max-content 1fr;gap:.1rem 1rem}dd{margin:0}
@media print{body{margin:0}h2{break-after:avoid}tr{break-inside:avoid}}
`;

export function reportView(): View {
  const status = statusLine();
  const box = h("div", { class: "report-box" });
  const saveBtn = h("button", { type: "button", "data-testid": "report-save" }, t("rep.save_html"));
  const printBtn = h("button", { type: "button", class: "primary", "data-testid": "report-print" }, t("rep.print"));
  const reloadBtn = h("button", { type: "button" }, t("rep.reload"));
  const el = h(
    "div",
    { class: "view report-view" },
    h("header", { class: "view-head no-print" }, h("h2", null, t("nav.report")), h("div", { class: "actions" }, reloadBtn, saveBtn, printBtn)),
    h("p", { class: "muted small no-print" }, t("rep.help")),
    status.el,
    box,
  );
  if (!hasBackend()) {
    box.append(h("p", { class: "notice" }, t("ctl.no_backend")));
    return { el };
  }
  let report: HTMLElement | null = null;
  async function load() {
    try {
      const d = await api.reportData();
      clear(box);
      report = renderReport(d);
      box.append(report);
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }
  reloadBtn.addEventListener("click", () => void load());
  printBtn.addEventListener("click", () => window.print());
  saveBtn.addEventListener("click", async () => {
    if (!report) return;
    try {
      const r = await api.reportSaveHtml(reportDocument(report));
      status.set(r.saved && r.path ? t("export.saved", { path: r.path }) : t("export.not_saved"), r.saved ? "ok" : "info");
    } catch (e) {
      status.set(errorText(e), "error");
    }
  });
  void load();
  return { el };
}
