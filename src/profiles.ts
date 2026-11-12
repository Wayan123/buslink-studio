// Profile cache (profiles are bundled and do not change while the app runs).
import { api } from "./api.ts";
import type { DeviceView, Profile, ProfileView, Register } from "./types.ts";

let cache: Promise<Map<string, Profile>> | null = null;
let ready = new Map<string, Profile>();

export function loadProfiles(): Promise<Map<string, Profile>> {
  if (!cache) {
    cache = api.listProfiles().then((list) => {
      ready = new Map(list.map((p) => [p.key, p.profile]));
      return ready;
    });
    cache.catch(() => {
      cache = null;
    });
  }
  return cache;
}

/** LAN mode: profiles arrive inside the snapshot. */
export function primeProfiles(list: ProfileView[]): void {
  ready = new Map(list.map((p) => [p.key, p.profile]));
  cache = Promise.resolve(ready);
}

export function profileOf(d: DeviceView): Profile | undefined {
  return ready.get(`${d.profile_id}@${d.profile_version}`);
}

export function registerOf(d: DeviceView, tagId: string): Register | undefined {
  return profileOf(d)?.registers.find((r) => r.id === tagId);
}
