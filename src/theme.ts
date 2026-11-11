// Theme: follows the OS by default (light ISA grey unless the OS asks for dark); user can override.
export type ThemeChoice = "system" | "light" | "dark";
const KEY = "buslink.theme";
const listeners = new Set<() => void>();

export function themeChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* default */
  }
  return "system";
}

export function effectiveTheme(): "light" | "dark" {
  const c = themeChoice();
  if (c !== "system") return c;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(choice: ThemeChoice = themeChoice()): void {
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    /* not persisted */
  }
  document.documentElement.dataset.theme = effectiveTheme();
  for (const f of listeners) f();
}

export function onThemeChange(f: () => void): void {
  listeners.add(f);
}

window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", () => {
  if (themeChoice() === "system") applyTheme("system");
});

/** Read a CSS custom property (charts need concrete colors). */
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
