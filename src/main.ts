// BusLink Studio UI: shell, router and live-data subscription.
// Navigation follows the ISA-101 display levels: L1 overview, L2 device, L3 trends, L4 inspector/diagnostics.
import "./styles.css";
import { api, errorText, hasBackend, onFrame } from "./api.ts";
import { announce, clear, h, setText } from "./dom.ts";
import { lang, onLangChange, setLang, t } from "./i18n.ts";
import { loadProfiles } from "./profiles.ts";
import { store } from "./store.ts";
import { applyTheme, themeChoice, type ThemeChoice } from "./theme.ts";
import type { AlarmView, LinkView } from "./types.ts";
import { alarmBanner, alarmsView, alarmTitle } from "./views/alarms.ts";
import { auditView } from "./views/audit.ts";
import { commissionView } from "./views/commission.ts";
import { controlView } from "./views/control.ts";
import { reportView } from "./views/report.ts";
import { connectView } from "./views/connect.ts";
import { dashboardView } from "./views/dashboard.ts";
import { inspectorView } from "./views/inspector.ts";
import { libraryView } from "./views/library.ts";
import { trainingView } from "./views/training.ts";
import { trendsView } from "./views/trends.ts";
import type { View } from "./views/view.ts";

applyTheme();

const NAV: [string, string][] = [
  ["#/dashboard", "nav.overview"],
  ["#/connect", "nav.connect"],
  ["#/library", "nav.library"],
  ["#/trends", "nav.trends"],
  ["#/inspector", "nav.inspector"],
  ["#/alarms", "nav.alarms"],
  ["#/control", "nav.control"],
  ["#/commission", "nav.commission"],
  ["#/audit", "nav.audit"],
  ["#/report", "nav.report"],
  ["#/training", "nav.training"],
];

const root = document.getElementById("app") ?? document.body;
let current: View | null = null;
const main = h("main", { id: "main", tabindex: -1 });
const navEl = h("nav", { class: "nav", "aria-label": t("nav.label") });
const notices = h("div", { class: "notices" });
const footer = h("footer", { class: "app-footer" }, "BusLink Studio");

