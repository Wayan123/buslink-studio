// Training mode (dashboard-ui Req 7): guided exercises (id/en) and fault injection on simulated devices.
import { api, errorText } from "../api.ts";
import { simulatedBanner } from "../components/badges.ts";
import { clear, h } from "../dom.ts";
import { fmtAge } from "../format.ts";
import { lang, t } from "../i18n.ts";
import type { TrainingFault, TrainingStatus } from "../types.ts";
import { preserveFocus, statusLine, type View } from "./view.ts";

const FAULTS: { fault: TrainingFault; seconds: number }[] = [
  { fault: "timeout", seconds: 15 },
  { fault: "crc_error", seconds: 15 },
  { fault: "exception02", seconds: 15 },
  { fault: "stale", seconds: 30 },
  { fault: "wrong_order", seconds: 30 },
];
const UNIT_NAMES: Record<number, string> = { 1: "train.unit.1", 2: "train.unit.2", 3: "train.unit.3", 4: "train.unit.4" };
const EXERCISES = ["read", "timeout", "order", "stale", "alarm", "export"];

export function trainingView(): View {
  const status = statusLine();
  const faultsBox = h("div", { class: "fault-grid" });
  const activeBox = h("ul", { class: "active-faults", "aria-live": "polite" });
  const startBtn = h("button", { type: "button", class: "primary big", "data-testid": "start-training" }, t("connect.training.action"));
  startBtn.addEventListener("click", async () => {
    startBtn.disabled = true;
    try {
      render(await api.trainingStart());
      status.set(t("connect.training.started"), "ok");
    } catch (e) {
      status.set(errorText(e), "error");
    } finally {
      startBtn.disabled = false;
    }
  });

  function render(s: TrainingStatus) {
    startBtn.hidden = s.started;
    preserveFocus(faultsBox, () => {
      clear(faultsBox);
      if (!s.started) {
        faultsBox.append(h("p", { class: "muted" }, t("train.not_started")));
        return;
      }
      for (const unit of s.units) {
        const box = h(
          "section",
          { class: "card fault-card", "aria-label": t(UNIT_NAMES[unit] ?? "train.unit.other", { unit }) },
          h("h4", null, t(UNIT_NAMES[unit] ?? "train.unit.other", { unit })),
        );
        const actions = h("div", { class: "actions wrap" });
        for (const f of FAULTS) {
          const b = h(
            "button",
            { type: "button", "data-fk": `${unit}:${f.fault}`, "data-testid": `fault-${unit}-${f.fault}` },
            t(`train.fault.${f.fault}`, { s: f.seconds }),
          );
          b.addEventListener("click", async () => {
            try {
              render(await api.trainingFault(unit, f.fault, f.seconds));
              status.set(t("train.injected", { fault: t(`train.fault_name.${f.fault}`), unit, s: f.seconds }), "ok");
            } catch (e) {
              status.set(errorText(e), "error");
            }
          });
          actions.append(b);
        }
        const clearBtn = h("button", { type: "button", class: "secondary", "data-fk": `${unit}:clear` }, t("train.clear"));
        clearBtn.addEventListener("click", async () => {
          try {
            render(await api.trainingClear(unit));
            status.set(t("train.cleared", { unit }), "ok");
          } catch (e) {
            status.set(errorText(e), "error");
          }
        });
        actions.append(clearBtn);
        box.append(actions);
        faultsBox.append(box);
      }
    });
    clear(activeBox);
    const L = lang();
    if (!s.faults.length) activeBox.append(h("li", { class: "muted" }, t("train.no_faults")));
    for (const f of s.faults) {
      activeBox.append(h("li", null, t("train.active", { fault: t(`train.fault_name.${f.fault}`), unit: f.unit, left: fmtAge(f.remaining_ms, L) })));
    }
  }

  async function refresh() {
    try {
      render(await api.trainingStatus());
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }
  const timer = window.setInterval(() => void refresh(), 1000);

  const el = h(
    "div",
    { class: "view training" },
    h("h2", null, t("nav.training")),
    simulatedBanner(),
    h("p", null, t("train.intro")),
    startBtn,
    status.el,
    h("h3", null, t("train.faults")),
    h("p", { class: "muted small" }, t("train.faults.note")),
    faultsBox,
    h("h3", null, t("train.active_title")),
    activeBox,
    h("h3", null, t("train.exercises")),
    h(
      "ol",
      { class: "exercises" },
      ...EXERCISES.map((x) =>
        h(
          "li",
          { class: "card exercise" },
          h("h4", null, t(`train.ex.${x}.title`)),
          h("p", null, t(`train.ex.${x}.steps`)),
          h("p", { class: "muted small" }, t(`train.ex.${x}.learn`)),
        ),
      ),
    ),
  );
  void refresh();
  return { el, destroy: () => window.clearInterval(timer) };
}
