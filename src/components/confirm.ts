// Confirmation dialog for one command (safety rule 3, ui.md "Control panel"): device, unit ID, register
// (doc address + PDU offset), function, old → new in engineering units, raw value and the encoded frame.
// The confirm button names the action and value. Focus is trapped by the modal dialog and restored on close.
import { h } from "../dom.ts";
import { fmtNumber, hex4 } from "../format.ts";
import { lang, t, tx } from "../i18n.ts";
import type { Preview } from "../types.ts";
import { simulatedBanner } from "./badges.ts";

export function fmtValue(v: number | null, p: Preview): string {
  if (v === null) return t("ctl.unknown");
  const s = fmtNumber(v, lang(), p.decimals ?? undefined);
  return p.unit ? `${s} ${p.unit}` : s;
}

/** Resolves true only when the operator presses the action button. */
export function confirmCommand(p: Preview, actionLabel: string, extra?: HTMLElement | null): Promise<boolean> {
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const fc = `FC${p.function.toString().padStart(2, "0")}`;
  const row = (k: string, v: string | HTMLElement, testid?: string) =>
    [h("dt", null, k), h("dd", testid ? { "data-testid": testid } : null, v)];
  const oldText = p.old_value !== null ? fmtValue(p.old_value, p) : `${t("ctl.unknown")}${p.old_error ? ` (${p.old_error})` : ""}`;
  const oldRaw = p.old_raw ? ` · raw ${p.old_raw.map(hex4).join(" ")}` : "";
  const cancel = h("button", { type: "button", "data-testid": "confirm-cancel" }, t("ctl.cancel"));
  const go = h("button", { type: "button", class: "primary big", "data-testid": "confirm-send" }, actionLabel);
  const dlg = h(
    "dialog",
    { class: "confirm-dialog", "aria-labelledby": "confirm-title", "data-testid": "confirm-dialog" },
    h("h2", { id: "confirm-title" }, t("ctl.confirm.title")),
    p.simulated ? simulatedBanner() : null,
    h(
      "dl",
      { class: "confirm-list" },
      ...row(t("ctl.confirm.device"), p.device_name),
      ...row(t("ctl.confirm.unit"), String(p.unit_id)),
      ...row(
        t("ctl.confirm.register"),
        t("ctl.confirm.register_value", { name: tx(p.register_name), doc: p.doc_address, offset: p.address, table: p.table }),
      ),
      ...row(t("ctl.confirm.function"), `${fc} ${p.function_name}`),
      ...row(t("ctl.confirm.old"), `${oldText}${oldRaw}`),
      ...row(t("ctl.confirm.new"), h("strong", null, fmtValue(p.new_value, p)), "confirm-new"),
      ...row(t("ctl.confirm.raw"), p.raw_registers.map((r) => `${hex4(r)} (${r})`).join(" ")),
      ...row(t("ctl.confirm.frame"), h("code", { class: "mono" }, p.frame_hex), "confirm-frame"),
      ...row(t("ctl.confirm.readback"), p.readback_register),
      ...row(t("ctl.confirm.risk"), t(`ctl.risk.${p.risk}`)),
    ),
    extra ?? null,
    h("p", { class: "muted small" }, t("ctl.confirm.once")),
    h("div", { class: "actions dialog-actions" }, cancel, go),
  );
  document.body.append(dlg);
  return new Promise((resolve) => {
    let done = false;
    const finish = (ok: boolean) => {
      if (done) return;
      done = true;
      dlg.close();
      dlg.remove();
      opener?.focus();
      resolve(ok);
    };
    cancel.addEventListener("click", () => finish(false));
    go.addEventListener("click", () => finish(true));
    dlg.addEventListener("cancel", (e) => {
      e.preventDefault();
      finish(false);
    });
    dlg.showModal();
    cancel.focus(); // the safe choice has the initial focus
  });
}
