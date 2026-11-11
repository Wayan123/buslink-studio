// Commissioning wizard (commissioning-control Req 3, PRD R17): unit ID or baud for one device.
// Journal → arm → preview + confirm → send once → reconnect at the new settings → read back → pass/fail.
// The old settings stay on screen as the recovery hint after anything was sent.
import { api, errorText, hasBackend } from "../api.ts";
import { simulatedBanner } from "../components/badges.ts";
import { confirmCommand } from "../components/confirm.ts";
import { ackKey } from "../control.ts";
import { append, clear, h, setText, svgShape } from "../dom.ts";
import { fmtNumber } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { loadProfiles, profileOf } from "../profiles.ts";
import { store } from "../store.ts";
import type { CommSettingKind, CommSettings, ControlStatus, DeviceView, Profile, Wizard, WizardStepCode } from "../types.ts";
import { savedUser, saveUser } from "./control.ts";
import { field, preserveFocus, select, statusLine, type View } from "./view.ts";

const STEPS = ["journal", "arm", "confirm", "send", "reconnect", "verify"] as const;

export function settingsText(s: CommSettings): string {
  if (!s.serial) return t("wiz.settings.tcp", { unit: s.unit_id });
  const p = s.serial.parity === "none" ? "N" : s.serial.parity === "even" ? "E" : "O";
  return t("wiz.settings.rtu", { unit: s.unit_id, baud: s.serial.baud, frame: `${s.serial.data_bits}${p}${s.serial.stop_bits}` });
}

/** Index of the current step in STEPS for a wizard state (and whether the device is armed). */
export function stepIndex(w: Wizard | null, armed: boolean, previewed: boolean): number {
  if (!w) return 0;
  const map: Record<WizardStepCode, number> = {
    journaled: armed ? (previewed ? 2 : 1) : 1,
    await_power_cycle: 4,
    await_reconnect: 4,
    reconnected: 5,
    passed: 6,
    failed: 6,
    needs_verification: 6,
    reverted: 6,
  };
  return map[w.step];
}

