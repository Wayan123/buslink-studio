// DOM helpers. Text always goes through textContent / text nodes: no innerHTML anywhere (ui.md).

type Child = Node | string | number | null | undefined | false;
type Handler = (e: Event) => void;
export type Attrs = Record<string, string | number | boolean | null | undefined | Handler>;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs?: Attrs | null,
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v === undefined || v === null || v === false) continue;
      if (k.startsWith("on:") && typeof v === "function") {
        el.addEventListener(k.slice(3), v);
      } else if (typeof v === "function") {
        continue;
      } else if (v === true) {
        el.setAttribute(k, "");
      } else {
        el.setAttribute(k, String(v));
      }
    }
  }
  append(el, ...children);
  return el;
}

export function append(el: Node, ...children: (Child | Child[])[]): void {
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    el.appendChild(typeof c === "string" || typeof c === "number" ? document.createTextNode(String(c)) : c);
  }
}

export function clear(el: Node): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

export function setText(el: Element, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

export function svgShape(kind: "triangle" | "diamond" | "square" | "circle" | "hex", label: string, cls: string): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.setAttribute("class", `shape ${cls}`);
  svg.setAttribute("aria-hidden", "true");
  const shapeEl =
    kind === "circle"
      ? document.createElementNS(ns, "circle")
      : document.createElementNS(ns, "polygon");
  if (kind === "circle") {
    shapeEl.setAttribute("cx", "10");
    shapeEl.setAttribute("cy", "10");
    shapeEl.setAttribute("r", "8.5");
  } else {
    const pts: Record<string, string> = {
      triangle: "10,1.5 19,18.5 1,18.5",
      diamond: "10,1 19,10 10,19 1,10",
      square: "2,2 18,2 18,18 2,18",
      hex: "5,1.5 15,1.5 19.5,10 15,18.5 5,18.5 0.5,10",
    };
    shapeEl.setAttribute("points", pts[kind] ?? pts.square ?? "");
  }
  svg.appendChild(shapeEl);
  if (label) {
    const text = document.createElementNS(ns, "text");
    text.setAttribute("x", "10");
    text.setAttribute("y", kind === "triangle" ? "15.5" : "14");
    text.setAttribute("text-anchor", "middle");
    text.textContent = label;
    svg.appendChild(text);
  }
  return svg;
}

export function announce(text: string, urgent = false): void {
  const el = document.getElementById(urgent ? "announcer-urgent" : "announcer");
  if (!el) return;
  el.textContent = "";
  window.setTimeout(() => {
    el.textContent = text;
  }, 30);
}
