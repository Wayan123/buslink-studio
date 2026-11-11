// Alarms (dashboard-ui Req 6, ISA-18.2): banner, separate process and communication lists,
// acknowledge / shelve / out of service, add alarm with research/02 default deadbands, alarm log.
import { api, errorText } from "../api.ts";
import { alarmStateBadge, priorityBadge } from "../components/badges.ts";
import { clear, h } from "../dom.ts";
import { fmtLocalDateTime, fmtLocalTime, fmtNumber } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { loadProfiles, profileOf } from "../profiles.ts";
import { store } from "../store.ts";
import type { AlarmKind, AlarmView } from "../types.ts";
import { field, preserveFocus, select, statusLine, type View } from "./view.ts";

/** Header banner: count per priority, oldest unacknowledged, flood indicator. */
export function alarmBanner(): HTMLElement {
  const el = h("a", { class: "alarm-banner", href: "#/alarms", "aria-label": t("alarm.banner.label") });
  const render = () => {
    const s = store.alarms.summary;
    clear(el);
    for (const p of [1, 2, 3] as const) {
      el.append(h("span", { class: `banner-count${s.counts[p - 1] ? " has" : ""}` }, priorityBadge(p, false), String(s.counts[p - 1])));
    }
    if (s.comm_active) el.append(h("span", { class: "badge q-bad" }, t("alarm.banner.comm", { n: s.comm_active })));
    if (s.oldest_unacked) {
      const a = store.alarms.alarms.find((x) => x.id === s.oldest_unacked);
      el.append(h("span", { class: "banner-oldest" }, t("alarm.banner.oldest", { text: a ? alarmTitle(a) : s.oldest_unacked })));
    }
    if (s.flood) el.append(h("span", { class: "badge flood" }, t("alarm.flood", { n: s.activations_10min })));
    el.classList.toggle("quiet", s.counts.every((n) => n === 0) && !s.comm_active);
  };
  store.subscribe((c) => {
    if (c.alarms) render();
  });
  render();
  return el;
}

export function alarmTitle(a: AlarmView): string {
  const d = store.device(a.device_id)?.device;
  const dev = d?.name ?? a.device_id;
  if (a.class === "comm") return `${dev}: ${a.message ?? t("alarm.comm")}`;
  const reg = d && a.tag_id ? profileOf(d)?.registers.find((r) => r.id === a.tag_id) : undefined;
  return `${dev}: ${reg ? tx(reg.name) : a.tag_id} ${a.kind ? t(`alarm.kind.${a.kind}`) : ""}`;
}

type Filter = "all" | "active" | "unacked";
type Sort = "priority" | "time";

