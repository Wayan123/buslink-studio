// MQTT panel (lan-mqtt Req 4.2): broker settings, write-only credentials, Test connection,
// enable/disable and the queue. Telemetry out only: nothing here subscribes to anything.
import { api, errorText } from "../api.ts";
import { clear, h, setText } from "../dom.ts";
import { fmtLocalDateTime } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { refreshSharing } from "../sharing.ts";
import type { MqttConfig, MqttStatus } from "../types.ts";
import { field, statusLine, type View } from "./view.ts";

function num(v: string, fallback: number): number {
  const n = Number(v);
  return Number.isFinite(n) ? n : fallback;
}

export function mqttView(): View {
  const status = statusLine();
  let config: MqttConfig | null = null;
  let running = false;

  const urlIn = h("input", { type: "text", inputmode: "url", autocomplete: "off", spellcheck: false, placeholder: "mqtts://broker.example.org:8883" });
  const siteIn = h("input", { type: "text", autocomplete: "off" });
  const gwIn = h("input", { type: "text", autocomplete: "off" });
  const idIn = h("input", { type: "text", autocomplete: "off" });
  const userIn = h("input", { type: "text", autocomplete: "off" });
  const pwIn = h("input", { type: "password", autocomplete: "new-password", "data-testid": "mqtt-password" });
  const caIn = h("textarea", { rows: 4, spellcheck: false, class: "mono" });
  const insecure = h("input", { type: "checkbox" });
  const expiryIn = h("input", { type: "number", min: 1, max: 604800 });
  const outboxIn = h("input", { type: "number", min: 100, max: 1000000 });
  const intervalIn = h("input", { type: "number", min: 0, max: 3600000 });
  const fUrl = field("mqtt-url", t("mqtt.url"), urlIn, t("mqtt.url.hint"));
  const fSite = field("mqtt-site", t("mqtt.site"), siteIn);
  const fGw = field("mqtt-gw", t("mqtt.gateway"), gwIn);
  const fId = field("mqtt-id", t("mqtt.client_id"), idIn);
  const fUser = field("mqtt-user", t("mqtt.username"), userIn);
  const pwHint = h("span", { class: "field-hint" });
  const fPw = field("mqtt-pw", t("mqtt.password"), pwIn);
  const fCa = field("mqtt-ca", t("mqtt.ca"), caIn as unknown as HTMLInputElement, t("mqtt.ca.hint"));
  const insecureWarn = h("p", { class: "advice", role: "note", hidden: true }, t("mqtt.insecure.warn"));
  const removePw = h("button", { type: "button", hidden: true }, t("mqtt.password.remove"));
  const storeNote = h("p", { class: "muted small" });

  const saveBtn = h("button", { type: "submit", class: "primary", "data-testid": "mqtt-save" }, t("mqtt.save"));
  const testBtn = h("button", { type: "button", "data-testid": "mqtt-test" }, t("mqtt.test"));
  const toggleBtn = h("button", { type: "button", "data-testid": "mqtt-toggle" }, t("mqtt.enable"));

  const form = h(
    "form",
    { class: "card form-grid", "aria-labelledby": "mqtt-form-h", novalidate: true },
    h("h3", { id: "mqtt-form-h" }, t("mqtt.settings")),
    fUrl.el,
    h("label", { class: "check" }, insecure, ` ${t("mqtt.insecure")}`),
    insecureWarn,
    fSite.el,
    fGw.el,
    fId.el,
    fUser.el,
    fPw.el,
    pwHint,
    removePw,
    storeNote,
    fCa.el,
    h(
      "details",
      null,
      h("summary", null, t("mqtt.advanced")),
      field("mqtt-expiry", t("mqtt.expiry"), expiryIn).el,
      field("mqtt-outbox", t("mqtt.outbox_max"), outboxIn).el,
      field("mqtt-interval", t("mqtt.interval"), intervalIn, t("mqtt.interval.hint")).el,
    ),
    h("div", { class: "actions" }, saveBtn, testBtn),
  );

  // ---- status ----
  const stateBadge = h("span", { class: "badge", "data-testid": "mqtt-state" });
  const rows = h("dl", { class: "kv" });
  const lastError = h("p", { class: "error-text", role: "status", "data-testid": "mqtt-error" });
  const statusBox = h(
    "section",
    { class: "card", "aria-labelledby": "mqtt-status-h" },
    h("h3", { id: "mqtt-status-h" }, t("mqtt.status")),
    h("p", null, stateBadge),
    rows,
    lastError,
    h("div", { class: "actions" }, toggleBtn),
  );

  function renderStatus(s: MqttStatus) {
    running = s.state !== "off";
    setText(stateBadge, t(`mqtt.state.${s.state}`));
    toggleBtn.textContent = running ? t("mqtt.disable") : t("mqtt.enable");
    toggleBtn.classList.toggle("danger", running);
    const L = lang();
    const items: [string, string][] = [
      ["mqtt.stat.depth", `${s.outbox_depth} / ${s.outbox_max}`],
      ["mqtt.stat.published", String(s.published)],
      ["mqtt.stat.dropped", String(s.dropped_overflow)],
      ["mqtt.stat.expired", String(s.expired)],
      ["mqtt.stat.throttled", String(s.throttled)],
      ["mqtt.stat.lagged", String(s.lagged)],
      ["mqtt.stat.rejected", String(s.rejected)],
      ["mqtt.stat.reconnects", String(s.reconnects)],
      ["mqtt.stat.last_ack", s.last_ack_ms === null ? t("mqtt.none") : fmtLocalDateTime(s.last_ack_ms, L)],
    ];
    clear(rows);
    for (const [k, v] of items) rows.append(h("dt", null, t(k)), h("dd", null, v));
    setText(lastError, s.last_error ? `${t("mqtt.stat.last_error")}: ${s.last_error}` : "");
  }

  function fill(c: MqttConfig, hasPw: boolean, store: string, persistent: boolean) {
    config = c;
    urlIn.value = c.broker_url;
    siteIn.value = c.site;
    gwIn.value = c.gateway;
    idIn.value = c.client_id;
    userIn.value = c.username;
    caIn.value = c.ca_pem;
    insecure.checked = c.allow_insecure;
    insecureWarn.hidden = !c.allow_insecure;
    expiryIn.value = String(c.expiry_s);
    outboxIn.value = String(c.outbox_max);
    intervalIn.value = String(c.min_interval_ms);
    setPasswordState(hasPw);
    setText(storeNote, persistent ? t("mqtt.store.persistent", { store }) : t("mqtt.store.memory"));
  }

  function setPasswordState(has: boolean) {
    pwIn.value = "";
    pwIn.placeholder = has ? t("mqtt.password.stored") : "";
    setText(pwHint, has ? t("mqtt.password.stored") : t("mqtt.password.none"));
    removePw.hidden = !has;
  }

  function read(): MqttConfig {
    const base = config;
    return {
      broker_url: urlIn.value.trim(),
      site: siteIn.value.trim(),
      gateway: gwIn.value.trim(),
      client_id: idIn.value.trim(),
      username: userIn.value.trim(),
      ca_pem: caIn.value.trim(),
      allow_insecure: insecure.checked,
      expiry_s: num(expiryIn.value, base?.expiry_s ?? 3600),
      keep_alive_s: base?.keep_alive_s ?? 30,
      outbox_max: num(outboxIn.value, base?.outbox_max ?? 10000),
      min_interval_ms: num(intervalIn.value, base?.min_interval_ms ?? 1000),
    };
  }

  /** A validation message from Rust is shown next to the field it names when we can tell, else in the status line. */
  function showError(message: string) {
    for (const f of [fUrl, fSite, fGw, fId, fUser, fCa]) f.error(null);
    if (/^broker URL/i.test(message)) fUrl.error(message);
    else if (/^client_id/.test(message)) fId.error(message);
    else if (/^ca_pem/.test(message)) fCa.error(message);
    else if (/^site/.test(message)) fSite.error(message);
    else if (/^gateway/.test(message)) fGw.error(message);
    else if (/^username/.test(message)) fUser.error(message);
    status.set(message, "error");
  }

  async function savePassword(): Promise<boolean> {
    if (!pwIn.value) return true;
    await api.mqttSetPassword(pwIn.value);
    setPasswordState(true);
    return true;
  }

  insecure.addEventListener("change", () => {
    insecureWarn.hidden = !insecure.checked;
  });
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    saveBtn.disabled = true;
    try {
      const saved = await api.mqttSave(read());
      await savePassword();
      config = saved;
      showError("");
      status.set(t("mqtt.saved") + (running ? ` ${t("mqtt.restart_needed")}` : ""), "ok");
    } catch (err) {
      showError(errorText(err));
    } finally {
      saveBtn.disabled = false;
    }
  });
  removePw.addEventListener("click", async () => {
    try {
      await api.mqttClearPassword();
      setPasswordState(false);
      status.set(t("mqtt.password.removed"), "ok");
    } catch (err) {
      status.set(errorText(err), "error");
    }
  });
  testBtn.addEventListener("click", async () => {
    testBtn.disabled = true;
    status.set(t("mqtt.testing"));
    try {
      // A typed password is used for this test only; it is not saved until Save settings.
      const r = await api.mqttTest(read(), pwIn.value || null);
      status.set(r.ok ? t("mqtt.test.ok", { detail: r.detail, ms: r.elapsed_ms }) : t("mqtt.test.fail", { detail: r.detail }), r.ok ? "ok" : "error");
    } catch (err) {
      status.set(errorText(err), "error");
    } finally {
      testBtn.disabled = false;
    }
  });
  toggleBtn.addEventListener("click", async () => {
    toggleBtn.disabled = true;
    try {
      if (running) {
        renderStatus(await api.mqttStop());
        status.set(t("mqtt.stopped"), "ok");
      } else {
        // Start with what is on screen: save first so the running publisher uses the same settings.
        const saved = await api.mqttSave(read());
        await savePassword();
        config = saved;
        renderStatus(await api.mqttStart());
        status.set(t("mqtt.started"), "ok");
      }
    } catch (err) {
      showError(errorText(err));
    } finally {
      toggleBtn.disabled = false;
      void refreshSharing();
    }
  });

  async function poll() {
    try {
      renderStatus(await api.mqttStatus());
    } catch {
      /* the next poll retries */
    }
  }
  void api
    .mqttGet()
    .then((v) => {
      fill(v.config, v.has_password, v.secret_store, v.secret_persistent);
      renderStatus(v.status);
    })
    .catch((e) => status.set(errorText(e), "error"));
  const timer = window.setInterval(() => void poll(), 2000);

  return {
    el: h(
      "div",
      { class: "view mqtt" },
      h("header", { class: "view-head" }, h("h2", null, t("mqtt.title"))),
      h("p", null, t("mqtt.intro")),
      h("p", { class: "advice", role: "note" }, t("mqtt.telemetry_only")),
      statusBox,
      form,
      status.el,
    ),
    destroy() {
      window.clearInterval(timer);
    },
  };
}