export function commissionView(params: { device?: string } = {}): View {
  const status = statusLine();
  const body = h("div", { class: "wizard-body" });
  const el = h("div", { class: "view commission" }, h("header", { class: "view-head" }, h("h2", null, t("wiz.title"))), status.el, body);
  if (!hasBackend()) {
    body.append(h("p", { class: "notice" }, t("ctl.no_backend")));
    return { el };
  }
  let destroyed = false;
  let timer = 0;

  void loadProfiles().then(async () => {
    if (destroyed) return;
    const entry = params.device ? store.device(params.device) : undefined;
    const profile = entry ? profileOf(entry.device) : undefined;
    if (!entry || !profile) {
      body.append(h("p", { class: "notice" }, t("wiz.pick_device")), picker());
      return;
    }
    const support = await api.wizardSupport(entry.device.id).catch((e) => ({ settings: [], reason: errorText(e) }));
    if (!support.settings.length) {
      body.append(h("p", { class: "notice", "data-testid": "wiz-unavailable" }, t("wiz.unavailable", { reason: support.reason ?? "" })));
      return;
    }
    build(entry.device, profile, support.settings, entry.link.simulated);
  });

  function picker(): HTMLElement {
    const list = h("ul", { class: "link-list" });
    for (const { device } of store.devices()) {
      const p = profileOf(device);
      if (!p?.comm_config || p.capabilities?.comm_config === "unsupported") continue;
      list.append(h("li", null, h("a", { href: `#/commission/${encodeURIComponent(device.id)}` }, device.name)));
    }
    if (!list.childElementCount) list.append(h("li", { class: "muted" }, t("wiz.none_available")));
    return list;
  }

  function build(device: DeviceView, profile: Profile, settings: CommSettingKind[], simulated: boolean) {
    const steps = h("ol", { class: "wiz-steps", "aria-label": t("wiz.steps") });
    const panel = h("section", { class: "card wiz-panel", "aria-live": "polite", "data-testid": "wiz-panel" });
    const recovery = h("section", { class: "card recovery", "aria-labelledby": "rec-h", "data-testid": "wiz-recovery" });
    append(
      body,
      simulated || profile.simulated_only ? simulatedBanner() : null,
      h("p", { class: "device-line" }, t("ctl.device_line", { device: device.name, unit: device.unit_id, profile: `${device.profile_id} ${device.profile_version}` })),
      h("p", { class: "advice" }, t("wiz.advice")),
      steps,
      panel,
      recovery,
    );

    // Start form (built once so typing survives the status polls).
    const settingSel = select(settings.map((s) => [s, t(`wiz.setting.${s}`)] as [string, string]), settings[0]);
    const unitIn = h("input", { type: "number", min: 1, max: 247, step: 1, inputmode: "numeric", "data-testid": "wiz-unit", "data-fk": "unit" });
    const codes = profile.comm_config?.baud?.codes ?? {};
    const baudSel = select(Object.values(codes).map((b) => [String(b), `${b}`] as [string, string]));
    const userIn = h("input", { type: "text", value: savedUser(), maxlength: 64, autocomplete: "username", "data-testid": "wiz-user", "data-fk": "user" });
    const fSetting = field("wiz-setting", t("wiz.setting"), settingSel);
    const fUnit = field("wiz-unit", t("wiz.new_unit"), unitIn, t("wiz.new_unit.hint"));
    const fBaud = field("wiz-baud", t("wiz.new_baud"), baudSel);
    const fUser = field("wiz-user", t("ctl.user"), userIn, t("wiz.user.hint"));
    const startBtn = h("button", { type: "button", class: "primary big", "data-testid": "wiz-start" }, t("wiz.start"));
    const syncFields = () => {
      fUnit.el.hidden = settingSel.value !== "unit_id";
      fBaud.el.hidden = settingSel.value !== "baud";
    };
    settingSel.addEventListener("change", syncFields);
    syncFields();
    startBtn.addEventListener("click", async () => {
      const setting = settingSel.value as CommSettingKind;
      const value = setting === "unit_id" ? Number(unitIn.value) : Number(baudSel.value);
      const user = userIn.value.trim();
      fUser.error(user ? null : t("ctl.err.user"));
      if (setting === "unit_id") fUnit.error(Number.isInteger(value) && value >= 1 && value <= 247 ? null : t("wiz.err.unit"));
      if (!user || (setting === "unit_id" && !(Number.isInteger(value) && value >= 1 && value <= 247))) return;
      saveUser(user);
      await act(() => api.wizardStart(device.id, setting, value, user), t("wiz.journaled"));
    });

    async function act<T>(f: () => Promise<T>, ok?: string): Promise<T | undefined> {
      try {
        const r = await f();
        if (ok) status.set(ok, "ok");
        await refresh();
        return r;
      } catch (e) {
        status.set(errorText(e), "error");
        await refresh();
        return undefined;
      }
    }

    async function sendOnce(w: Wizard) {
      const p = await act(() => api.wizardPreview());
      if (!p) return;
      const label =
        w.setting === "unit_id" ? t("wiz.apply.unit", { unit: w.value }) : t("wiz.apply.baud", { baud: fmtNumber(w.value, lang()) });
      const extra = h(
        "div",
        { class: "advice" },
        h("p", null, t("wiz.confirm.change", { old: settingsText(w.old), new: settingsText(w.new) })),
        w.other_devices.length ? h("p", null, t("wiz.baud_others", { list: w.other_devices.join(", ") })) : null,
      );
      const ok = await confirmCommand(p, label, extra);
      if (!ok) {
        await act(() => api.controlCancel(), t("ctl.cancelled"));
        return;
      }
      await act(async () => {
        await api.controlConfirm(p.id);
        const o = await api.controlExecute();
        status.set(t(ackKey(o.ack)), o.ack.kind === "acknowledged" ? "ok" : "error");
      });
    }

    function renderSteps(idx: number, w: Wizard | null) {
      clear(steps);
      STEPS.forEach((s, i) => {
        const state = i < idx ? "done" : i === idx ? "current" : "todo";
        steps.append(
          h(
            "li",
            { class: `wiz-step ${state}`, "aria-current": state === "current" ? "step" : undefined },
            svgShape(state === "done" ? "square" : state === "current" ? "triangle" : "circle", "", "step-shape"),
            t(`wiz.step.${s}`),
            h("span", { class: "sr-only" }, ` (${t(`wiz.state.${state}`)})`),
          ),
        );
      });
      if (w && idx >= STEPS.length) steps.append(h("li", { class: `wiz-step result result-${w.step}` }, t(`wiz.result.${w.step}`)));
    }

    let sig = "";
    const countdown = h("p", { class: "arm-text", "data-testid": "wiz-armed" });
    function renderPanel(s: ControlStatus) {
      const w = s.wizard && s.wizard.device_id === device.id ? s.wizard : null;
      const armed = s.armed_device === device.id;
      const previewed = armed && (s.state === "previewed" || s.state === "confirmed");
      setText(countdown, t("ctl.armed", { s: Math.ceil((s.armed_remaining_ms ?? 0) / 1000), user: s.armed_user ?? "", role: t("ctl.role.commissioner") }));
      // Rebuild only when the step changes, so typing and focus survive the 1 s status polls.
      const now = `${w?.step}|${w?.message}|${armed}|${previewed}|${s.wizard?.device_id}`;
      if (now === sig) return;
      sig = now;
      renderSteps(stepIndex(w, armed, previewed), w);
      preserveFocus(panel, () => {
        clear(panel);
        if (!w) {
          if (s.wizard && s.wizard.device_id !== device.id) {
            panel.append(h("p", { class: "notice" }, t("wiz.other_running", { device: s.wizard.device_name })));
            return;
          }
          panel.append(
            h("h3", null, t("wiz.step.journal")),
            h("p", null, t("wiz.current", { settings: t("wiz.settings.unit_only", { unit: device.unit_id }) })),
            h("div", { class: "form-grid compact" }, fSetting.el, fUnit.el, fBaud.el, fUser.el),
            h("p", { class: "muted small" }, t("wiz.baud_note")),
            startBtn,
          );
          startBtn.dataset.fk = "start";
          return;
        }
        panel.append(h("p", { "data-testid": "wiz-change" }, t("wiz.change", { old: settingsText(w.old), new: settingsText(w.new) })));
        if (w.message) panel.append(h("p", { class: w.step === "passed" ? undefined : "state-bad" }, w.message));
        const btn = (key: string, fk: string, f: () => void, cls = "primary big") => {
          const b = h("button", { type: "button", class: cls, "data-fk": fk, "data-testid": `wiz-${fk}` }, t(key));
          b.addEventListener("click", f);
          return b;
        };
        switch (w.step) {
          case "journaled":
            if (!armed) {
              panel.append(
                h("p", null, t("wiz.arm_hint", { s: Math.round(s.arm_timeout_ms / 1000) })),
                btn("wiz.arm", "arm", () => void act(() => api.controlArm(device.id, w.user, "commissioner"))),
              );
            } else {
              panel.append(countdown, btn("wiz.preview", "preview", () => void sendOnce(w)));
            }
            panel.append(btn("wiz.cancel", "close", () => void act(() => api.wizardClose()), "button"));
            break;
          case "await_power_cycle":
            panel.append(h("p", null, t("wiz.power_cycle")), btn("wiz.power_cycled", "powered", () => void act(() => api.wizardPowerCycled())));
            break;
          case "await_reconnect":
            panel.append(
              h("p", null, t(`ctl.ack.${w.ack?.kind ?? "none"}`)),
              h("p", null, t("wiz.reconnect_hint", { new: settingsText(w.new) })),
              btn("wiz.reconnect", "reconnect", () => void act(() => api.wizardReconnect(), t("wiz.reconnected"))),
            );
            break;
          case "reconnected":
            panel.append(h("p", null, t("wiz.verify_hint")), btn("wiz.verify", "verify", () => void act(() => api.wizardVerify())));
            break;
          case "passed":
            panel.append(
              h("p", { class: "wiz-pass", "data-testid": "wiz-result" }, svgShape("square", "", "step-shape"), t("wiz.pass", { new: settingsText(w.new) })),
              btn("wiz.close", "close", () => void act(() => api.wizardClose()), "button"),
            );
            break;
          case "failed":
          case "needs_verification":
            append(
              panel,
              h("p", { class: "state-bad", "data-testid": "wiz-result" }, svgShape("triangle", "!", "step-shape"), t(`wiz.result.${w.step}`)),
              w.ack ? btn("wiz.revert", "revert", () => void act(() => api.wizardRevert(), t("wiz.reverted")), "big") : null,
              w.ack?.kind === "exception" || w.ack?.kind === "not_sent" ? btn("wiz.close", "close", () => void act(() => api.wizardClose()), "button") : null,
            );
            break;
          case "reverted":
            panel.append(h("p", { "data-testid": "wiz-result" }, t("wiz.result.reverted")), btn("wiz.close", "close", () => void act(() => api.wizardClose()), "button"));
            break;
        }
      });
      // Recovery hint: always visible once the change was sent.
      renderRecovery(w);
    }

    function renderRecovery(w: Wizard | null) {
      clear(recovery);
      recovery.hidden = !(w && w.ack);
      if (w && w.ack) {
        recovery.append(
          h("h3", { id: "rec-h" }, t("wiz.recovery.title")),
          h("p", null, t("wiz.recovery.old", { old: settingsText(w.old) })),
          h("p", null, t("wiz.recovery.new", { new: settingsText(w.new) })),
          h("p", { class: "muted small" }, t("wiz.recovery.hint")),
        );
      }
    }

    async function refresh() {
      try {
        renderPanel(await api.controlStatus());
      } catch (e) {
        status.set(errorText(e), "error");
      }
    }

    async function tick() {
      if (destroyed) return;
      await refresh();
      timer = window.setTimeout(() => void tick(), 1000);
    }
    void tick();
  }

  return {
    el,
    destroy() {
      destroyed = true;
      window.clearTimeout(timer);
    },
  };
}