export function alarmsView(): View {
  const status = statusLine();
  const processBox = h("div", { class: "table-wrap" });
  const commBox = h("div", { class: "table-wrap" });
  const logBox = h("div", { class: "table-wrap" });
  let filter: Filter = "all";
  let sort: Sort = "priority";
  const filterSel = select([["all", t("alarm.filter.all")], ["active", t("alarm.filter.active")], ["unacked", t("alarm.filter.unacked")]], "all");
  filterSel.id = "alarm-filter";
  filterSel.addEventListener("change", () => {
    filter = filterSel.value as Filter;
    renderLists();
  });
  const sortSel = select([["priority", t("alarm.sort.priority")], ["time", t("alarm.sort.time")]], "priority");
  sortSel.id = "alarm-sort";
  sortSel.addEventListener("change", () => {
    sort = sortSel.value as Sort;
    renderLists();
  });

  const act = (a: AlarmView, label: string, fn: () => Promise<void>, cls = "") => {
    const b = h("button", { type: "button", class: cls, "data-fk": `${a.id}:${label}` }, label);
    b.addEventListener("click", async () => {
      b.disabled = true;
      try {
        await fn();
        status.set(t("alarm.done", { action: label }), "ok");
      } catch (e) {
        status.set(errorText(e), "error");
      } finally {
        b.disabled = false;
      }
    });
    return b;
  };

  function actions(a: AlarmView): HTMLElement {
    const box = h("div", { class: "actions" });
    if (a.state === "UNACKED" || a.state === "RTN_UNACKED") box.append(act(a, t("alarm.ack"), () => api.alarmAck(a.id), "primary"));
    if (a.shelved_until_ms) box.append(act(a, t("alarm.unshelve"), () => api.alarmUnshelve(a.id)));
    else {
      const minutes = select([["15", "15 min"], ["60", "1 h"], ["480", "8 h"]], "15");
      minutes.setAttribute("aria-label", t("alarm.shelve_for"));
      minutes.dataset.fk = `${a.id}:shelve-min`;
      box.append(minutes, act(a, t("alarm.shelve"), () => api.alarmShelve(a.id, Number(minutes.value))));
    }
    box.append(
      act(a, a.out_of_service ? t("alarm.in_service") : t("alarm.oos"), () => api.alarmSetOos(a.id, !a.out_of_service)),
    );
    if (a.class === "process") box.append(h("span", { class: "spacer" }), act(a, t("alarm.remove"), () => api.alarmRemove(a.id), "danger"));
    return box;
  }

  function table(list: AlarmView[], comm: boolean): HTMLElement {
    const L = lang();
    if (!list.length) return h("p", { class: "muted" }, comm ? t("alarm.none.comm") : t("alarm.none.process"));
    const cols = ["alarm.col.priority", "alarm.col.state", "alarm.col.alarm", comm ? "alarm.col.message" : "alarm.col.limit", "alarm.col.value", "alarm.col.since", "alarm.col.actions"];
    return h(
      "table",
      { class: "alarm-table" },
      h("caption", { class: "sr-only" }, comm ? t("alarm.comm_list") : t("alarm.process_list")),
      h("thead", null, h("tr", null, ...cols.map((k) => h("th", { scope: "col" }, t(k))))),
      h(
        "tbody",
        null,
        list.map((a) =>
          h(
            "tr",
            { class: `arow astate-${a.state.toLowerCase()} p${a.priority}` },
            h("td", null, priorityBadge(a.priority)),
            h(
              "td",
              null,
              alarmStateBadge(a.state),
              a.held ? h("span", { class: "muted small" }, ` ${t("alarm.held")}`) : null,
              a.latch ? h("span", { class: "muted small" }, ` ${t("alarm.latched")}`) : null,
              a.chattering ? h("span", { class: "badge chatter" }, t("alarm.chattering")) : null,
              a.shelved_until_ms ? h("span", { class: "muted small" }, ` ${t("alarm.until", { time: fmtLocalTime(a.shelved_until_ms, L) })}`) : null,
            ),
            h("th", { scope: "row" }, alarmTitle(a), !comm && a.message ? h("br") : null, !comm && a.message ? h("span", { class: "muted small" }, a.message) : null),
            h(
              "td",
              null,
              comm
                ? (a.message ?? "")
                : `${a.kind ? t(`alarm.kind.${a.kind}`) : ""} ${fmtNumber(a.limit, L)} · ${t("alarm.deadband_short", { db: fmtNumber(a.deadband, L) })} · ${t("alarm.delay_short", { on: a.on_delay_ms / 1000, off: a.off_delay_ms / 1000 })}`,
            ),
            h("td", { class: "num" }, comm ? "—" : fmtNumber(a.value, L)),
            h("td", null, a.activated_ms ? fmtLocalTime(a.activated_ms, L) : "—"),
            h("td", null, actions(a)),
          ),
        ),
      ),
    );
  }

  function pick(list: AlarmView[]): AlarmView[] {
    const f = list.filter((a) =>
      filter === "all" ? true : filter === "active" ? a.active : a.state === "UNACKED" || a.state === "RTN_UNACKED",
    );
    return f.sort((a, b) =>
      sort === "priority"
        ? a.priority - b.priority || (b.activated_ms ?? 0) - (a.activated_ms ?? 0)
        : (b.activated_ms ?? b.last_change_ms) - (a.activated_ms ?? a.last_change_ms),
    );
  }

  function renderLists() {
    preserveFocus(processBox, () => {
      clear(processBox);
      processBox.append(table(pick(store.alarms.alarms.filter((a) => a.class === "process")), false));
    });
    preserveFocus(commBox, () => {
      clear(commBox);
      commBox.append(table(pick(store.alarms.alarms.filter((a) => a.class === "comm")), true));
    });
  }

  async function renderLog() {
    try {
      const rows = (await api.alarmEvents()).slice(-200).reverse();
      const L = lang();
      clear(logBox);
      logBox.append(
        rows.length
          ? h(
              "table",
              { class: "log-table" },
              h("caption", { class: "sr-only" }, t("alarm.log")),
              h("thead", null, h("tr", null, ...["alarm.log.time", "alarm.log.severity", "alarm.log.text"].map((k) => h("th", { scope: "col" }, t(k))))),
              h(
                "tbody",
                null,
                rows.map((r) => h("tr", null, h("td", null, fmtLocalDateTime(r.ts_ms, L)), h("td", null, r.severity), h("td", null, r.text))),
              ),
            )
          : h("p", { class: "muted" }, t("alarm.log.empty")),
      );
    } catch (e) {
      clear(logBox);
      logBox.append(h("p", { class: "error-text" }, errorText(e)));
    }
  }

  // --- add alarm ---
  const devSel = h("select");
  const tagSel = h("select");
  const kindSel = select([["HI", t("alarm.kind.HI")], ["HI_HI", t("alarm.kind.HI_HI")], ["LO", t("alarm.kind.LO")], ["LO_LO", t("alarm.kind.LO_LO")]], "HI");
  const limitIn = h("input", { type: "number", step: "any", required: true });
  const dbIn = h("input", { type: "number", step: "any", min: 0, value: 0 });
  const onIn = h("input", { type: "number", min: 0, max: 3600, value: 15 });
  const offIn = h("input", { type: "number", min: 0, max: 3600, value: 15 });
  const prioSel = select([["1", t("alarm.priority.1")], ["2", t("alarm.priority.2")], ["3", t("alarm.priority.3")]], "3");
  const latchIn = h("input", { type: "checkbox" });
  const msgIn = h("input", { type: "text", maxlength: 120 });
  const fLimit = field("al-limit", t("alarm.form.limit"), limitIn);
  const fDb = field("al-db", t("alarm.form.deadband"), dbIn, t("alarm.form.deadband.hint"));
  const fOn = field("al-on", t("alarm.form.on_delay"), onIn);
  const fOff = field("al-off", t("alarm.form.off_delay"), offIn);
  const fTag = field("al-tag", t("alarm.form.tag"), tagSel);

  function fillDevices() {
    clear(devSel);
    for (const { device } of store.devices()) devSel.append(h("option", { value: device.id }, device.name));
    fillTags();
  }
  function fillTags() {
    clear(tagSel);
    const d = store.device(devSel.value)?.device;
    const regs = d ? (profileOf(d)?.registers ?? []).filter((r) => r.type !== "string" && r.type !== "bool" && r.type !== "bitfield") : [];
    for (const r of regs) tagSel.append(h("option", { value: r.id }, `${tx(r.name)}${r.unit ? ` (${r.unit})` : ""}`));
    void suggest();
  }
  async function suggest() {
    const d = store.device(devSel.value)?.device;
    const r = d ? profileOf(d)?.registers.find((x) => x.id === tagSel.value) : undefined;
    if (!r) return;
    const span = r.valid_range ? r.valid_range[1] - r.valid_range[0] : 100;
    try {
      const def = await api.alarmDefaults(r.unit ?? null, span);
      dbIn.value = String(Math.round(def.deadband * 1000) / 1000);
      onIn.value = String(def.delay_ms / 1000);
      offIn.value = String(def.delay_ms / 1000);
    } catch {
      /* keep the typed values */
    }
  }
  devSel.addEventListener("change", fillTags);
  tagSel.addEventListener("change", () => void suggest());

  const form = h(
    "form",
    { class: "card form-grid", "aria-labelledby": "al-add-h" },
    h("h3", { id: "al-add-h" }, t("alarm.form.title")),
    field("al-dev", t("alarm.form.device"), devSel).el,
    fTag.el,
    field("al-kind", t("alarm.form.kind"), kindSel).el,
    fLimit.el,
    fDb.el,
    fOn.el,
    fOff.el,
    field("al-prio", t("alarm.form.priority"), prioSel).el,
    h("div", { class: "field" }, h("label", { class: "check" }, latchIn, ` ${t("alarm.form.latch")}`)),
    field("al-msg", t("alarm.form.message"), msgIn).el,
    h("p", { class: "muted small" }, t("alarm.form.defaults_note")),
    h("button", { type: "submit", class: "primary" }, t("alarm.form.add")),
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    for (const f of [fLimit, fDb, fOn, fOff, fTag]) f.error(null);
    const limit = Number(limitIn.value);
    const deadband = Number(dbIn.value);
    const on = Number(onIn.value);
    const off = Number(offIn.value);
    let bad = false;
    if (!tagSel.value) {
      fTag.error(t("alarm.err.tag"));
      bad = true;
    }
    if (limitIn.value.trim() === "" || !Number.isFinite(limit)) {
      fLimit.error(t("alarm.err.limit"));
      bad = true;
    }
    if (!Number.isFinite(deadband) || deadband < 0) {
      fDb.error(t("alarm.err.deadband"));
      bad = true;
    }
    for (const [f, v] of [[fOn, on], [fOff, off]] as const) {
      if (!Number.isFinite(v) || v < 0 || v > 3600) {
        f.error(t("alarm.err.delay"));
        bad = true;
      }
    }
    if (bad) return;
    const kind = kindSel.value as AlarmKind;
    try {
      await api.alarmAdd({
        id: `${devSel.value}.${tagSel.value}.${kind.toLowerCase()}.${Date.now().toString(36)}`,
        device_id: devSel.value,
        tag_id: tagSel.value,
        kind,
        limit,
        deadband,
        on_delay_ms: Math.round(on * 1000),
        off_delay_ms: Math.round(off * 1000),
        priority: Number(prioSel.value),
        latch: latchIn.checked,
        message: msgIn.value.trim() || null,
      });
      status.set(t("alarm.added"), "ok");
      limitIn.value = "";
    } catch (err) {
      fLimit.error(errorText(err));
    }
  });

  const unsub = store.subscribe((c) => {
    if (c.alarms) {
      renderLists();
      void renderLog();
    }
    if (c.links && devSel.options.length !== store.devices().length) fillDevices();
  });

  const el = h(
    "div",
    { class: "view alarms" },
    h("header", { class: "view-head" }, h("h2", null, t("nav.alarms")), h("div", { class: "actions" }, h("label", { for: "alarm-filter" }, t("alarm.filter")), filterSel, h("label", { for: "alarm-sort" }, t("alarm.sort")), sortSel)),
    h("p", { class: "muted small" }, t("alarm.intro")),
    status.el,
    h("section", { "aria-labelledby": "al-proc-h" }, h("h3", { id: "al-proc-h" }, t("alarm.process_list")), processBox),
    h("section", { "aria-labelledby": "al-comm-h" }, h("h3", { id: "al-comm-h" }, t("alarm.comm_list")), h("p", { class: "muted small" }, t("alarm.comm_note")), commBox),
    form,
    h("section", { "aria-labelledby": "al-log-h" }, h("h3", { id: "al-log-h" }, t("alarm.log")), logBox),
  );
  void loadProfiles().then(() => {
    fillDevices();
    renderLists();
  });
  void renderLog();
  return { el, destroy: unsub };
}
