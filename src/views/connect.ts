// Connection center (dashboard-ui Req 1): ports with VID:PID chip labels, RTU/TCP settings,
// Training mode, links with state and counters. L4 link diagnostics live here too.
import { api, errorText, type ConnectReq } from "../api.ts";
import { linkStateBadge, simulatedBanner } from "../components/badges.ts";
import { clear, h } from "../dom.ts";
import { fmtAge, fmtNumber } from "../format.ts";
import { lang, t } from "../i18n.ts";
import { store } from "../store.ts";
import type { LinkView, PortView } from "../types.ts";
import { field, preserveFocus, select, statusLine, type View } from "./view.ts";

const BAUDS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200];

export function connectView(): View {
  const portsBox = h("div", { class: "ports" });
  const portStatus = statusLine();
  const linksBox = h("div", { class: "links" });
  const status = statusLine();

  // --- RTU form ---
  const portSel = h("select");
  const baudIn = h("input", { type: "number", inputmode: "numeric", min: 300, max: 3000000, value: 9600, list: "bauds" });
  const bauds = h("datalist", { id: "bauds" }, ...BAUDS.map((b) => h("option", { value: b })));
  const dataSel = select([["8", "8"], ["7", "7"]], "8");
  const paritySel = select([["none", t("serial.parity.none")], ["even", t("serial.parity.even")], ["odd", t("serial.parity.odd")]], "none");
  const stopSel = select([["1", "1"], ["2", "2"]], "1");
  const fPort = field("rtu-port", t("serial.port"), portSel);
  const fBaud = field("rtu-baud", t("serial.baud"), baudIn, t("serial.baud.hint"));
  const rtuForm = h(
    "form",
    { class: "card form-grid", "aria-labelledby": "rtu-h" },
    h("h3", { id: "rtu-h" }, t("connect.rtu")),
    fPort.el,
    fBaud.el,
    bauds,
    field("rtu-data", t("serial.data_bits"), dataSel).el,
    field("rtu-parity", t("serial.parity"), paritySel).el,
    field("rtu-stop", t("serial.stop_bits"), stopSel).el,
    h("p", { class: "advice", role: "note" }, t("connect.isolation")),
    h("button", { type: "submit", class: "primary" }, t("connect.rtu.action")),
  );
  rtuForm.addEventListener("submit", (e) => {
    e.preventDefault();
    fPort.error(null);
    fBaud.error(null);
    const baud = Number(baudIn.value);
    let bad = false;
    if (!portSel.value) {
      fPort.error(t("connect.err.port"));
      bad = true;
    }
    if (!Number.isInteger(baud) || baud < 300 || baud > 3_000_000) {
      fBaud.error(t("connect.err.baud"));
      bad = true;
    }
    if (bad) return;
    void doConnect({
      kind: "rtu",
      port: portSel.value,
      baud,
      data_bits: Number(dataSel.value),
      parity: paritySel.value as ConnectReq["parity"],
      stop_bits: Number(stopSel.value),
    });
  });

  // --- TCP form ---
  const hostIn = h("input", { type: "text", autocomplete: "off", spellcheck: "false", placeholder: "192.168.1.50" });
  const tcpPortIn = h("input", { type: "number", inputmode: "numeric", min: 1, max: 65535, value: 502 });
  const tcpKind = select([["tcp", t("connect.tcp.mbap")], ["rtu_over_tcp", t("connect.tcp.rtu")]], "tcp");
  const fHost = field("tcp-host", t("connect.tcp.host"), hostIn);
  const fTcpPort = field("tcp-port", t("connect.tcp.port"), tcpPortIn);
  const tcpForm = h(
    "form",
    { class: "card form-grid", "aria-labelledby": "tcp-h" },
    h("h3", { id: "tcp-h" }, t("connect.tcp")),
    field("tcp-kind", t("connect.tcp.kind"), tcpKind, t("connect.tcp.kind.hint")).el,
    fHost.el,
    fTcpPort.el,
    h("p", { class: "advice", role: "note" }, t("connect.tcp.wifi")),
    h("button", { type: "submit", class: "primary" }, t("connect.tcp.action")),
  );
  tcpForm.addEventListener("submit", (e) => {
    e.preventDefault();
    fHost.error(null);
    fTcpPort.error(null);
    const host = hostIn.value.trim();
    const port = Number(tcpPortIn.value);
    let bad = false;
    if (!/^[A-Za-z0-9.\-:[\]]{1,253}$/.test(host)) {
      fHost.error(t("connect.err.host"));
      bad = true;
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      fTcpPort.error(t("connect.err.tcp_port"));
      bad = true;
    }
    if (bad) return;
    void doConnect({ kind: tcpKind.value as ConnectReq["kind"], host, tcp_port: port, baud: 9600 });
  });

  // --- Training ---
  const trainingBtn = h("button", { type: "button", class: "primary big", "data-testid": "start-training" }, t("connect.training.action"));
  trainingBtn.addEventListener("click", async () => {
    trainingBtn.disabled = true;
    status.set(t("connect.training.starting"));
    try {
      await api.trainingStart();
      status.set(t("connect.training.started"), "ok");
      location.hash = "#/dashboard";
    } catch (e) {
      status.set(errorText(e), "error");
    } finally {
      trainingBtn.disabled = false;
    }
  });

  async function doConnect(req: ConnectReq) {
    status.set(t("connect.connecting"));
    try {
      const r = await api.connect(req);
      status.set(t("connect.connected", { id: r.link_id }), "ok");
    } catch (e) {
      status.set(errorText(e), "error");
    }
  }

  function renderPorts(ports: PortView[]) {
    clear(portsBox);
    const prev = portSel.value;
    clear(portSel);
    portSel.append(h("option", { value: "" }, t("serial.port.choose")));
    if (ports.length === 0) {
      portsBox.append(h("p", { class: "muted" }, t("connect.ports.none")));
    }
    const rows = ports.map((p) => {
      const chip = p.chip ? `${p.chip.vendor} ${p.chip.chip} (${p.chip.driver})` : t("connect.ports.unknown_chip");
      portSel.append(h("option", { value: p.name }, `${p.name}${p.vid_pid ? ` · ${p.vid_pid}` : ""}${p.chip ? ` · ${p.chip.chip}` : ""}`));
      return h(
        "tr",
        null,
        h("th", { scope: "row" }, p.name),
        h("td", null, p.vid_pid ?? "—"),
        h("td", null, chip),
        h("td", null, [p.manufacturer, p.product].filter(Boolean).join(" · ") || "—"),
      );
    });
    if (rows.length) {
      portsBox.append(
        h(
          "div",
          { class: "table-wrap" },
          h(
            "table",
            null,
            h("caption", { class: "sr-only" }, t("connect.ports")),
            h("thead", null, h("tr", null, h("th", { scope: "col" }, t("serial.port")), h("th", { scope: "col" }, "VID:PID"), h("th", { scope: "col" }, t("connect.ports.chip")), h("th", { scope: "col" }, t("connect.ports.desc")))),
            h("tbody", null, rows),
          ),
        ),
      );
    }
    if (prev) portSel.value = prev;
  }

  async function refreshPorts() {
    portStatus.set(t("connect.ports.scanning"));
    try {
      const ports = await api.listPorts();
      renderPorts(ports);
      portStatus.set(t("connect.ports.found", { n: ports.length }), "ok");
    } catch (e) {
      portStatus.set(errorText(e), "error");
    }
  }

  function linkActions(l: LinkView): HTMLElement {
    const act = (label: string, fn: () => Promise<void>, cls = "") => {
      const b = h("button", { type: "button", class: cls, "data-fk": `${l.id}:${label}` }, label);
      b.addEventListener("click", async () => {
        b.disabled = true;
        try {
          await fn();
        } catch (e) {
          status.set(errorText(e), "error");
        } finally {
          b.disabled = false;
        }
      });
      return b;
    };
    const box = h("div", { class: "actions" });
    if (l.state === "connected") box.append(act(t("link.start"), () => api.startPolling(l.id), "primary"));
    if (l.state === "polling") box.append(act(t("link.stop"), () => api.stopPolling(l.id)));
    if (l.state === "down" || l.state === "disconnected") box.append(act(t("link.reconnect"), () => api.reconnect(l.id)));
    if (l.state === "connected" || l.state === "polling") box.append(h("span", { class: "spacer" }), act(t("link.disconnect"), () => api.disconnect(l.id)));
    if (l.state === "down" || l.state === "disconnected") box.append(h("span", { class: "spacer" }), act(t("link.remove"), () => api.removeLink(l.id), "danger"));
    return box;
  }

  function renderLinks() {
    clear(linksBox);
    if (store.links.length === 0) {
      linksBox.append(h("p", { class: "muted" }, t("connect.links.none")));
      return;
    }
    const L = lang();
    for (const l of store.links) {
      const rows = l.devices.map((d) =>
        h(
          "tr",
          null,
          h("th", { scope: "row" }, d.name, h("br"), h("span", { class: "muted small" }, `${t("device.unit")} ${d.unit_id}`)),
          h("td", { class: "num" }, fmtNumber(d.counters.requests, L)),
          h("td", { class: "num" }, fmtNumber(d.counters.good, L)),
          h("td", { class: "num" }, fmtNumber(d.counters.timeouts, L)),
          h("td", { class: "num" }, fmtNumber(d.counters.crc_errors, L)),
          h("td", { class: "num" }, fmtNumber(d.counters.exceptions, L)),
          h("td", { class: "num" }, fmtNumber(d.counters.decode_errors, L)),
          h("td", { class: "num" }, d.counters.cycle_ms === null ? "—" : fmtAge(d.counters.cycle_ms, L)),
          h("td", { class: "small" }, d.counters.last_error ?? "—"),
        ),
      );
      linksBox.append(
        h(
          "section",
          { class: `card link-card link-${l.state}`, "aria-label": l.label },
          h("header", { class: "card-head" }, h("h3", null, l.label), linkStateBadge(l.state), h("span", { class: "muted small" }, t(`link.kind.${l.kind}`))),
          l.simulated ? simulatedBanner() : null,
          l.error ? h("p", { class: "error-text" }, l.error) : null,
          rows.length
            ? h(
                "div",
                { class: "table-wrap" },
                h(
                  "table",
                  { class: "counters" },
                  h("caption", { class: "sr-only" }, t("connect.counters")),
                  h(
                    "thead",
                    null,
                    h(
                      "tr",
                      null,
                      ...["device.name", "counter.requests", "counter.good", "counter.timeouts", "counter.crc", "counter.exceptions", "counter.decode", "counter.cycle", "counter.last_error"].map((k) =>
                        h("th", { scope: "col" }, t(k)),
                      ),
                    ),
                  ),
                  h("tbody", null, rows),
                ),
              )
            : h("p", { class: "muted" }, t("connect.links.no_devices")),
          linkActions(l),
        ),
      );
    }
  }

  const refreshBtn = h("button", { type: "button" }, t("connect.ports.refresh"));
  refreshBtn.addEventListener("click", () => void refreshPorts());

  const el = h(
    "div",
    { class: "view connect" },
    h("h2", null, t("nav.connect")),
    h(
      "section",
      { class: "card training-cta", "aria-labelledby": "train-h" },
      h("h3", { id: "train-h" }, t("connect.training")),
      h("p", null, t("connect.training.text")),
      trainingBtn,
    ),
    status.el,
    h("section", { class: "card", "aria-labelledby": "ports-h" }, h("header", { class: "card-head" }, h("h3", { id: "ports-h" }, t("connect.ports")), refreshBtn), portStatus.el, portsBox),
    h("div", { class: "two-col" }, rtuForm, tcpForm),
    h("h3", null, t("connect.links")),
    linksBox,
  );

  const unsub = store.subscribe((c) => {
    if (c.links) preserveFocus(linksBox, renderLinks);
  });
  renderLinks();
  void refreshPorts();
  return { el, destroy: unsub };
}
