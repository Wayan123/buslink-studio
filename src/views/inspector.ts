// Register inspector (dashboard-ui Req 5, L4): doc address and PDU offset, FC, raw hex,
// decoded value in the profile order and all four orders side by side, quality, exception hint,
// last frames and link counters.
import { api, errorText } from "../api.ts";
import { qualityBadge } from "../components/badges.ts";
import { clear, h } from "../dom.ts";
import { fmtAge, fmtLocalTime, fmtNumber } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { exceptionHintKey, hex2 } from "../quality.ts";
import { store } from "../store.ts";
import type { DeviceInspection } from "../types.ts";
import { select, statusLine, type View } from "./view.ts";

const ORDERS = ["ABCD", "CDAB", "BADC", "DCBA"];

export function inspectorView(params: { device?: string } = {}): View {
  let deviceId = params.device ?? store.devices()[0]?.device.id ?? "";
  const status = statusLine();
  const body = h("div", { class: "inspector-body" });
  const deviceSel = select(store.devices().map(({ device }) => [device.id, device.name]), deviceId);
  deviceSel.id = "insp-device";
  deviceSel.addEventListener("change", () => {
    deviceId = deviceSel.value;
    location.hash = `#/inspector/${encodeURIComponent(deviceId)}`;
  });
  let paused = false;
  const pauseBtn = h("button", { type: "button", "aria-pressed": "false" }, t("dash.pause"));
  pauseBtn.addEventListener("click", () => {
    paused = !paused;
    pauseBtn.setAttribute("aria-pressed", String(paused));
    pauseBtn.textContent = paused ? t("dash.resume") : t("dash.pause");
  });

  function render(d: DeviceInspection) {
    const L = lang();
    clear(body);
    const c = d.device.counters;
    body.append(
      h(
        "dl",
        { class: "card counters-dl" },
        ...(
          [
            ["device.unit_id", String(d.device.unit_id)],
            ["inspector.profile", `${d.device.profile_id}@${d.device.profile_version}`],
            ["counter.requests", fmtNumber(c.requests, L)],
            ["counter.good", fmtNumber(c.good, L)],
            ["counter.timeouts", fmtNumber(c.timeouts, L)],
            ["counter.crc", fmtNumber(c.crc_errors, L)],
            ["counter.exceptions", fmtNumber(c.exceptions, L)],
            ["counter.decode", fmtNumber(c.decode_errors, L)],
            ["counter.cycle", c.cycle_ms === null ? "—" : fmtAge(c.cycle_ms, L)],
            ["counter.last_error", c.last_error ?? "—"],
          ] as [string, string][]
        ).flatMap(([k, v]) => [h("dt", null, t(k)), h("dd", null, v)]),
      ),
    );
    const head = [
      "inspector.tag",
      "inspector.table",
      "inspector.doc_address",
      "inspector.pdu_offset",
      "inspector.fc",
      "inspector.type",
      "inspector.raw",
      "inspector.decoded",
      ...ORDERS.map((o) => `order.${o}`),
      "inspector.quality",
      "inspector.exception",
    ];
    const rows = d.rows.map((r) => {
      const hint = exceptionHintKey(r.exception);
      const orders = ORDERS.map((o) => {
        const ov = r.orders.find((x) => x.order === o);
        if (!ov) return h("td", { class: "muted" }, "—");
        return h(
          "td",
          { class: ov.is_profile ? "order-profile mono" : "mono" },
          ov.value ?? (ov.error ? t("inspector.decode_failed") : "—"),
          ov.is_profile ? h("span", { class: "tag-profile" }, ` ${t("inspector.profile_order")}`) : null,
        );
      });
      const age = r.value_ts_ms === null ? null : Date.now() - r.value_ts_ms;
      return h(
        "tr",
        null,
        h("th", { scope: "row" }, tx(r.name), h("br"), h("span", { class: "muted small mono" }, r.tag_id)),
        h("td", null, t(`table.${r.table}`)),
        h("td", { class: "mono" }, r.doc_address),
        h("td", { class: "mono" }, String(r.pdu_offset)),
        h("td", { class: "mono", title: r.function_name }, `FC${hex2(r.function)}`),
        h("td", { class: "mono" }, `${r.type_name}${r.word_order ? ` ${r.word_order}` : ""}`),
        h("td", { class: "mono" }, r.raw_hex ?? "—"),
        h("td", { class: "mono" }, r.decoded ?? "—", r.unit ? ` ${r.unit}` : ""),
        ...orders,
        h("td", null, r.quality ? (qualityBadge(r.quality, r.exception, age === null ? undefined : fmtAge(age, L)) ?? t("quality.GOOD")) : t("tile.waiting")),
        h(
          "td",
          { class: "small" },
          r.exception !== null ? `${hex2(r.exception)} ${r.exception_name ?? ""}` : "",
          hint ? h("br") : null,
          hint ? h("span", { class: "muted" }, t(hint)) : null,
          r.error && r.exception === null && r.quality !== "GOOD" ? r.error : null,
        ),
      );
    });
    body.append(
      h(
        "div",
        { class: "table-wrap card" },
        h(
          "table",
          { class: "inspector-table" },
          h("caption", null, t("inspector.caption")),
          h("thead", null, h("tr", null, ...head.map((k) => h("th", { scope: "col" }, t(k))))),
          h("tbody", null, rows),
        ),
      ),
      h("p", { class: "muted small" }, t("inspector.address_note")),
      h("h3", null, t("inspector.frames")),
      h(
        "div",
        { class: "table-wrap card" },
        h(
          "table",
          { class: "frames-table" },
          h("caption", { class: "sr-only" }, t("inspector.frames")),
          h(
            "thead",
            null,
            h("tr", null, ...["inspector.time", "inspector.outcome", "inspector.tx", "inspector.rx", "inspector.elapsed"].map((k) => h("th", { scope: "col" }, t(k)))),
          ),
          h(
            "tbody",
            null,
            d.frames.map((f) =>
              h(
                "tr",
                { class: f.outcome === "OK" ? "" : "frame-bad" },
                h("td", null, fmtLocalTime(f.ts_ms, L)),
                h("td", null, f.outcome === "OK" ? "OK" : h("span", { class: "badge q-bad" }, f.outcome)),
                h("td", { class: "mono" }, f.request_hex),
                h("td", { class: "mono" }, f.response_hex || "—"),
                h("td", { class: "num" }, `${f.elapsed_ms} ms`),
              ),
            ),
          ),
        ),
      ),
    );
  }

  async function refresh() {
    if (!deviceId || paused) return;
    try {
      render(await api.inspect(deviceId));
      status.set("");
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }

  const timer = window.setInterval(() => void refresh(), 1000);
  const el = h(
    "div",
    { class: "view inspector" },
    h(
      "header",
      { class: "view-head" },
      h("h2", null, t("nav.inspector")),
      h("div", { class: "actions" }, h("label", { for: "insp-device" }, t("inspector.device")), deviceSel, pauseBtn),
    ),
    status.el,
    deviceId ? body : h("p", { class: "muted" }, t("inspector.none")),
  );
  void refresh();
  return { el, destroy: () => window.clearInterval(timer) };
}
