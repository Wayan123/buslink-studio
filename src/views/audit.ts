// Audit log (commissioning-control Req 4): every arm, preview, confirm, send, result, feedback,
// journal and verification row of the session, filterable, exportable as CSV/XLSX (Rust writes the files).
import { api, errorText, hasBackend } from "../api.ts";
import { clear, h } from "../dom.ts";
import { fmtLocalDateTime, fmtNumber, fmtUtc } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { filterAudit } from "../control.ts";
import type { AuditRow } from "../types.ts";
import { field, select, statusLine, type View } from "./view.ts";

export function auditView(): View {
  const status = statusLine();
  const tableBox = h("div", { class: "table-wrap" });
  const count = h("p", { class: "muted small", role: "status" });
  const deviceSel = select([["", t("audit.all")]]);
  const eventSel = select([["", t("audit.all")]]);
  const textIn = h("input", { type: "text", "data-testid": "audit-text" });
  let rows: AuditRow[] = [];

  const el = h(
    "div",
    { class: "view audit" },
    h("header", { class: "view-head" }, h("h2", null, t("audit.title")), h("div", { class: "actions" })),
    h("p", { class: "muted small" }, t("audit.help")),
    status.el,
  );
  if (!hasBackend()) {
    el.append(h("p", { class: "notice" }, t("ctl.no_backend")));
    return { el };
  }
  const actions = el.querySelector(".actions");
  const refreshBtn = h("button", { type: "button" }, t("audit.refresh"));
  refreshBtn.addEventListener("click", () => void load());
  actions?.append(refreshBtn);
  for (const fmt of ["csv", "xlsx"] as const) {
    const b = h("button", { type: "button", "data-testid": `audit-export-${fmt}` }, t(`audit.export.${fmt}`));
    b.addEventListener("click", async () => {
      try {
        const r = await api.auditExport(fmt);
        status.set(r.saved && r.path ? t("export.saved", { path: r.path }) : t("export.not_saved"), r.saved ? "ok" : "info");
      } catch (e) {
        status.set(errorText(e), "error");
      }
    });
    actions?.append(b);
  }
  el.append(
    h(
      "div",
      { class: "card form-grid compact" },
      field("audit-device", t("audit.device"), deviceSel).el,
      field("audit-event", t("audit.event"), eventSel).el,
      field("audit-text", t("audit.search"), textIn).el,
    ),
    count,
    tableBox,
  );
  for (const c of [deviceSel, eventSel]) c.addEventListener("change", render);
  textIn.addEventListener("input", render);

  function options(sel: HTMLSelectElement, values: string[], label: (v: string) => string) {
    const keep = sel.value;
    clear(sel);
    sel.append(h("option", { value: "" }, t("audit.all")), ...values.map((v) => h("option", { value: v }, label(v))));
    sel.value = values.includes(keep) ? keep : "";
  }

  function render() {
    const L = lang();
    const shown = filterAudit(rows, { device: deviceSel.value, event: eventSel.value, text: textIn.value });
    count.textContent = t("audit.count", { n: shown.length, total: rows.length });
    clear(tableBox);
    const head = ["time", "event", "user", "device", "register", "values", "frames", "result", "note"];
    const tbl = h("table", { class: "audit-table", "data-testid": "audit-table" }, h("thead", null, h("tr", null, ...head.map((k) => h("th", { scope: "col" }, t(`audit.col.${k}`))))));
    const tb = h("tbody");
    for (const r of [...shown].reverse()) {
      const fc = r.function !== null ? `FC${String(r.function).padStart(2, "0")}` : "";
      const values = [
        r.old_value !== null ? fmtNumber(r.old_value, L, 3) : null,
        r.requested_value !== null ? fmtNumber(r.requested_value, L, 3) : null,
      ];
      tb.append(
        h(
          "tr",
          { class: `audit-${r.event ?? "none"}` },
          h("td", { title: fmtUtc(r.ts_ms) }, fmtLocalDateTime(r.ts_ms, L), h("br"), h("span", { class: "muted small mono" }, fmtUtc(r.ts_ms))),
          h("td", null, t(`audit.event.${r.event ?? "none"}`)),
          h("td", null, `${r.user} (${r.role})`),
          h("td", null, r.device_id, h("br"), h("span", { class: "muted small" }, `${r.profile_id} ${r.profile_version} · unit ${r.unit_id ?? "—"}`)),
          h("td", null, [r.register_id ?? r.command_id ?? "—", r.address !== null ? ` · ${fc} @${r.address}` : ""].join("")),
          h("td", { class: "num" }, values[0] !== null || values[1] !== null ? `${values[0] ?? "—"} → ${values[1] ?? "—"}` : "—"),
          h(
            "td",
            { class: "mono small" },
            r.request_hex ? h("div", null, `TX ${r.request_hex}`) : null,
            r.response_hex ? h("div", null, `RX ${r.response_hex}`) : null,
          ),
          h(
            "td",
            null,
            [r.ack, r.exception !== null ? `EXC ${r.exception.toString(16).padStart(2, "0").toUpperCase()}` : null, r.readback ? `${t("audit.readback")} ${r.readback}` : null, r.readback_value !== null ? fmtNumber(r.readback_value, L, 3) : null]
              .filter(Boolean)
              .join(" · ") || "—",
          ),
          h("td", { class: "small" }, r.note ?? ""),
        ),
      );
    }
    tbl.append(tb);
    tableBox.append(tbl);
  }

  async function load() {
    try {
      rows = await api.auditList();
      options(deviceSel, [...new Set(rows.map((r) => r.device_id).filter(Boolean))], (v) => v);
      options(eventSel, [...new Set(rows.map((r) => r.event ?? "").filter(Boolean))], (v) => t(`audit.event.${v}`));
      render();
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }
  void load();
  return { el };
}
