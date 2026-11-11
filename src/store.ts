// Live state in the webview. Pure (no DOM) so it can be unit-tested with node --test.
import type {
  AlarmSnapshot,
  AlarmView,
  DeviceView,
  LinkView,
  LiveFrame,
  LiveSnapshot,
  QualityCode,
  TagView,
} from "./types.ts";

export const BUFFER_MS = 30 * 60_000;
export const MAX_POINTS_PER_TAG = 7_200;

export interface TrendBuffer {
  t: number[];
  v: (number | null)[];
  q: QualityCode[];
}

export interface Change {
  tags: Set<string>;
  links: boolean;
  alarms: boolean;
  points: boolean;
}

export const EMPTY_ALARMS: AlarmSnapshot = {
  alarms: [],
  summary: {
    counts: [0, 0, 0],
    unacked: 0,
    oldest_unacked: null,
    oldest_unacked_ms: null,
    activations_10min: 0,
    flood: false,
    comm_active: 0,
  },
};

export class Store {
  tags = new Map<string, TagView>();
  links: LinkView[] = [];
  alarms: AlarmSnapshot = EMPTY_ALARMS;
  historianError: string | null = null;
  droppedTotal = 0;
  lastFrameMs = 0;
  buffers = new Map<string, TrendBuffer>();
  private listeners = new Set<(c: Change) => void>();

  subscribe(f: (c: Change) => void): () => void {
    this.listeners.add(f);
    return () => this.listeners.delete(f);
  }

  private emit(c: Change): void {
    for (const f of this.listeners) f(c);
  }

  applySnapshot(s: LiveSnapshot): void {
    this.tags.clear();
    for (const tag of s.tags) this.tags.set(tag.key, tag);
    this.links = s.links;
    this.alarms = s.alarms;
    this.historianError = s.historian_error;
    this.emit({ tags: new Set(this.tags.keys()), links: true, alarms: true, points: false });
  }

  applyFrame(f: LiveFrame): void {
    this.lastFrameMs = f.ts_ms;
    const changed = new Set<string>();
    for (const tag of f.tags) {
      this.tags.set(tag.key, tag);
      changed.add(tag.key);
    }
    for (const p of f.points) {
      let b = this.buffers.get(p.k);
      if (!b) {
        b = { t: [], v: [], q: [] };
        this.buffers.set(p.k, b);
      }
      // Points arrive in publish order; keep the buffer sorted even if a late one slips in.
      const last = b.t[b.t.length - 1];
      if (last !== undefined && p.t < last) {
        let i = b.t.length;
        while (i > 0 && (b.t[i - 1] ?? 0) > p.t) i--;
        b.t.splice(i, 0, p.t);
        b.v.splice(i, 0, p.q === "GOOD" ? p.v : null);
        b.q.splice(i, 0, p.q);
      } else {
        b.t.push(p.t);
        b.v.push(p.q === "GOOD" ? p.v : null);
        b.q.push(p.q);
      }
    }
    this.trim(f.ts_ms);
    this.droppedTotal += f.dropped;
    if (f.links) this.links = f.links;
    if (f.alarms) this.alarms = f.alarms;
    this.historianError = f.historian_error;
    this.emit({ tags: changed, links: f.links !== null, alarms: f.alarms !== null, points: f.points.length > 0 });
  }

  private trim(now: number): void {
    const oldest = now - BUFFER_MS;
    for (const b of this.buffers.values()) {
      let cut = 0;
      while (cut < b.t.length && (b.t[cut] ?? 0) < oldest) cut++;
      cut = Math.max(cut, b.t.length - MAX_POINTS_PER_TAG);
      if (cut > 0) {
        b.t.splice(0, cut);
        b.v.splice(0, cut);
        b.q.splice(0, cut);
      }
    }
  }

  devices(): { device: DeviceView; link: LinkView }[] {
    return this.links.flatMap((link) => link.devices.map((device) => ({ device, link })));
  }

  device(id: string): { device: DeviceView; link: LinkView } | undefined {
    return this.devices().find((d) => d.device.id === id);
  }

  tagsOf(deviceId: string): TagView[] {
    return [...this.tags.values()].filter((t) => t.device_id === deviceId);
  }

  alarmsOfTag(deviceId: string, tagId: string): AlarmView[] {
    return this.alarms.alarms.filter((a) => a.class === "process" && a.device_id === deviceId && a.tag_id === tagId);
  }

  commAlarm(deviceId: string): AlarmView | undefined {
    return this.alarms.alarms.find((a) => a.class === "comm" && a.device_id === deviceId);
  }
}

export const store = new Store();
