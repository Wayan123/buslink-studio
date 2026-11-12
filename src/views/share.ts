// Share panel (lan-mqtt Req 4.1): turn the read-only LAN view on and off, show the link as
// text and QR, the certificate fingerprint and who is watching. Off at every start.
import { api, errorText } from "../api.ts";
import { clear, h, setText } from "../dom.ts";
import { fmtLocalTime } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { refreshSharing } from "../sharing.ts";
import type { LanDetails, LanInterfaces, LanStatus } from "../types.ts";
import { field, select, statusLine, type View } from "./view.ts";

const QUIET = 4;

/** Draw a QR matrix (rows of '0'/'1') on a canvas: dark modules on white with a quiet zone. */
export function drawQr(canvas: HTMLCanvasElement, rows: string[], size: number, scale = 5): void {
  const px = (size + QUIET * 2) * scale;
  canvas.width = px;
  canvas.height = px;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.fillStyle = "#FFFFFF";
  ctx.fillRect(0, 0, px, px);
  ctx.fillStyle = "#000000";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === "1") ctx.fillRect((x + QUIET) * scale, (y + QUIET) * scale, scale, scale);
    }
  });
}

export function shareView(): View {
  const status = statusLine();
  let ifaces: LanInterfaces = { interfaces: [], default_port: 8443 };

  // ---- off: choose interface and port ----
  const ifaceSel = select([]);
  const portIn = h("input", { type: "number", inputmode: "numeric", min: 1024, max: 65535, value: 8443 });
  const fIface = field("share-iface", t("share.iface"), ifaceSel, t("share.private_only"));
  const fPort = field("share-port", t("share.port"), portIn, t("share.port.hint"));
  const noIfaces = h("p", { class: "advice", role: "note", hidden: true }, t("share.no_interfaces"));
  const startBtn = h("button", { type: "submit", class: "primary", "data-testid": "share-start" }, t("share.start"));
  const offBox = h(
    "form",
    { class: "card form-grid", "aria-labelledby": "share-off-h", novalidate: true },
    h("h3", { id: "share-off-h" }, t("share.off")),
    noIfaces,
    fIface.el,
    fPort.el,
    h("div", { class: "actions" }, startBtn),
  );

  // ---- on: link, QR, clients ----
  const urlIn = h("input", { type: "text", readonly: true, "aria-label": t("share.url"), "data-testid": "share-url", class: "mono wide" });
  const copyBtn = h("button", { type: "button" }, t("share.copy"));
  const qr = h("canvas", { role: "img", "aria-label": t("share.qr_label"), class: "qr", "data-testid": "share-qr" });
  const fp = h("code", { class: "mono" });
  const clientsBox = h("div", { class: "table-wrap" });
  const counters = h("p", { class: "muted small" });
  const stopBtn = h("button", { type: "button", class: "danger", "data-testid": "share-stop" }, t("share.stop"));
  const onBox = h(
    "section",
    { class: "card", "aria-labelledby": "share-on-h", hidden: true },
    h("h3", { id: "share-on-h" }, t("share.on")),
    h("p", { class: "advice", role: "note" }, t("share.url_secret")),
    h("label", { for: "share-url" }, t("share.url")),
    h("div", { class: "actions" }, urlIn, copyBtn),
    h("p", { class: "muted small" }, t("share.cert_hint")),
    qr,
    h("p", null, t("share.fingerprint"), " ", fp),
    h("h4", null, t("share.clients")),
    clientsBox,
    counters,
    h("div", { class: "actions" }, stopBtn),
  );
  urlIn.id = "share-url";

  function renderClients(d: LanDetails) {
    clear(clientsBox);
    if (!d.clients.length) {
      clientsBox.append(h("p", { class: "muted" }, t("share.clients.none", { max: d.max_clients })));
      return;
    }
    const L = lang();
    clientsBox.append(
      h(
        "table",
        null,
        h("thead", null, h("tr", null, h("th", { scope: "col" }, t("share.clients.addr")), h("th", { scope: "col" }, t("share.clients.since")))),
        h("tbody", null, ...d.clients.map((c) => h("tr", null, h("td", null, c.addr), h("td", null, fmtLocalTime(c.since_ms, L))))),
      ),
    );
  }

  function apply(st: LanStatus) {
    const d = st.details;
    offBox.hidden = st.running;
    onBox.hidden = !st.running;
    if (!st.running || !d) return;
    if (urlIn.value !== d.url) urlIn.value = d.url;
    setText(fp, d.fingerprint);
    if (d.qr) drawQr(qr, d.qr.rows, d.qr.size);
    renderClients(d);
    setText(counters, t("share.counters", { rejected: d.rejected, limited: d.rate_limited, refused: d.refused_connections + d.refused_clients }));
  }

  async function poll() {
    try {
      apply(await api.lanStatus());
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }

  offBox.addEventListener("submit", async (e) => {
    e.preventDefault();
    fIface.error(null);
    fPort.error(null);
    const port = Number(portIn.value);
    let bad = false;
    if (!ifaceSel.value) {
      fIface.error(t("share.err.iface"));
      bad = true;
    }
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      fPort.error(t("share.err.port"));
      bad = true;
    }
    if (bad) return;
    startBtn.disabled = true;
    try {
      apply(await api.lanStart(ifaceSel.value, port));
      status.set(t("share.started"), "ok");
      urlIn.focus();
      urlIn.select();
    } catch (err) {
      status.set(errorText(err), "error");
    } finally {
      startBtn.disabled = false;
      void refreshSharing();
    }
  });
  stopBtn.addEventListener("click", async () => {
    stopBtn.disabled = true;
    try {
      apply(await api.lanStop());
      status.set(t("share.stopped"), "ok");
      startBtn.focus();
    } catch (err) {
      status.set(errorText(err), "error");
    } finally {
      stopBtn.disabled = false;
      void refreshSharing();
    }
  });
  copyBtn.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(urlIn.value);
      status.set(t("share.copied"), "ok");
    } catch {
      urlIn.select();
      status.set(t("share.copy_failed"), "error");
    }
  });

  void api
    .lanInterfaces()
    .then((i) => {
      ifaces = i;
      clear(ifaceSel);
      for (const f of i.interfaces) ifaceSel.append(h("option", { value: f.ip }, `${f.name} — ${f.ip}`));
      portIn.value = String(i.default_port);
      noIfaces.hidden = i.interfaces.length > 0;
      startBtn.disabled = i.interfaces.length === 0;
    })
    .catch((e) => status.set(errorText(e), "error"));
  void poll();
  const timer = window.setInterval(() => void poll(), 2000);
  void ifaces;

  return {
    el: h(
      "div",
      { class: "view share" },
      h("header", { class: "view-head" }, h("h2", null, t("share.title"))),
      h("p", null, t("share.intro")),
      offBox,
      onBox,
      status.el,
    ),
    destroy() {
      window.clearInterval(timer);
    },
  };
}