function route(): void {
  current?.destroy?.();
  clear(main);
  const [path = "", arg] = location.hash.replace(/^#\/?/, "").split("/");
  const param = arg ? decodeURIComponent(arg) : undefined;
  const make: Record<string, () => View> = {
    dashboard: () => dashboardView(),
    device: () => dashboardView({ device: param }),
    connect: () => connectView(),
    library: () => libraryView(),
    trends: () => trendsView({ device: param }),
    inspector: () => inspectorView({ device: param }),
    alarms: () => alarmsView(),
    control: () => controlView({ device: param }),
    commission: () => commissionView({ device: param }),
    audit: () => auditView(),
    report: () => reportView(),
    training: () => trainingView(),
  };
  const fallback = store.links.length ? "dashboard" : "connect";
  const factory = make[path] ?? make[fallback];
  current = factory ? factory() : null;
  if (current) main.append(current.el);
  for (const a of navEl.querySelectorAll("a")) {
    const active = a.getAttribute("href") === `#/${path === "device" ? "dashboard" : path || fallback}`;
    if (active) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
  const heading = main.querySelector("h2");
  document.title = `${heading?.textContent ?? ""} · BusLink Studio`;
}

function renderNav(): void {
  clear(navEl);
  navEl.append(h("ul", null, ...NAV.map(([href, key]) => h("li", null, h("a", { href }, t(key))))));
}

function header(): HTMLElement {
  const langSel = h(
    "select",
    { id: "lang", "aria-label": t("settings.language") },
    h("option", { value: "id" }, "Bahasa Indonesia"),
    h("option", { value: "en" }, "English"),
  );
  langSel.value = lang();
  langSel.addEventListener("change", () => setLang(langSel.value === "id" ? "id" : "en"));
  const themeSel = h(
    "select",
    { id: "theme", "aria-label": t("settings.theme") },
    h("option", { value: "system" }, t("settings.theme.system")),
    h("option", { value: "light" }, t("settings.theme.light")),
    h("option", { value: "dark" }, t("settings.theme.dark")),
  );
  themeSel.value = themeChoice();
  themeSel.addEventListener("change", () => applyTheme(themeSel.value as ThemeChoice));
  return h(
    "header",
    { class: "app-header" },
    h("a", { class: "skip", href: "#main" }, t("nav.skip")),
    h("div", { class: "brand" }, h("span", { class: "brand-name" }, "BusLink Studio"), h("span", { class: "brand-sub" }, t("app.tagline"))),
    alarmBanner(),
    h("div", { class: "settings" }, langSel, themeSel),
    navEl,
  );
}

function renderNotices(): void {
  clear(notices);
  if (!hasBackend()) notices.append(h("p", { class: "notice error-text", role: "alert" }, t("app.no_backend")));
  if (store.historianError) notices.append(h("p", { class: "notice error-text" }, t("app.historian_error", { error: store.historianError })));
  if (store.droppedTotal > 0) notices.append(h("p", { class: "notice" }, t("app.dropped", { n: store.droppedTotal })));
  if (store.links.some((l) => l.simulated)) notices.append(h("p", { class: "notice sim-note" }, t("app.training_active")));
}

// Announce link changes and new alarm activations (never individual samples).
let prevLinks = new Map<string, string>();
let prevActive = new Set<string>();
function announceChanges(links: LinkView[] | null, alarms: AlarmView[] | null): void {
  if (links) {
    const now = new Map(links.map((l) => [l.id, l.state]));
    for (const l of links) {
      const before = prevLinks.get(l.id);
      if (before && before !== l.state) announce(t("announce.link", { link: l.label, state: t(`link.state.${l.state}`) }));
    }
    prevLinks = now;
  }
  if (alarms) {
    const active = new Set(alarms.filter((a) => a.active).map((a) => a.id));
    for (const a of alarms) {
      if (a.active && !prevActive.has(a.id) && a.state !== "SHELVED" && a.state !== "OUT_OF_SERVICE") {
        announce(t("announce.alarm", { p: a.priority, text: alarmTitle(a) }), a.priority === 1);
      }
    }
    prevActive = active;
  }
}

async function boot(): Promise<void> {
  shell();
  window.addEventListener("hashchange", route);
  // Safety rule 3: arming is dropped when the app goes to the background (minimized, phone home button).
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && hasBackend()) void api.controlDisarm("app_background").catch(() => undefined);
  });
  if (!hasBackend()) {
    route();
    return;
  }
  try {
    await loadProfiles();
    const snap = await api.snapshot();
    store.applySnapshot(snap);
    prevLinks = new Map(snap.links.map((l) => [l.id, l.state]));
    prevActive = new Set(snap.alarms.alarms.filter((a) => a.active).map((a) => a.id));
    await onFrame((f) => {
      store.applyFrame(f);
      announceChanges(f.links, f.alarms?.alarms ?? null);
    });
    store.subscribe((c) => {
      if (c.links || c.points) renderNotices();
    });
  } catch (e) {
    notices.append(h("p", { class: "notice error-text", role: "alert" }, errorText(e)));
  }
  route();
  const appVersion = await api.appInfo().catch(() => null);
  if (appVersion?.profile_errors.length) {
    notices.append(h("p", { class: "notice error-text" }, t("app.profile_errors", { list: appVersion.profile_errors.join("; ") })));
  }
  info = appVersion;
  renderFooter();
}

let info: Awaited<ReturnType<typeof api.appInfo>> | null = null;
function renderFooter(): void {
  setText(footer, info ? `BusLink Studio ${info.version} · ${info.platform} · ${t("app.session")} ${info.session_id} · Apache-2.0` : "BusLink Studio");
}

function shell(): void {
  clear(root);
  renderNav();
  root.append(header(), notices, main, footer);
  renderNotices();
  renderFooter();
}

let booted = false;
const start = () => {
  if (booted) {
    // Language switch: rebuild the shell and the current view with the new strings.
    shell();
    route();
    return;
  }
  booted = true;
  void boot();
};
onLangChange(start);
start();
