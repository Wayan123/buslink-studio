// Status badges: every state has a text label and a shape; color is redundant (WCAG 1.4.1).
import { h, svgShape } from "../dom.ts";
import { t } from "../i18n.ts";
import { qualityInfo } from "../quality.ts";
import type { AlarmStateCode, LinkStateCode, QualityCode, SupportLevel } from "../types.ts";

export function qualityBadge(code: QualityCode, exception: number | null, ageText?: string): HTMLElement | null {
  const q = qualityInfo(code, exception);
  if (!q.badge) return null;
  const label = ageText ? `${q.badge} · ${ageText}` : q.badge;
  return h(
    "span",
    { class: `badge ${q.cls}`, title: `${t(`quality.${code}`)} (OPC UA ${q.opcua})` },
    svgShape(code === "STALE" ? "circle" : code === "DISCONNECTED" ? "square" : "hex", "", "q-shape"),
    label,
  );
}

/** Priority: P1 red triangle "1", P2 orange diamond "2", P3 yellow square "3" (research/02 §1). */
export function priorityBadge(p: number, withText = true): HTMLElement {
  const kind = p === 1 ? "triangle" : p === 2 ? "diamond" : "square";
  return h(
    "span",
    { class: `prio prio-${p}`, title: t(`alarm.priority.${p}`) },
    svgShape(kind, String(p), `prio-shape p${p}`),
    withText ? h("span", { class: "prio-text" }, `P${p}`) : null,
  );
}

export function alarmStateBadge(s: AlarmStateCode): HTMLElement {
  return h("span", { class: `astate astate-${s.toLowerCase()}` }, t(`alarm.state.${s}`));
}

export function linkStateBadge(s: LinkStateCode): HTMLElement {
  const abnormal = s === "down" || s === "disconnected";
  return h(
    "span",
    { class: `badge link-${s}${abnormal ? " q-disc" : ""}` },
    abnormal ? svgShape("square", "", "q-shape") : null,
    t(`link.state.${s}`),
  );
}

export function supportBadge(level: SupportLevel): HTMLElement {
  const shape = level === "hardware_verified" ? "circle" : level === "documented" ? "square" : level === "simulated" ? "diamond" : "hex";
  return h(
    "span",
    { class: `support support-${level}`, title: t(`support.${level}.help`) },
    svgShape(shape, "", "support-shape"),
    t(`support.${level}`),
  );
}

export function simulatedBanner(): HTMLElement {
  return h("p", { class: "sim-banner", role: "note" }, svgShape("diamond", "", "support-shape"), t("sim.banner"));
}
