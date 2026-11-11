// Trends (dashboard-ui Req 4): uPlot, null gaps with hatched bands and reasons, alarm-limit bands,
// drag-zoom + reset, ≤ 4 Hz refresh. Live ranges come from the store; long ranges from the historian
// with min-max decimation in Rust.
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { api, errorText } from "../api.ts";
import { clear, h } from "../dom.ts";
import { fmtLocalTime, fmtNumber, fmtUtc } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { loadProfiles, profileOf } from "../profiles.ts";
import { align, bufferToSeries, firstPointMs, liveBounds, NO_DATA, type Series } from "../series.ts";
import { store } from "../store.ts";
import { cssVar, onThemeChange } from "../theme.ts";
import type { AlarmView, DeviceView, Gap, Register } from "../types.ts";
import { statusLine, type View } from "./view.ts";

interface TagRef {
  key: string;
  device: DeviceView;
  reg: Register;
}

type RangeId = "5m" | "15m" | "30m" | "1h" | "8h" | "session";
const LIVE: Record<string, number> = { "5m": 5 * 60_000, "15m": 15 * 60_000, "30m": 30 * 60_000 };
const HIST: Record<string, number> = { "1h": 3_600_000, "8h": 8 * 3_600_000 };
const MAX_TAGS = 6;
const DASHES: number[][] = [[], [10, 5], [2, 4], [14, 4, 3, 4], [6, 3], [1, 3]];
const REFRESH_MS = 250;

export function gapLabel(g: Gap): string {
  return g.reason === NO_DATA ? t("trend.no_data") : t("trend.no_data_reason", { reason: g.reason });
}

function numericRegisters(d: DeviceView): Register[] {
  return (profileOf(d)?.registers ?? []).filter((r) => r.type !== "string" && r.type !== "bool");
}

function hatch(ctx: CanvasRenderingContext2D, color: string): CanvasPattern | string {
  const c = document.createElement("canvas");
  c.width = 10;
  c.height = 10;
  const g = c.getContext("2d");
  if (!g) return color;
  g.strokeStyle = color;
  g.globalAlpha = 0.45;
  g.lineWidth = 2;
  g.beginPath();
  g.moveTo(0, 10);
  g.lineTo(10, 0);
  g.stroke();
  return ctx.createPattern(c, "repeat") ?? color;
}

