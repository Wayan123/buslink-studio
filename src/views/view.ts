import { h } from "../dom.ts";

export interface View {
  el: HTMLElement;
  destroy?: () => void;
}

/** A status line next to a form or action; text only, announced politely. */
export function statusLine(): { el: HTMLElement; set: (text: string, kind?: "ok" | "error" | "info") => void } {
  const el = h("p", { class: "status-line", role: "status", "aria-live": "polite" });
  return {
    el,
    set(text, kind = "info") {
      el.textContent = text;
      el.dataset.kind = kind;
    },
  };
}

/** Labeled form field with an error slot linked by aria-describedby (WCAG 3.3.1). */
export function field(
  id: string,
  label: string,
  control: HTMLInputElement | HTMLSelectElement,
  hint?: string,
): { el: HTMLElement; error: (text: string | null) => void } {
  control.id = id;
  const err = h("span", { class: "field-error", id: `${id}-err` });
  const hintEl = hint ? h("span", { class: "field-hint", id: `${id}-hint` }, hint) : null;
  control.setAttribute("aria-describedby", [hint ? `${id}-hint` : "", `${id}-err`].filter(Boolean).join(" "));
  const el = h("div", { class: "field" }, h("label", { for: id }, label), control, hintEl, err);
  return {
    el,
    error(text) {
      err.textContent = text ?? "";
      if (text) control.setAttribute("aria-invalid", "true");
      else control.removeAttribute("aria-invalid");
    },
  };
}

export function select(options: [string, string][], value?: string): HTMLSelectElement {
  const s = h("select", null, ...options.map(([v, label]) => h("option", { value: v }, label)));
  if (value !== undefined) s.value = value;
  return s;
}

/**
 * Re-render `container` without losing keyboard focus: elements carry `data-fk`
 * keys, and the focused key is restored after `render` (WCAG 2.4.3 / 2.4.7).
 */
export function preserveFocus(container: HTMLElement, render: () => void): void {
  const active = document.activeElement;
  const key = active instanceof HTMLElement && container.contains(active) ? active.dataset.fk : undefined;
  render();
  if (key) {
    const again = container.querySelector<HTMLElement>(`[data-fk="${CSS.escape(key)}"]`);
    again?.focus();
  }
}
