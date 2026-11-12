// ISA-101 tag tile: name, value + unit, quality label, age, analog bar with normal band and limit ticks.
// Updated in place; never rebuilt per frame.
import { barPct as pct, barRange, outOfScale, type Range } from "../barscale.ts";
import { clear, h, setText } from "../dom.ts";
import { decimalsFor, fmtAge, fmtNumber } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { displayQuality, qualityInfo } from "../quality.ts";
import type { AlarmView, Register, TagView } from "../types.ts";
import { priorityBadge, qualityBadge } from "./badges.ts";

export { barRange };
export type { Range };

export interface TileContext {
  staleAfterMs: number;
  cycleMs: number | null;
  alarms: AlarmView[];
  now: number;
}

/** Normal band between the innermost LO and HI limits. */
export function normalBand(alarms: AlarmView[], r: Range): Range | null {
  const his = alarms.filter((a) => a.kind === "HI" || a.kind === "HI_HI").map((a) => a.limit ?? Infinity);
  const los = alarms.filter((a) => a.kind === "LO" || a.kind === "LO_LO").map((a) => a.limit ?? -Infinity);
  if (his.length === 0 && los.length === 0) return null;
  return { min: los.length ? Math.max(...los) : r.min, max: his.length ? Math.min(...his) : r.max };
}

export class Tile {
  el: HTMLElement;
  private valueEl: HTMLElement;
  private unitEl: HTMLElement;
  private badgeSlot: HTMLElement;
  private alarmSlot: HTMLElement;
  private ageEl: HTMLElement;
  private bar: HTMLElement;
  private reg: Register;
  private last: TagView | null = null;
  private ctx: TileContext | null = null;
  private shown: string | null = null;

  constructor(reg: Register, deviceId: string) {
    this.reg = reg;
    this.valueEl = h("span", { class: "tile-value" }, "—");
    this.unitEl = h("span", { class: "tile-unit" }, reg.unit ?? "");
    this.badgeSlot = h("span", { class: "tile-badge" });
    this.alarmSlot = h("span", { class: "tile-alarm" });
    this.ageEl = h("span", { class: "tile-age" });
    this.bar = h("div", { class: "bar", role: "img" });
    this.el = h(
      "article",
      { class: "tile q-none", "data-tag": `${deviceId}/${reg.id}` },
      h("header", { class: "tile-head" }, h("h4", { class: "tile-name" }, tx(reg.name)), this.alarmSlot, this.badgeSlot),
      h("p", { class: "tile-reading" }, this.valueEl, " ", this.unitEl),
      this.bar,
      h("p", { class: "tile-meta" }, this.ageEl),
    );
  }

  update(tag: TagView | undefined, ctx: TileContext): void {
    this.last = tag ?? null;
    this.ctx = ctx;
    const L = lang();
    const q = tag ? displayQuality(tag, ctx.staleAfterMs, ctx.now) : null;
    const info = q ? qualityInfo(q, tag?.exception ?? null) : null;
    this.shown = q;
    this.el.className = `tile ${info ? info.cls : "q-none"} cue-${info?.cue ?? "none"}`;
    // Value: the last good value (dimmed/struck by CSS when not current); never 0 for "no data".
    let valueText: string;
    if (!tag || (tag.value === null && tag.text === null)) {
      valueText = "—";
    } else if (tag.text !== null && tag.value === null) {
      valueText = tag.text;
    } else {
      valueText = fmtNumber(tag.value, L, decimalsFor(this.reg.scale, this.reg.decimals));
    }
    setText(this.valueEl, valueText);
    clear(this.badgeSlot);
    if (!tag) {
      this.badgeSlot.append(h("span", { class: "badge q-stale" }, t("tile.waiting")));
    } else if (q) {
      const b = qualityBadge(q, tag.exception, q === "GOOD" ? undefined : fmtAge(this.ageMs(), L));
      if (b) this.badgeSlot.append(b);
    }
    clear(this.alarmSlot);
    const active = ctx.alarms.filter((a) => a.active || a.state === "UNACKED").sort((a, b) => a.priority - b.priority);
    const top = active[0];
    if (top) {
      this.alarmSlot.append(priorityBadge(top.priority));
      this.el.classList.add(`alarm-p${top.priority}`);
    }
    this.renderBar(tag, ctx);
    this.tick(ctx.now);
    this.valueEl.setAttribute(
      "aria-label",
      `${tx(this.reg.name)}: ${valueText} ${this.reg.unit ?? ""}, ${q ? t(`quality.${q}`) : t("tile.waiting")}`,
    );
  }

  private ageMs(): number | null {
    const tag = this.last;
    if (!tag || tag.value_ts_ms === null || !this.ctx) return null;
    return Math.max(0, this.ctx.now - tag.value_ts_ms);
  }

  /** Refresh the age text (called once per second). */
  tick(now: number): void {
    if (this.ctx) this.ctx.now = now;
    const L = lang();
    const age = this.ageMs();
    const cycle = this.ctx?.cycleMs ?? null;
    const parts = [t("tile.age", { age: fmtAge(age, L) })];
    if (cycle !== null) parts.push(t("tile.cycle", { cycle: fmtAge(cycle, L) }));
    setText(this.ageEl, parts.join(" · "));
    if (this.last && this.ctx) {
      const q = displayQuality(this.last, this.ctx.staleAfterMs, now);
      if (q !== this.shown) {
        // Became stale by age (e.g. polling stopped): re-render once with the new quality.
        this.update(this.last, this.ctx);
        return;
      }
      const badge = this.badgeSlot.querySelector(".badge");
      if (badge && q !== "GOOD") {
        const info = qualityInfo(q, this.last.exception);
        const textNode = badge.lastChild;
        if (textNode) textNode.textContent = `${info.badge} · ${fmtAge(age, L)}`;
      }
    }
  }

  private renderBar(tag: TagView | undefined, ctx: TileContext): void {
    clear(this.bar);
    const r = barRange(this.reg);
    if (!r) {
      this.bar.hidden = true;
      return;
    }
    this.bar.hidden = false;
    const track = h("div", { class: "bar-track" });
    const band = normalBand(ctx.alarms, r);
    if (band) {
      const left = pct(band.min, r);
      const width = Math.max(0, pct(band.max, r) - left);
      track.append(h("div", { class: "bar-band", style: `left:${left}%;width:${width}%` }));
    }
    for (const a of ctx.alarms) {
      if (a.limit === null || a.limit < r.min || a.limit > r.max) continue;
      track.append(h("div", { class: `bar-tick p${a.priority}`, style: `left:${pct(a.limit, r)}%`, title: `${a.kind} ${a.limit}` }));
    }
    const v = tag?.value;
    const oos = outOfScale(v, r);
    if (v !== null && v !== undefined) {
      track.append(h("div", { class: `bar-marker${oos ? " oos" : ""}`, style: `left:${pct(v, r)}%` }));
    }
    const L = lang();
    this.bar.append(
      track,
      ...(oos ? [h("div", { class: "bar-oos" }, t(oos === "high" ? "tile.oos_high" : "tile.oos_low"))] : []),
      h(
        "div",
        { class: "bar-scale" },
        h("span", null, fmtNumber(r.min, L)),
        h("span", null, fmtNumber(r.max, L)),
      ),
    );
    this.bar.setAttribute(
      "aria-label",
      t("tile.bar", { min: fmtNumber(r.min, L), max: fmtNumber(r.max, L) }) + (oos ? `. ${t(oos === "high" ? "tile.oos_high" : "tile.oos_low")}` : ""),
    );
  }
}
