// Locale formatting for the UI only. Exports never use these (they come from Rust with `.` decimals and UTC).

export type Lang = "en" | "id";

const nf = new Map<string, Intl.NumberFormat>();

function formatter(lang: Lang, decimals: number | undefined): Intl.NumberFormat {
  const key = `${lang}:${decimals ?? "auto"}`;
  let f = nf.get(key);
  if (!f) {
    f = new Intl.NumberFormat(lang === "id" ? "id-ID" : "en-US", {
      minimumFractionDigits: decimals ?? 0,
      maximumFractionDigits: decimals ?? 3,
      useGrouping: true,
    });
    nf.set(key, f);
  }
  return f;
}

export function fmtNumber(v: number | null | undefined, lang: Lang, decimals?: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  return formatter(lang, decimals).format(v);
}

/** Decimals implied by a scale such as 0.1 or 0.01 when the profile gives none. */
export function decimalsFor(scale: number | undefined, decimals: number | undefined): number | undefined {
  if (decimals !== undefined) return decimals;
  if (scale === undefined || scale === 1) return 0;
  const d = Math.round(-Math.log10(Math.abs(scale)));
  return Math.max(0, Math.min(6, d));
}

/** Compact age: "800 ms", "12 s", "3 min", "2 h". */
export function fmtAge(ms: number | null | undefined, lang: Lang): string {
  if (ms === null || ms === undefined || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${fmtNumber(Math.floor(ms / 1000), lang)} s`;
  if (ms < 3_600_000) return `${fmtNumber(Math.floor(ms / 60_000), lang)} min`;
  return `${fmtNumber(Math.floor(ms / 3_600_000), lang)} h`;
}

export function fmtLocalTime(ms: number | null | undefined, lang: Lang): string {
  if (ms === null || ms === undefined) return "—";
  return new Date(ms).toLocaleTimeString(lang === "id" ? "id-ID" : "en-GB", { hour12: false });
}

export function fmtLocalDateTime(ms: number | null | undefined, lang: Lang): string {
  if (ms === null || ms === undefined) return "—";
  return new Date(ms).toLocaleString(lang === "id" ? "id-ID" : "en-GB", { hour12: false });
}

export function fmtUtc(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "—";
  return new Date(ms).toISOString();
}

export function hex4(n: number): string {
  return n.toString(16).toUpperCase().padStart(4, "0");
}
