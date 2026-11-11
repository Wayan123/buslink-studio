// Device library (dashboard-ui Req 2): bundled profiles with support-level badges; add a device.
import { api, errorText } from "../api.ts";
import { simulatedBanner, supportBadge } from "../components/badges.ts";
import { clear, h } from "../dom.ts";
import { t, tx } from "../i18n.ts";
import { store } from "../store.ts";
import type { ProfileView } from "../types.ts";
import { field, statusLine, type View } from "./view.ts";

/** Unit ID rules shown next to the field (the backend checks them again). */
export function unitIdError(unit: number, linkKind: string): string | null {
  if (!Number.isInteger(unit)) return t("library.err.unit_number");
  if (unit === 0) return t("library.err.unit_zero");
  const max = linkKind === "tcp" ? 255 : 247;
  if (unit < 1 || unit > max) return t("library.err.unit_range", { max });
  return null;
}

export function libraryView(): View {
  const list = h("div", { class: "profile-grid" });
  const status = statusLine();

  function card(pv: ProfileView): HTMLElement {
    const p = pv.profile;
    const linkSel = h("select");
    const unitIn = h("input", {
      type: "number",
      inputmode: "numeric",
      min: 1,
      max: 255,
      value: p.transport.rtu?.unit_id ?? p.transport.tcp?.unit_id ?? 1,
    });
    const nameIn = h("input", { type: "text", maxlength: 80, placeholder: tx(p.model) });
    const pollIn = h("input", { type: "number", inputmode: "numeric", min: 100, max: 86400000, step: 100, value: 1000 });
    const idp = p.profile_id.replace(/[^a-z0-9]/g, "-");
    const fLink = field(`lib-link-${idp}`, t("library.link"), linkSel);
    const fUnit = field(`lib-unit-${idp}`, t("device.unit_id"), unitIn, t("library.unit.hint"));
    const fPoll = field(`lib-poll-${idp}`, t("library.poll"), pollIn);
    const links = store.links.filter((l) => l.state === "connected" || l.state === "polling");
    for (const l of links) linkSel.append(h("option", { value: l.id }, l.label));
    const form = h(
      "form",
      { class: "form-grid compact" },
      fLink.el,
      fUnit.el,
      field(`lib-name-${idp}`, t("device.name"), nameIn).el,
      fPoll.el,
      h("button", { type: "submit", class: "primary", disabled: links.length === 0 }, t("library.add")),
    );
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      fUnit.error(null);
      fLink.error(null);
      fPoll.error(null);
      const link = store.links.find((l) => l.id === linkSel.value);
      if (!link) {
        fLink.error(t("library.err.link"));
        return;
      }
      const unit = Number(unitIn.value);
      const ue = unitIdError(unit, link.kind);
      if (ue) {
        fUnit.error(ue);
        return;
      }
      const poll = Number(pollIn.value);
      if (!Number.isInteger(poll) || poll < 100 || poll > 86_400_000) {
        fPoll.error(t("library.err.poll"));
        return;
      }
      try {
        const id = await api.addDevice({
          link_id: link.id,
          profile_key: pv.key,
          unit_id: unit,
          name: nameIn.value.trim() || undefined,
          poll_interval_ms: poll,
        });
        status.set(t("library.added", { id }), "ok");
      } catch (err) {
        fUnit.error(errorText(err));
      }
    });
    return h(
      "article",
      { class: "card profile-card" },
      h("header", { class: "card-head" }, h("h3", null, `${tx(p.vendor)} ${tx(p.model)}`), supportBadge(p.support_level)),
      p.simulated_only ? simulatedBanner() : null,
      h("p", { class: "muted small" }, `${p.profile_id} · v${p.profile_version} · ${t("library.registers", { n: p.registers.length })}`),
      p.description ? h("p", null, tx(p.description)) : null,
      p.source ? h("p", { class: "small" }, t("library.source", { doc: tx(p.source.document), rev: tx(p.source.revision) })) : null,
      links.length === 0 ? h("p", { class: "advice" }, t("library.no_link")) : null,
      form,
    );
  }

  async function load() {
    try {
      const profiles = await api.listProfiles();
      clear(list);
      for (const pv of profiles) list.append(card(pv));
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }

  const el = h(
    "div",
    { class: "view library" },
    h("h2", null, t("nav.library")),
    h("p", null, t("library.intro")),
    h(
      "dl",
      { class: "legend" },
      ...(["generic", "documented", "simulated", "hardware_verified"] as const).flatMap((lvl) => [
        h("dt", null, supportBadge(lvl)),
        h("dd", null, t(`support.${lvl}.help`)),
      ]),
    ),
    status.el,
    list,
  );
  let linkCount = store.links.length;
  const unsub = store.subscribe((c) => {
    if (c.links && store.links.length !== linkCount) {
      linkCount = store.links.length;
      void load();
    }
  });
  void load();
  return { el, destroy: unsub };
}