export function trendsView(params: { device?: string } = {}): View {
  const selected = new Set<string>();
  let range: RangeId = "5m";
  let charts: { plot: uPlot; tags: TagRef[]; series: Series[]; box: HTMLElement }[] = [];
  let zoomed = false;
  let pending = false;
  let lastDraw = 0;
  const status = statusLine();
  const picker = h("fieldset", { class: "card tag-picker" });
  const chartsBox = h("div", { class: "charts" });
  const note = h("p", { class: "muted small", role: "note" });
  const gapsList = h("ul", { class: "gap-list", "aria-label": t("trend.gaps") });

  const rangeBox = h("div", { class: "segmented", role: "group", "aria-label": t("trend.range") });
  const ranges: RangeId[] = ["5m", "15m", "30m", "1h", "8h", "session"];
  for (const r of ranges) {
    const b = h("button", { type: "button", "aria-pressed": String(r === range), "data-range": r }, t(`trend.range.${r}`));
    b.addEventListener("click", () => {
      range = r;
      zoomed = false;
      for (const x of rangeBox.querySelectorAll("button")) x.setAttribute("aria-pressed", String(x.dataset.range === r));
      void rebuild();
    });
    rangeBox.append(b);
  }
  const resetBtn = h("button", { type: "button" }, t("trend.reset_zoom"));
  resetBtn.addEventListener("click", () => {
    zoomed = false;
    void draw(true);
  });
  const reloadBtn = h("button", { type: "button" }, t("trend.reload"));
  reloadBtn.addEventListener("click", () => void draw(true));

  function tagRefs(): TagRef[] {
    const out: TagRef[] = [];
    for (const { device } of store.devices()) {
      for (const reg of numericRegisters(device)) {
        const key = `${device.id}/${reg.id}`;
        if (selected.has(key)) out.push({ key, device, reg });
      }
    }
    return out;
  }

  function renderPicker() {
    clear(picker);
    picker.append(h("legend", null, t("trend.pick", { n: MAX_TAGS })));
    for (const { device } of store.devices()) {
      const regs = numericRegisters(device);
      if (!regs.length) continue;
      const group = h("div", { class: "pick-group" }, h("p", { class: "pick-device" }, device.name));
      for (const reg of regs) {
        const key = `${device.id}/${reg.id}`;
        const cb = h("input", { type: "checkbox", value: key, checked: selected.has(key) });
        cb.addEventListener("change", () => {
          if (cb.checked) {
            if (selected.size >= MAX_TAGS) {
              cb.checked = false;
              status.set(t("trend.too_many", { n: MAX_TAGS }), "error");
              return;
            }
            selected.add(key);
          } else selected.delete(key);
          void rebuild();
        });
        group.append(h("label", { class: "check" }, cb, ` ${tx(reg.name)}${reg.unit ? ` (${reg.unit})` : ""}`));
      }
      picker.append(group);
    }
  }

  function bounds(): { from: number; to: number } {
    const now = Date.now();
    if (range in LIVE) {
      // Auto-fit from the first buffered point of the selected tags, capped at the chosen window.
      const first = firstPointMs(tagRefs().map((tg) => store.buffers.get(tg.key)));
      return liveBounds(now, LIVE[range] ?? 300_000, first);
    }
    if (range in HIST) return { from: now - (HIST[range] ?? 3_600_000), to: now };
    return { from: 0, to: now };
  }

  async function seriesFor(tags: TagRef[], width: number): Promise<{ series: Series[]; decimated: boolean }> {
    const { from, to } = bounds();
    if (range in LIVE) {
      return {
        series: tags.map((tg) => bufferToSeries(store.buffers.get(tg.key), from, to, tg.device.stale_after_ms)),
        decimated: false,
      };
    }
    const res = await Promise.all(
      tags.map((tg) => api.trend({ device_id: tg.device.id, tag_id: tg.reg.id, from_ms: from, to_ms: to, buckets: Math.max(50, Math.floor(width)) })),
    );
    return {
      series: res.map((r) => ({ t: r.t, v: r.v, gaps: r.gaps })),
      decimated: res.some((r) => r.decimated),
    };
  }

  function groups(tags: TagRef[]): TagRef[][] {
    // At most two units (two y-axes) per chart.
    const units = [...new Set(tags.map((tg) => tg.reg.unit ?? ""))];
    const out: TagRef[][] = [];
    for (let i = 0; i < units.length; i += 2) {
      const us = units.slice(i, i + 2);
      out.push(tags.filter((tg) => us.includes(tg.reg.unit ?? "")));
    }
    return out;
  }

  function makePlot(box: HTMLElement, tags: TagRef[], getSeries: () => Series[], getAlarms: () => AlarmView[]): uPlot {
    const L = lang();
    const text = cssVar("--text") || "#1F1F1F";
    const muted = cssVar("--text-muted") || "#4D4D4D";
    const grid = cssVar("--grid") || "#C4C4C4";
    const bad = cssVar("--quality-bad") || "#AA3377";
    const palette = [cssVar("--series-1"), cssVar("--series-2"), cssVar("--series-3"), cssVar("--series-4"), cssVar("--series-5"), cssVar("--series-6")];
    const units = [...new Set(tags.map((tg) => tg.reg.unit ?? ""))];
    const prioColor = (p: number) => cssVar(`--alarm-${p}`) || "#CC3311";
    const tooltip = h("div", { class: "chart-tip", "aria-hidden": "true" });
    const opts: uPlot.Options = {
      width: Math.max(280, box.clientWidth - 8),
      height: 260,
      ms: 1,
      cursor: { drag: { x: true, y: false, setScale: true } },
      scales: Object.fromEntries([["x", { time: true }], ...units.map((u) => [u || "_", { auto: true }])]),
      legend: { show: true, live: true },
      series: [
        { label: t("trend.time"), value: (_u, v) => (v === null || v === undefined ? "—" : fmtLocalTime(v, L)) },
        ...tags.map((tg, i) => ({
          label: `${tg.device.name}: ${tx(tg.reg.name)}${tg.reg.unit ? ` (${tg.reg.unit})` : ""}`,
          scale: tg.reg.unit || "_",
          stroke: palette[i % palette.length] || text,
          width: 2,
          dash: DASHES[i % DASHES.length],
          spanGaps: false,
          points: { show: false },
          value: (_u: uPlot, v: number | null) => (v === null || v === undefined ? "—" : fmtNumber(v, L, 3)),
        })),
      ],
      axes: [
        { stroke: muted, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid } },
        ...units.map((u, i) => ({
          scale: u || "_",
          side: i === 0 ? 3 : 1,
          label: u || t("trend.value"),
          stroke: muted,
          grid: { show: i === 0, stroke: grid, width: 1 },
          ticks: { stroke: grid },
          values: (_u: uPlot, vals: number[]) => vals.map((v) => fmtNumber(v, L)),
        })),
      ],
      hooks: {
        drawClear: [
          (u) => {
            const ctx = u.ctx;
            const { left, top, width, height } = u.bbox;
            ctx.save();
            ctx.beginPath();
            ctx.rect(left, top, width, height);
            ctx.clip();
            // Alarm-limit bands.
            for (const a of getAlarms()) {
              const tg = tags.find((x) => x.device.id === a.device_id && x.reg.id === a.tag_id);
              if (!tg || a.limit === null) continue;
              const y = u.valToPos(a.limit, tg.reg.unit || "_", true);
              if (!Number.isFinite(y)) continue;
              const high = a.kind === "HI" || a.kind === "HI_HI";
              ctx.globalAlpha = 0.12;
              ctx.fillStyle = prioColor(a.priority);
              if (high) ctx.fillRect(left, top, width, Math.max(0, y - top));
              else ctx.fillRect(left, y, width, Math.max(0, top + height - y));
              ctx.globalAlpha = 1;
              ctx.strokeStyle = prioColor(a.priority);
              ctx.setLineDash([6, 4]);
              ctx.lineWidth = 1.5;
              ctx.beginPath();
              ctx.moveTo(left, y);
              ctx.lineTo(left + width, y);
              ctx.stroke();
              ctx.setLineDash([]);
            }
            // Gap bands (hatched) with their reason.
            const pattern = hatch(ctx, bad);
            ctx.font = `${12 * devicePixelRatio}px system-ui, sans-serif`;
            for (const s of getSeries()) {
              for (const g of s.gaps) {
                const x0 = Math.max(left, u.valToPos(g.from_ms, "x", true));
                const x1 = Math.min(left + width, u.valToPos(g.to_ms, "x", true));
                if (!(x1 > x0)) continue;
                ctx.fillStyle = pattern;
                ctx.fillRect(x0, top, Math.max(2, x1 - x0), height);
                const label = gapLabel(g);
                if (x1 - x0 > ctx.measureText(label).width + 8) {
                  ctx.fillStyle = text;
                  ctx.fillText(label, x0 + 4, top + 16 * devicePixelRatio);
                }
              }
            }
            ctx.restore();
          },
        ],
        setSelect: [
          (u) => {
            if (u.select.width > 0) zoomed = true;
          },
        ],
        setCursor: [
          (u) => {
            const idx = u.cursor.idx;
            if (idx === null || idx === undefined) {
              tooltip.textContent = "";
              return;
            }
            const x = u.data[0][idx];
            if (x === undefined) return;
            const lines = [`${fmtLocalTime(x, lang())} · UTC ${fmtUtc(x)}`];
            getSeries().forEach((s, i) => {
              const v = u.data[i + 1]?.[idx];
              const gap = s.gaps.find((g) => x >= g.from_ms && x <= g.to_ms);
              const name = tags[i] ? tx(tags[i].reg.name) : "";
              if (gap && (v === null || v === undefined)) lines.push(`${name}: ${gapLabel(gap)}`);
              else if (v !== null && v !== undefined) lines.push(`${name}: ${fmtNumber(v, lang(), 3)} (GOOD)`);
            });
            tooltip.textContent = lines.join("\n");
          },
        ],
      },
    };
    const plot = new uPlot(opts, [[]], box);
    box.append(tooltip);
    return plot;
  }

  function renderGapList() {
    clear(gapsList);
    const L = lang();
    let n = 0;
    charts.forEach((c) =>
      c.series.forEach((s, i) => {
        for (const g of s.gaps) {
          if (n++ > 40) return;
          const tag = c.tags[i];
          gapsList.append(
            h(
              "li",
              { class: "gap-item" },
              `${tag ? `${tag.device.name}: ${tx(tag.reg.name)} — ` : ""}${gapLabel(g)} · ${fmtLocalTime(g.from_ms, L)}–${fmtLocalTime(g.to_ms, L)}`,
            ),
          );
        }
      }),
    );
    if (n === 0) gapsList.append(h("li", { class: "muted" }, t("trend.gaps.none")));
  }

  async function draw(force = false) {
    if (!charts.length) return;
    const now = Date.now();
    if (!force && now - lastDraw < REFRESH_MS) {
      if (!pending) {
        pending = true;
        window.setTimeout(() => {
          pending = false;
          void draw(true);
        }, REFRESH_MS);
      }
      return;
    }
    lastDraw = now;
    const started = performance.now();
    let anyDecimated = false;
    try {
      for (const c of charts) {
        const { series, decimated } = await seriesFor(c.tags, c.box.clientWidth);
        anyDecimated ||= decimated;
        c.series = series;
        const { x, ys } = align(series);
        c.plot.setData([x, ...ys] as uPlot.AlignedData, false);
        if (!zoomed) {
          const { from, to } = bounds();
          const min = range === "session" ? (x[0] ?? from) : from;
          c.plot.setScale("x", { min, max: to });
        }
      }
      note.textContent = `${anyDecimated ? t("trend.decimated") : range in LIVE ? t("trend.raw_live") : t("trend.raw")} · ${t("trend.render_ms", { ms: fmtNumber(performance.now() - started, lang(), 1) })}`;
      renderGapList();
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }

  async function rebuild() {
    for (const c of charts) c.plot.destroy();
    charts = [];
    clear(chartsBox);
    const tags = tagRefs();
    if (!tags.length) {
      chartsBox.append(h("p", { class: "muted" }, t("trend.empty")));
      renderGapList();
      return;
    }
    for (const g of groups(tags)) {
      const box = h("figure", { class: "card chart", "aria-label": t("trend.chart_label", { tags: g.map((x) => tx(x.reg.name)).join(", ") }) });
      chartsBox.append(box);
      const entry: { plot: uPlot; tags: TagRef[]; series: Series[]; box: HTMLElement } = {
        plot: null as unknown as uPlot,
        tags: g,
        series: [],
        box,
      };
      entry.plot = makePlot(
        box,
        g,
        () => entry.series,
        () => store.alarms.alarms.filter((a) => a.class === "process" && g.some((x) => x.device.id === a.device_id && x.reg.id === a.tag_id)),
      );
      charts.push(entry);
    }
    await draw(true);
  }

  const unsub = store.subscribe((c) => {
    if (c.links) {
      const before = picker.querySelectorAll("input").length;
      const count = store.devices().reduce((n, d) => n + numericRegisters(d.device).length, 0);
      if (before !== count) renderPicker();
    }
    if (c.points && range in LIVE) void draw();
  });
  const ro = new ResizeObserver(() => {
    for (const c of charts) c.plot.setSize({ width: Math.max(280, c.box.clientWidth - 8), height: 260 });
  });
  ro.observe(chartsBox);
  let destroyed = false;
  onThemeChange(() => {
    if (!destroyed) void rebuild();
  });

  const el = h(
    "div",
    { class: "view trends" },
    h("header", { class: "view-head" }, h("h2", null, t("nav.trends")), h("div", { class: "actions" }, rangeBox, resetBtn, reloadBtn)),
    h("p", { class: "muted small" }, t("trend.help")),
    status.el,
    h("div", { class: "trend-layout" }, picker, h("div", { class: "trend-main" }, chartsBox, note, h("h3", null, t("trend.gaps")), gapsList)),
  );

  void loadProfiles().then(() => {
    const devs = store.devices();
    const first = params.device ? devs.find((d) => d.device.id === params.device) : devs[0];
    if (first) for (const r of numericRegisters(first.device).slice(0, 2)) selected.add(`${first.device.id}/${r.id}`);
    renderPicker();
    void rebuild();
  });
  return {
    el,
    destroy() {
      destroyed = true;
      unsub();
      ro.disconnect();
      for (const c of charts) c.plot.destroy();
    },
  };
}
