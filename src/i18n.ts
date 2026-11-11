// i18n: every UI string comes from i18n/en.json and i18n/id.json (same keys; checked by scripts/check-i18n.mjs).
import en from "./i18n/en.json";
import id from "./i18n/id.json";
import type { Lang } from "./format.ts";
import type { Text } from "./types.ts";

type Dict = Record<string, string>;
const dicts: Record<Lang, Dict> = { en: en as Dict, id: id as Dict };
const STORAGE_KEY = "buslink.lang";

let current: Lang = detect();
const listeners = new Set<() => void>();

function detect(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "en" || saved === "id") return saved;
  } catch {
    /* storage unavailable: follow the system */
  }
  const nav = (navigator.languages?.[0] ?? navigator.language ?? "en").toLowerCase();
  return nav.startsWith("id") || nav.startsWith("in") ? "id" : "en";
}

export function lang(): Lang {
  return current;
}

export function setLang(l: Lang): void {
  current = l;
  try {
    localStorage.setItem(STORAGE_KEY, l);
  } catch {
    /* not persisted; still switched for this run */
  }
  document.documentElement.lang = l;
  for (const f of listeners) f();
}

export function onLangChange(f: () => void): void {
  listeners.add(f);
}

/** Translate `key`, replacing `{name}` placeholders. A missing key shows the key itself (and fails the build check). */
export function t(key: string, vars?: Record<string, string | number>): string {
  let s = dicts[current][key] ?? dicts.en[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
  }
  return s;
}

/** Profile text (plain or bilingual) in the current language. */
export function tx(v: Text | undefined | null): string {
  if (v === undefined || v === null) return "";
  if (typeof v === "string") return v;
  return current === "id" ? v.id : v.en;
}

document.documentElement.lang = current;
