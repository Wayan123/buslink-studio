// Control panel for simulated devices (commissioning-control Req 1, 2; ui.md "Control panel").
// Every command: arm → preview (Rust) → confirm dialog → single send → read-back → process feedback.
// "Write acknowledged" and "process reached target" are separate fields; nothing here is green.
import { api, errorText, hasBackend } from "../api.ts";
import { qualityBadge, simulatedBanner } from "../components/badges.ts";
import { confirmCommand } from "../components/confirm.ts";
import { actionLabel, ackKey, checkBounds, secondsLeft, splitCommands, valueText } from "../control.ts";
import { announce, append, clear, h, setText } from "../dom.ts";
import { fmtAge, fmtLocalTime, fmtNumber } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import { loadProfiles, profileOf } from "../profiles.ts";
import { store } from "../store.ts";
import type { ControlStatus, DeviceView, Profile, ProfileCommand, Role } from "../types.ts";
import { field, preserveFocus, select, statusLine, type View } from "./view.ts";

const POLL_MS = 500;
const USER_KEY = "buslink.user";

export function savedUser(): string {
  try {
    return localStorage.getItem(USER_KEY) ?? "";
  } catch {
    return "";
  }
}

export function saveUser(u: string): void {
  try {
    localStorage.setItem(USER_KEY, u);
  } catch {
    /* not persisted */
  }
}

