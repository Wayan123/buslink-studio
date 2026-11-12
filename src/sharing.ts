// What is shared right now (LAN view, MQTT), polled so the header always shows it:
// sharing must never be on without the operator seeing it (lan-mqtt Req 4.4).
import { api } from "./api.ts";
import { clear, h, svgShape } from "./dom.ts";
import { t } from "./i18n.ts";
import type { MqttState } from "./types.ts";

export interface SharingState {
  lan: boolean;
  viewers: number;
  mqtt: MqttState;
  queue: number;
}

let state: SharingState = { lan: false, viewers: 0, mqtt: "off", queue: 0 };
const listeners = new Set<(s: SharingState) => void>();

export function sharing(): SharingState {
  return state;
}

export function onSharing(f: (s: SharingState) => void): () => void {
  listeners.add(f);
  return () => listeners.delete(f);
}

export function setSharing(next: SharingState): void {
  state = next;
  for (const f of listeners) f(state);
}

export async function refreshSharing(): Promise<void> {
  try {
    const [lan, mqtt] = await Promise.all([api.lanStatus(), api.mqttStatus()]);
    setSharing({ lan: lan.running, viewers: lan.details?.clients.length ?? 0, mqtt: mqtt.state, queue: mqtt.outbox_depth });
  } catch {
    /* no backend (static page): nothing is shared */
  }
}

export function startSharingPoll(): void {
  void refreshSharing();
  window.setInterval(() => void refreshSharing(), 3000);
}

/** Header indicators: text + shape, links to the panels. Empty while nothing is on. */
export function sharingIndicator(): HTMLElement {
  const el = h("div", { class: "share-indicators", role: "status", "aria-live": "polite" });
  const render = (s: SharingState) => {
    clear(el);
    if (s.lan) {
      el.append(h("a", { class: "badge sharing", href: "#/share", "data-testid": "ind-lan" }, svgShape("hex", "", "sharing-shape"), t("share.ind.lan", { n: s.viewers })));
    }
    if (s.mqtt !== "off") {
      el.append(h("a", { class: "badge sharing", href: "#/mqtt", "data-testid": "ind-mqtt" }, svgShape("hex", "", "sharing-shape"), t("share.ind.mqtt", { state: t(`mqtt.state.${s.mqtt}`), n: s.queue })));
    }
  };
  onSharing(render);
  render(state);
  return el;
}
