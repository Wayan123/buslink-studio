// Live dashboard (dashboard-ui Req 3): L1 overview of all devices (worst state first) and the
// L2 device page (route #/device/<id>). Tiles update in place; ages tick once per second.
import { api, errorText } from "../api.ts";
import { linkStateBadge, priorityBadge, qualityBadge, simulatedBanner } from "../components/badges.ts";
import { Tile } from "../components/tile.ts";
import { append, clear, h, setText } from "../dom.ts";
import { fmtAge } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { loadProfiles, profileOf } from "../profiles.ts";
import { displayQuality } from "../quality.ts";
import { store } from "../store.ts";
import type { DeviceView, ExportFormat, LinkView, QualityCode } from "../types.ts";
import { preserveFocus, statusLine, type View } from "./view.ts";

interface Card {
  el: HTMLElement;
  head: HTMLElement;
  tiles: Map<string, Tile>;
  rank: number;
}

/** Lower = worse: link down/disconnected 0, P1 1, P2 2, P3 3, bad quality 4, normal 5. */
export function deviceRank(linkState: string, alarmPriorities: number[], qualities: QualityCode[]): number {
  if (linkState === "down" || linkState === "disconnected") return 0;
  const p = alarmPriorities.length ? Math.min(...alarmPriorities) : 0;
  if (p >= 1 && p <= 3) return p;
  if (qualities.some((q) => q !== "GOOD")) return 4;
  return 5;
}

export function exportBar(): HTMLElement {
  const status = statusLine();
  const btn = (fmt: ExportFormat) => {
    const b = h("button", { type: "button", "data-testid": `export-${fmt}` }, t(`export.${fmt}`));
    b.addEventListener("click", async () => {
      b.disabled = true;
      status.set(t("export.working"));
      try {
        const r = await api.exportSession(fmt);
        if (r.saved && r.path) status.set(t("export.saved", { path: r.path }), "ok");
        else status.set(t("export.not_saved"), "info");
      } catch (e) {
        status.set(errorText(e), "error");
      } finally {
        b.disabled = false;
      }
    });
    return b;
  };
  return h(
    "section",
    { class: "card export-bar", "aria-labelledby": "export-h" },
    h("h3", { id: "export-h" }, t("export.title")),
    h("p", { class: "muted small" }, t("export.hint")),
    h("div", { class: "actions" }, btn("csv"), btn("xlsx"), btn("json")),
    status.el,
  );
}