export function controlView(params: { device?: string } = {}): View {
  const status = statusLine();
  const body = h("div", { class: "control-body" });
  const el = h("div", { class: "view control" }, h("header", { class: "view-head" }, h("h2", null, t("ctl.title"))), status.el, body);
  if (!hasBackend()) {
    body.append(h("p", { class: "notice" }, t("ctl.no_backend")));
    return { el };
  }
  let timer = 0;
  let destroyed = false;
  let last: ControlStatus | null = null;
  let busy = false;

  void loadProfiles().then(() => {
    if (destroyed) return;
    const entry = params.device ? store.device(params.device) : store.devices().find((d) => d.link.simulated && profileOf(d.device)?.simulated_only);
    const profile = entry ? profileOf(entry.device) : undefined;
    if (!entry || !profile) {
      body.append(h("p", { class: "notice" }, t("ctl.pick_device")), devicePicker());
      return;
    }
    if (!entry.link.simulated || !profile.simulated_only) {
      body.append(h("p", { class: "notice" }, t("ctl.simulated_only")));
      return;
    }
    build(entry.device, profile);
  });

  function devicePicker(): HTMLElement {
    const list = h("ul", { class: "link-list" });
    for (const { device, link } of store.devices()) {
      if (!link.simulated || !profileOf(device)?.simulated_only) continue;
      list.append(h("li", null, h("a", { href: `#/control/${encodeURIComponent(device.id)}` }, device.name)));
    }
    if (!list.childElementCount) list.append(h("li", { class: "muted" }, t("ctl.none_available")));
    return list;
  }

  function build(device: DeviceView, profile: Profile) {
    const L = lang();
    const regOf = (id: string) => profile.registers.find((r) => r.id === id);
    const cmds = profile.commands ?? [];
    const { stops, others } = splitCommands(cmds);

    // ---- arming ----
    const armBox = h("section", { class: "card arm-box", "aria-labelledby": "arm-h", "data-testid": "arm-box" });
    const userIn = h("input", { type: "text", value: savedUser(), autocomplete: "username", maxlength: 64, "data-testid": "arm-user", "data-fk": "user" });
    const roleSel = select(
      [
        ["operator", t("ctl.role.operator")],
        ["commissioner", t("ctl.role.commissioner")],
      ],
      "operator",
    );
    const userField = field("arm-user", t("ctl.user"), userIn);
    const roleField = field("arm-role", t("ctl.role"), roleSel, t("ctl.role.hint"));
    const armBtn = h("button", { type: "button", class: "primary big", "data-testid": "arm" }, t("ctl.arm"));
    const disarmBtn = h("button", { type: "button", class: "big", "data-testid": "disarm" }, t("ctl.disarm"));
    const armText = h("p", { class: "arm-text", "data-testid": "arm-text" });
    armBtn.addEventListener("click", async () => {
      const u = userIn.value.trim();
      if (!u) {
        userField.error(t("ctl.err.user"));
        userIn.focus();
        return;
      }
      userField.error(null);
      saveUser(u);
      try {
        render(await api.controlArm(device.id, u, roleSel.value as Role));
        announce(t("ctl.announce.armed", { device: device.name }));
      } catch (e) {
        status.set(errorText(e), "error");
      }
    });
    disarmBtn.addEventListener("click", async () => {
      try {
        render(await api.controlDisarm("operator"));
        announce(t("ctl.announce.disarmed"));
      } catch (e) {
        status.set(errorText(e), "error");
      }
    });

    // ---- software stop (always visible while armed) ----
    const stopBox = h("section", { class: "card stop-box", "aria-labelledby": "stop-h" }, h("h3", { id: "stop-h" }, t("ctl.stop.title")));
    const stopButtons: HTMLButtonElement[] = [];
    for (const c of stops) {
      const b = h("button", { type: "button", class: "big stop-button", "data-testid": `cmd-${c.id}` }, tx(c.label));
      b.addEventListener("click", () => void run(c, undefined, tx(c.label)));
      stopButtons.push(b);
      stopBox.append(b);
    }
    stopBox.append(h("p", { class: "advice" }, t("ctl.estop_advice")));

    // ---- other commands ----
    const cmdBox = h("section", { class: "card cmd-box", "aria-labelledby": "cmd-h" }, h("h3", { id: "cmd-h" }, t("ctl.commands")));
    const cmdButtons: HTMLButtonElement[] = [];
    for (const c of others) {
      const reg = regOf(c.register);
      if (c.kind === "setpoint" && reg) {
        const w = reg.write;
        const input = h("input", {
          type: "number",
          inputmode: "decimal",
          min: w?.min,
          max: w?.max,
          step: w?.step ?? "any",
          "data-testid": `value-${c.id}`,
        });
        const hint = w?.min !== undefined && w.max !== undefined
          ? t("ctl.bounds", { min: valueText(w.min, reg, L), max: valueText(w.max, reg, L), step: w.step !== undefined ? fmtNumber(w.step, L) : "—" })
          : "";
        const f = field(`val-${c.id}`, `${tx(reg.name)}${reg.unit ? ` (${reg.unit})` : ""}`, input, hint);
        const b = h("button", { type: "button", class: "primary", "data-testid": `cmd-${c.id}` }, tx(c.label));
        const relabel = () => {
          const v = input.value === "" ? undefined : Number(input.value);
          setText(b, actionLabel(tx(c.label), c, v, reg, lang()));
        };
        input.addEventListener("input", relabel);
        b.addEventListener("click", () => {
          const v = input.value === "" ? Number.NaN : Number(input.value);
          const err = checkBounds(reg, v);
          if (err) {
            f.error(t(err.key, Object.fromEntries(Object.entries(err).filter(([k]) => k !== "key")) as Record<string, string | number>));
            input.focus();
            return;
          }
          f.error(null);
          void run(c, v, actionLabel(tx(c.label), c, v, reg, lang()));
        });
        cmdButtons.push(b);
        cmdBox.append(h("div", { class: "cmd-row" }, f.el, b));
      } else {
        const b = h("button", { type: "button", "data-testid": `cmd-${c.id}` }, tx(c.label));
        b.addEventListener("click", () => void run(c, undefined, tx(c.label)));
        cmdButtons.push(b);
        cmdBox.append(h("div", { class: "cmd-row" }, b, h("span", { class: "muted small" }, t("ctl.sets", { reg: tx(reg?.name), value: fmtNumber(c.value ?? null, L) }))));
      }
    }

    // ---- result: sent / acknowledged / process feedback ----
    const result = h("section", { class: "card result-box", "aria-labelledby": "res-h", "aria-live": "polite", "data-testid": "result" });
    // ---- live values of this device ----
    const live = h("section", { class: "card live-box", "aria-labelledby": "live-h" });

    body.append(
      simulatedBanner(),
      h("p", { class: "device-line" }, t("ctl.device_line", { device: device.name, unit: device.unit_id, profile: `${device.profile_id} ${device.profile_version}` })),
      armBox,
      stopBox,
      cmdBox,
      result,
      live,
      h("p", null, h("a", { href: "#/audit" }, t("ctl.see_audit"))),
    );

    async function run(c: ProfileCommand, value: number | undefined, label: string) {
      if (busy) return;
      busy = true;
      try {
        const p = await api.controlPreview(device.id, { kind: "named", command: c.id, value });
        const ok = await confirmCommand(p, label);
        if (!ok) {
          render(await api.controlCancel());
          status.set(t("ctl.cancelled"), "info");
          return;
        }
        await api.controlConfirm(p.id);
        status.set(t("ctl.sending"), "info");
        const o = await api.controlExecute();
        status.set(t(ackKey(o.ack)), o.ack.kind === "acknowledged" ? "ok" : "error");
        render(await api.controlStatus());
      } catch (e) {
        status.set(errorText(e), "error");
        try {
          render(await api.controlStatus());
        } catch {
          /* the next poll retries */
        }
      } finally {
        busy = false;
      }
    }

    let armSig = "";
    function renderArm(s: ControlStatus) {
      const mine = s.armed_device === device.id;
      armBox.classList.toggle("armed", mine);
      el.classList.toggle("is-armed", mine);
      const remaining = t("ctl.armed", { s: secondsLeft(s.armed_remaining_ms), user: s.armed_user ?? "", role: t(`ctl.role.${s.armed_role ?? "operator"}`) });
      // Rebuild only when the arming situation changes; the countdown updates in place (typing is never interrupted).
      const sig = `${mine}|${s.armed_device}|${s.disarm_reason}|${s.arm_timeout_ms}`;
      if (sig === armSig) {
        if (mine) setText(armText, remaining);
        stopBox.hidden = !mine;
        for (const b of [...cmdButtons, ...stopButtons]) b.disabled = !mine || busy;
        return;
      }
      armSig = sig;
      preserveFocus(armBox, () => {
        clear(armBox);
        armBox.append(h("h3", { id: "arm-h" }, t("ctl.arm.title")));
        if (mine) {
          armText.textContent = remaining;
          armBox.append(armText, h("p", { class: "muted small" }, t("ctl.armed.idle", { s: Math.round(s.arm_timeout_ms / 1000) })), disarmBtn);
          disarmBtn.dataset.fk = "disarm";
        } else {
          append(
            armBox,
            s.armed_device ? h("p", { class: "notice" }, t("ctl.armed_other", { device: s.armed_device })) : h("p", null, t("ctl.monitoring")),
            s.disarm_reason ? h("p", { class: "muted", "data-testid": "disarm-reason" }, t(`ctl.disarm.${s.disarm_reason}`)) : null,
            h("div", { class: "form-grid compact" }, userField.el, roleField.el),
            h("p", { class: "muted small" }, t("ctl.arm.hint", { s: Math.round(s.arm_timeout_ms / 1000) })),
            s.armed_device ? disarmBtn : armBtn,
          );
          armBtn.dataset.fk = "arm";
        }
      });
      stopBox.hidden = !mine;
      for (const b of [...cmdButtons, ...stopButtons]) b.disabled = !mine || busy;
    }

    function renderResult(s: ControlStatus) {
      const lc = s.last && s.last.preview.device_id === device.id ? s.last : null;
      preserveFocus(result, () => {
        clear(result);
        result.append(h("h3", { id: "res-h" }, t("ctl.result.title")));
        if (!lc) {
          result.append(h("p", { class: "muted" }, t("ctl.result.none")));
          return;
        }
        const p = lc.preview;
        const reg = regOf(p.register_id);
        const fb = lc.feedback;
        const fbReg = fb ? regOf(fb.register) : undefined;
        const o = lc.outcome;
        const dl = h("dl", { class: "result-list" });
        const add = (k: string, v: string | HTMLElement, cls?: string, testid?: string) =>
          dl.append(h("dt", null, k), h("dd", { class: cls, "data-testid": testid }, v));
        add(t("ctl.result.command"), `${p.command_label ? tx(p.command_label) : p.register_id} → ${valueText(p.new_value, reg, lang())}`);
        add(t("ctl.result.sent"), lc.sent ? `${t("ctl.yes")} · ${o?.request_hex ?? ""}` : t("ctl.result.not_sent"), "mono", "res-sent");
        add(t("ctl.result.ack"), o ? t(ackKey(o.ack)) : t("ctl.pending"), o && o.ack.kind !== "acknowledged" ? "state-bad" : undefined, "res-ack");
        add(
          t("ctl.result.readback"),
          o ? `${t(`ctl.readback.${o.readback}`)}${o.readback_value !== null ? ` · ${valueText(o.readback_value, regOf(p.readback_register), lang())}` : ""}${o.readback_error ? ` · ${o.readback_error}` : ""}` : t("ctl.pending"),
          o && o.readback !== "verified" ? "state-unknown" : undefined,
          "res-readback",
        );
        if (fb) {
          const target = fb.bit !== null ? t("ctl.fb.bit", { bit: fb.bit, value: fb.target }) : `${valueText(fb.target, fbReg, lang())} ± ${fmtNumber(fb.tolerance, lang())}`;
          const now = Date.now();
          const left = fb.state === "pending" || fb.state === "feedback_unavailable" ? ` · ${t("ctl.fb.left", { t: fmtAge(Math.max(0, fb.deadline_ms - now), lang()) })}` : "";
          const lastV = fb.last_value !== null ? ` · ${t("ctl.fb.now", { v: fb.bit !== null ? String(fb.last_value) : valueText(fb.last_value, fbReg, lang()) })}` : "";
          add(
            t("ctl.result.feedback"),
            `${t(`ctl.fb.${fb.state}`)} · ${t("ctl.fb.target", { reg: tx(fbReg?.name) || fb.register, target })}${lastV}${left}`,
            fb.state === "not_reached" || fb.state === "feedback_unavailable" ? "state-bad" : undefined,
            "res-feedback",
          );
        } else if (lc.sent) {
          add(t("ctl.result.feedback"), t("ctl.fb.none"), "muted", "res-feedback");
        }
        result.append(dl);
      });
    }

    function renderLive() {
      clear(live);
      live.append(h("h3", { id: "live-h" }, t("ctl.live")));
      const tbl = h("table", null, h("thead", null, h("tr", null, h("th", null, t("ctl.live.tag")), h("th", null, t("ctl.live.value")), h("th", null, t("ctl.live.quality")))));
      const tb = h("tbody");
      for (const r of profile.registers) {
        const tag = store.tags.get(`${device.id}/${r.id}`);
        const v = tag?.value ?? null;
        const q = tag?.quality ?? null;
        tb.append(
          h(
            "tr",
            { class: q && q !== "GOOD" ? "dim" : undefined },
            h("th", { scope: "row" }, tx(r.name)),
            h("td", { class: "num" }, v === null ? "—" : valueText(v, r, lang())),
            h("td", null, q ? (qualityBadge(q, tag?.exception ?? null, fmtAge(tag?.age_ms ?? null, lang())) ?? `GOOD · ${fmtAge(tag?.age_ms ?? null, lang())}`) : "—", ` ${tag ? fmtLocalTime(tag.value_ts_ms, lang()) : ""}`),
          ),
        );
      }
      tbl.append(tb);
      live.append(h("div", { class: "table-wrap" }, tbl));
    }

    function render(s: ControlStatus) {
      if (!s) return;
      const was = last?.armed_device === device.id;
      last = s;
      renderArm(s);
      renderResult(s);
      if (was && s.armed_device !== device.id && s.disarm_reason) announce(t(`ctl.disarm.${s.disarm_reason}`));
    }

    async function tick() {
      if (destroyed) return;
      try {
        render(await api.controlStatus());
      } catch (e) {
        status.set(errorText(e), "error");
      }
      renderLive();
      timer = window.setTimeout(() => void tick(), POLL_MS);
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
