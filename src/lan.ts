// LAN (browser) mode: the same frontend, served read-only by the app (lan-mqtt design §3).
// The server selects the mode with <meta name="buslink-mode" content="lan">. This module is the
// only transport in that mode: GET snapshot, GET trend, one WebSocket. There is no function here
// that sends a body, and the host has no route that would accept one.
import type { LiveFrame, LiveSnapshot, ProfileView, TrendSeries } from "./types.ts";

export interface LanSnapshot extends LiveSnapshot {
  profiles: ProfileView[];
}

export type StreamState = "connecting" | "live" | "reconnecting";

export type StreamMessage =
  | { type: "snapshot"; data: LanSnapshot }
  | { type: "frame"; data: LiveFrame }
  | { type: "lagged"; data: number };

/** The only views a browser gets: overview (L1/L2) and trends (L3). */
export const LAN_NAV: [string, string][] = [
  ["#/dashboard", "nav.overview"],
  ["#/trends", "nav.trends"],
];

export const LAN_ROUTES: readonly string[] = ["dashboard", "device", "trends"];

export function isLan(): boolean {
  if (typeof document === "undefined") return false;
  return document.querySelector('meta[name="buslink-mode"]')?.getAttribute("content") === "lan";
}

export function lanToken(search: string = typeof location === "undefined" ? "" : location.search): string {
  return new URLSearchParams(search).get("t") ?? "";
}

/** Parse one WebSocket text message; anything unexpected is ignored (null). */
export function parseMessage(text: string): StreamMessage | null {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof v !== "object" || v === null) return null;
  const m = v as { type?: unknown; data?: unknown };
  if (m.type === "snapshot" && isObj(m.data) && Array.isArray((m.data as { tags?: unknown }).tags)) return { type: "snapshot", data: m.data as LanSnapshot };
  if (m.type === "frame" && isObj(m.data) && Array.isArray((m.data as { tags?: unknown }).tags)) return { type: "frame", data: m.data as LiveFrame };
  if (m.type === "lagged" && typeof m.data === "number") return { type: "lagged", data: m.data };
  return null;
}

function isObj(v: unknown): boolean {
  return typeof v === "object" && v !== null;
}

/** Reconnect delay: 1 s, 2 s, 4 s … up to 15 s. */
export function backoffMs(attempt: number): number {
  return Math.min(15_000, 1000 * 2 ** Math.max(0, Math.min(attempt, 10)));
}

export function query(token: string, params: Record<string, string | number> = {}): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) q.set(k, String(v));
  q.set("t", token);
  return `?${q.toString()}`;
}

async function get<T>(path: string, params: Record<string, string | number> = {}): Promise<T> {
  const res = await fetch(`${path}${query(lanToken(), params)}`, { method: "GET", credentials: "same-origin", cache: "no-store" });
  if (!res.ok) throw new Error(`${res.status} ${await res.text().catch(() => "")}`.trim());
  return (await res.json()) as T;
}

export function lanSnapshot(): Promise<LanSnapshot> {
  return get<LanSnapshot>("/api/snapshot");
}

export function lanTrend(req: { device_id: string; tag_id: string; from_ms: number; to_ms: number; buckets: number }): Promise<TrendSeries> {
  return get<TrendSeries>("/api/trend", { tag: `${req.device_id}/${req.tag_id}`, from: Math.floor(req.from_ms), to: Math.ceil(req.to_ms), buckets: req.buckets });
}

export interface StreamHandlers {
  onState: (s: StreamState) => void;
  onSnapshot: (s: LanSnapshot) => void;
  onFrame: (f: LiveFrame) => void;
  onLagged: (missed: number) => void;
}

/** One WebSocket with automatic reconnect; the first message after each connect is a full snapshot. */
export function lanStream(h: StreamHandlers): { close: () => void } {
  let ws: WebSocket | null = null;
  let attempt = 0;
  let closed = false;
  let timer: number | undefined;

  const open = () => {
    if (closed) return;
    h.onState(attempt === 0 ? "connecting" : "reconnecting");
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${scheme}//${location.host}/api/stream${query(lanToken())}`);
    ws.onopen = () => {
      attempt = 0;
      h.onState("live");
    };
    ws.onmessage = (e) => {
      if (typeof e.data !== "string") return;
      const m = parseMessage(e.data);
      if (!m) return;
      if (m.type === "snapshot") h.onSnapshot(m.data);
      else if (m.type === "frame") h.onFrame(m.data);
      else h.onLagged(m.data);
    };
    ws.onclose = () => {
      ws = null;
      if (closed) return;
      h.onState("reconnecting");
      timer = window.setTimeout(open, backoffMs(attempt++));
    };
    ws.onerror = () => ws?.close();
  };
  open();
  return {
    close() {
      closed = true;
      window.clearTimeout(timer);
      ws?.close();
    },
  };
}