export function dashboardView(params: { device?: string } = {}): View {
  const cardsBox = h("div", { class: "device-list" });
  const cards = new Map<string, Card>();
  let paused = false;
  const pauseBanner = h("p", { class: "paused-banner", role: "status", hidden: true }, t("dash.paused"));
  const pauseBtn = h("button", { type: "button", "aria-pressed": "false" }, t("dash.pause"));
  pauseBtn.addEventListener("click", () => {
    paused = !paused;
    pauseBtn.setAttribute("aria-pressed", String(paused));
    pauseBtn.textContent = paused ? t("dash.resume") : t("dash.pause");
    pauseBanner.hidden = !paused;
    if (!paused) renderAll();
  });
  const densityBtn = h("button", { type: "button", "aria-pressed": "false" }, t("dash.compact"));
  densityBtn.addEventListener("click", () => {
    const on = !cardsBox.classList.contains("compact");
    cardsBox.classList.toggle("compact", on);
    densityBtn.setAttribute("aria-pressed", String(on));
  });
  const empty = h(
    "div",
    { class: "card empty" },
    h("p", null, t("dash.empty")),
    h("p", null, h("a", { href: "#/connect", class: "button primary" }, t("dash.empty.action"))),
  );

  function selected(): { device: DeviceView; link: LinkView }[] {
    const all = store.devices();
    return params.device ? all.filter((d) => d.device.id === params.device) : all;
  }

  function cardFor(device: DeviceView, link: LinkView): Card | null {
    const existing = cards.get(device.id);
    if (existing) return existing;
    const profile = profileOf(device);
    if (!profile) return null;
    const head = h("header", { class: "card-head device-head" });
    const grid = h("div", { class: "tile-grid" });
    const tiles = new Map<string, Tile>();
    for (const reg of profile.registers) {
      const tile = new Tile(reg, device.id);
      tiles.set(reg.id, tile);
      grid.append(tile.el);
    }
    const el = h(
      "section",
      { class: "card device-card", "data-device": device.id, "aria-label": device.name },
      head,
      link.simulated || profile.simulated_only ? simulatedBanner() : null,
      grid,
    );
    const c: Card = { el, head, tiles, rank: 5 };
    cards.set(device.id, c);
    return c;
  }

  function renderHead(c: Card, device: DeviceView, link: LinkView, now: number) {
    const profile = profileOf(device);
    const comm = store.commAlarm(device.id);
    preserveFocus(c.head, () => {
    clear(c.head);
    const L = lang();
    append(
      c.head,
      h("h3", null, params.device ? device.name : h("a", { href: `#/device/${encodeURIComponent(device.id)}`, "data-fk": "device" }, device.name)),
      h(
        "p",
        { class: "muted small" },
        `${profile ? `${tx(profile.vendor)} ${tx(profile.model)} · ` : ""}${t("device.unit")} ${device.unit_id} · ${link.label}`,
      ),
      h("span", { class: "head-label" }, t("dash.link")),
      linkStateBadge(link.state),
      comm && comm.active ? h("span", { class: "badge q-bad" }, t("dash.comm_alarm")) : null,
      h(
        "span",
        { class: "muted small" },
        t("dash.poll", {
          set: fmtAge(device.poll_interval_ms, L),
          actual: device.counters.cycle_ms === null ? "—" : fmtAge(device.counters.cycle_ms, L),
        }),
      ),
      h(
        "nav",
        { class: "head-links", "aria-label": t("dash.device_links") },
        h("a", { href: `#/trends/${encodeURIComponent(device.id)}`, "data-fk": "trends" }, t("nav.trends")),
        h("a", { href: `#/inspector/${encodeURIComponent(device.id)}`, "data-fk": "inspector" }, t("nav.inspector")),
        // Control panel only for simulated devices on a simulator link (safety rule 8).
        link.simulated && profile?.simulated_only
          ? h("a", { href: `#/control/${encodeURIComponent(device.id)}`, "data-fk": "control", "data-testid": `control-${device.id}` }, t("nav.control"))
          : null,
        profile?.capabilities?.comm_config && profile.capabilities.comm_config !== "unsupported"
          ? h("a", { href: `#/commission/${encodeURIComponent(device.id)}`, "data-fk": "commission" }, t("nav.commission"))
          : null,
      ),
    );
    });
    void now;
  }

  function updateCard(device: DeviceView, link: LinkView, changed: Set<string> | null, now: number) {
    const c = cardFor(device, link);
    if (!c) return;
    if (changed === null) renderHead(c, device, link, now);
    c.el.classList.toggle("disconnected", link.state === "down" || link.state === "disconnected");
    const qualities: QualityCode[] = [];
    const prios: number[] = [];
    for (const [tagId, tile] of c.tiles) {
      const key = `${device.id}/${tagId}`;
      const tag = store.tags.get(key);
      const alarms = store.alarmsOfTag(device.id, tagId);
      if (tag) qualities.push(displayQuality(tag, device.stale_after_ms, now));
      for (const a of alarms) if (a.active || a.state === "UNACKED") prios.push(a.priority);
      if (changed === null || changed.has(key)) {
        tile.update(tag, { staleAfterMs: device.stale_after_ms, cycleMs: device.counters.cycle_ms, alarms, now });
      }
    }
    c.rank = deviceRank(link.state, prios, qualities);
  }

  function order() {
    const list = selected()
      .map(({ device }) => cards.get(device.id))
      .filter((c): c is Card => c !== undefined)
      .sort((a, b) => a.rank - b.rank);
    list.forEach((c, i) => {
      if (cardsBox.children[i] !== c.el) cardsBox.insertBefore(c.el, cardsBox.children[i] ?? null);
    });
    // Drop cards of removed devices.
    const ids = new Set(selected().map((d) => d.device.id));
    for (const [id, c] of cards) {
      if (!ids.has(id)) {
        c.el.remove();
        cards.delete(id);
      }
    }
  }

  function renderAll() {
    const now = Date.now();
    const sel = selected();
    empty.hidden = sel.length > 0;
    for (const { device, link } of sel) updateCard(device, link, null, now);
    order();
  }

  const unsub = store.subscribe((c) => {
    if (paused) return;
    const now = Date.now();
    if (c.links || c.alarms) {
      renderAll();
      return;
    }
    if (c.tags.size === 0) return;
    for (const { device, link } of selected()) updateCard(device, link, c.tags, now);
    order();
  });

  const timer = window.setInterval(() => {
    if (paused) return;
    const now = Date.now();
    for (const c of cards.values()) for (const tile of c.tiles.values()) tile.tick(now);
  }, 1000);

  const title = params.device ? store.device(params.device)?.device.name ?? params.device : t("nav.overview");
  const summary = h("p", { class: "summary", role: "status" });
  function renderSummary() {
    const s = store.alarms.summary;
    clear(summary);
    append(
      summary,
      t("dash.summary", { devices: selected().length }),
      " ",
      ...([1, 2, 3] as const).map((p) => h("span", { class: "summary-prio" }, priorityBadge(p, false), ` ${s.counts[p - 1]}`)),
      s.comm_active ? h("span", { class: "summary-comm" }, qualityBadge("TIMEOUT", null, String(s.comm_active)) ?? "") : null,
    );
  }
  const unsubSummary = store.subscribe((c) => {
    if (c.alarms || c.links) renderSummary();
  });

  const el = h(
    "div",
    { class: "view dashboard" },
    h(
      "header",
      { class: "view-head" },
      h("h2", null, params.device ? h("span", null, h("a", { href: "#/dashboard" }, t("nav.overview")), " › ", title) : title),
      h("div", { class: "actions" }, pauseBtn, densityBtn),
    ),
    pauseBanner,
    summary,
    empty,
    cardsBox,
    exportBar(),
  );
  void loadProfiles()
    .then(() => {
      renderAll();
      renderSummary();
    })
    .catch((e) => setText(summary, errorText(e)));
  return {
    el,
    destroy() {
      unsub();
      unsubSummary();
      window.clearInterval(timer);
    },
  };
}
