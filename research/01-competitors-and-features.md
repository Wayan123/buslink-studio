# 01 — Competitors, user pain points, feature gaps

Research date: 2026-11-11. Scope: Modbus RTU/TCP commissioning, monitoring and logging tools, as input for BusLink Studio (Tauri 2, Windows + Android OTG + read-only LAN browser dashboard).

Conventions: facts come from the linked page. "UNVERIFIED" means I could not confirm it from a primary source. "[INFERENCE]" marks my own conclusion.

## Summary

- The field is crowded with desktop register pollers. On r/PLC a commenter notes "there is a new modbus tool posted on this subreddit every month" ([Busloom thread](https://www.reddit.com/r/PLC/comments/1w2b4qp/busloom_modern_tcprtu_modbus_client_with_charts/)). The closest new competitors are **Busloom** (also Tauri + Rust, desktop only, $149 Pro), **Modbench** with the open **.mbmap** map library (CC0, 621 maps), **ModbusLens**, **OpenModScan**, **ModScan Community** and **ModbusMate** (browser, Web Serial, RTU only).
- No tool I found combines all three of these: (a) the same device profile on a Windows PC and an Android phone over USB OTG, (b) per-value data-quality/staleness flags carried into logs and trends, (c) a read-only live view that other people on the LAN open in a browser. That combination is BusLink Studio's clearest opening. [INFERENCE]
- Write safety, device libraries and PDF/CSV map import have each started to appear in 2026 tools (Busloom, Modbench, ModbusCloud Diagnostics, Modbus Field Tester). They are no longer unique. BusLink should do them well but should not pitch them as novel.

---

## 1. Competitor matrix

Legend: ✅ yes · ⚠️ partial/limited · ❌ no · ? not stated on the source page (UNVERIFIED).

### 1a. Desktop GUI tools

| Tool | Platforms | RTU / TCP | Android OTG | Profiles / templates | Logging / charting | Export | Write safety | Price / license |
|---|---|---|---|---|---|---|---|---|
| **Modbus Poll** (Witte) [product](https://www.modbustools.com/modbus_poll.html), [order](https://www.modbustools.com/order.html), [download](https://www.modbustools.com/download.html) | Windows x86/x64/ARM64 | ✅ RTU, ASCII, TCP, TLS, RTU/ASCII over TCP/UDP | ❌ | Workspace files; no device library | Text log, log to Excel, real-time chart | Text, Excel (OLE/VBA) | Not in feature list ([Alledio comparison](https://www.alledio.com/post/modbus-poll-alternative)) | $129 perpetual; trial cuts the connection every 10 min, no connections after 30 days |
| **Modbus Slave** (Witte) | Windows | ✅ | ❌ | — (simulator) | — | — | — | $129 separate licence |
| **ModScan64 / ModSim64** (WinTECH) [home](https://win-tech.com/), [order](https://www.win-tech.com/html/orderm64.html) | Windows 10/11; iPhone app | ✅ RTU, ASCII, TCP, Secure Modbus (v8) | ❌ | ? | ? | ? | ? | ModScan64 $99.95 |
| **Simply Modbus** [home](https://simplymodbus.ca/), [TCP client](https://simplymodbus.ca/tools-client.html), [prices](https://simplymodbus.ca/purchase.html) | Windows (LabVIEW-based; Win 10 "unsupported but pragmatically functional") | ✅ but separate TCP Client and RTU Master products | ❌ | Config files, notes column for labels | Text log, byte history log | Text | ? | $60 per tool; demo limited to 6 messages per run |
| **CAS Modbus Scanner** (Chipkin) [page](https://store.chipkin.com/products/tools/cas-modbus-scanner) | Windows | ✅ RTU (RS232/485), TCP | ❌ | ❌ | ❌ read/scan only | ? | Read-only tool | Free, no registration |
| **QModMaster** [SourceForge](https://sourceforge.net/projects/qmodmaster/), [site](https://qmodmaster.com/) | Windows, Linux | ✅ | ❌ | Session save/load | Bus monitor, log to file; no chart | No direct CSV export | ? | Free, LGPLv3; last binary 0.5.3-beta, Mar 2022 |
| **Radzio Modbus Master Simulator (RMMS)** [page](https://en.radzio.dxp.pl/modbus-master-simulator/) | Windows | ✅ RTU, TCP, RTU over TCP | ❌ | ? | ❌ | ? | ? | Free, permissive licence; last update 28 Oct 2017 |
| **Modbus Doctor** (KScada) [page](https://www.kscada.com/modbusdoctor.html) | Windows | ✅ | ❌ | ? | Spy mode for raw frames | ? | ? | Free |
| **ModbusMechanic** [README](https://github.com/SciFiDryer/ModbusMechanic/blob/master/README.md) | Any OS with Java | ✅ | ❌ | Bookmarks | RTU bus monitor | ? | ? | Free, open source (licence UNVERIFIED); also an RTU→TCP gateway and register/RTU scanner |
| **OpenModScan** [repo](https://github.com/sanny32/OpenModScan) | Windows, many Linux distros | ✅ RTU, TCP, RTU over TCP | ❌ | ? | Traffic log | ? | ? | Free, MIT; v1.16.1, 737 stars |
| **ModScan Community** [Reddit](https://www.reddit.com/r/PLC/comments/1t48spq/modscan_community_an_opensource_modbus_rtutcp/), [repo](https://github.com/ThanabordeeN/modscan-pro) | Win/macOS/Linux | ✅ | ❌ | Project save/load; device library requested by users | CSV log, topology view | CSV | Author asked the sub what write safety should look like | AGPLv3 |
| **ModbusLens** [Reddit v2.1](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/) | Windows only | ✅ RTU, ASCII, TCP | ❌ | Tag list CSV (scaling not saved in v2.1) | Trend, raw TX/RX table | CSV | Min/max write bounds | Free, open source (Python) |
| **Busloom** [site](https://busloom.com/), [Reddit](https://www.reddit.com/r/PLC/comments/1w2b4qp/busloom_modern_tcprtu_modbus_client_with_charts/) | Win/macOS/Linux; **Tauri + Rust** | ✅ RTU, TCP, TLS | ❌ | Projects (Git-friendly file), CSV/TSV/XLSX map import with conflict preview, public device catalogue (v0.4) | Trends, alarms, recording + session replay | CSV, TSV, XLSX, JSON, JSONL, PNG/PDF reports | Read-only operator guard, encoded write preview, optional read-back, dry runs | Free non-commercial; Pro $149 one-time |
| **Modbench** + **mbmap-library** [Modbench](https://nobrega8.github.io/modbench/), [library](https://github.com/nobrega8/mbmap-library) | Win/macOS/Linux | ✅ TCP, RTU, ASCII | ❌ | Opens `.mbmap.json`; online library of 621 maps (18 verified, 450 official, 153 unofficial) | Charts, frames, CSV per cycle | CSV | Typed controls, limits from map, optional confirm, read-only mode | Free, open source; maps CC0 |
| **ModbusMate** (Alledio) [article](https://www.alledio.com/post/modbus-poll-alternative) | Chrome/Edge/Opera desktop (Web Serial) | ⚠️ RTU/ASCII only; TCP "coming soon" | ❌ (Web Serial is desktop-only) | JSON workspaces | Live chart, traffic log with CRC checks | CSV, XLSX, PNG | "Writes allowed" toggle → monitoring-only | Free (profile after 5 min) |
| **Busscope** [compare page](https://busscope.com/compare) | Windows only | ✅ TCP, TLS, RTU, ASCII | ❌ | ? | Live charts, CSV/Excel | CSV, Excel | RS-485 echo removal, RTS toggle | $39 launch / $69 |
| **Modbus Monitor XPF** (QBS) [FAQ](https://docs.quantumbitsolutions.com/support/faq/) | Windows | ✅ | ❌ (separate Android apps) | ? | Logging, charting, MQTT | CSV/JSON | ? | Paid (price UNVERIFIED) |
| **ModbusCloud Diagnostics** [manual import doc](https://www.modbuscloud.com/docs/import-a-manual) | Windows, macOS | ✅ (via USB-RS485 / TCP) | ❌ | Device-type library; **imports PDF/XLSX/CSV/TXT register tables, then checks them on the device** and marks "Verified on hardware" | Live tab | Gateway JSON template | Check step is read-only | Tied to the ModbusCloud gateway/portal (pricing UNVERIFIED) |

### 1b. Android apps

| App | RTU / TCP | USB OTG chips | Profiles | Logging / charting | Write safety | Price | Notes |
|---|---|---|---|---|---|---|---|
| **Modbus Monitor Advanced** (QBS) [product](https://quantumbitsolutions.com/android-monitor-advance/), [AppBrain](https://www.appbrain.com/app/modbus-monitor-advanced/com.Bhavan.Galex) | ✅ TCP, UDP, RTU/ASCII over TCP, RTU/ASCII over USB serial and Bluetooth | FTDI, PL2303, CP210x, CH34x, CDC | Import/export point configs; 408 paid QBS maps + free community maps | CSV export by interval; MQTT and Google Sheets as paid add-ons | Saved write presets | $3.99 (raised Oct 2026); ~5k installs | Reviews: rotating the screen wipes configured points; each point is its own poll, no block reads; "not intuitive" |
| **Modbus Monitor Free** (QBS) [guide](https://docs.quantumbitsolutions.com/products/android/free-guide/) | ✅ TCP, RTU over USB/Bluetooth | Same family | ❌ | ❌ (console text) | ❌ | Free | Form-based single read/write; shows 6-digit address |
| **Modbus Field Tester** (DaSoonTech) [Play](https://play.google.com/store/apps/details?hl=en_US&id=com.rs485.tester) | ✅ RTU (USB-RS485), TCP | ? | Template search; Free = 3 custom templates | TX/RX history (Pro: 60 entries) | Confirmation preview before every write | Free + Pro IAP, ads; 1k+ installs | Cloud AI diagnosis; Play data-safety lists location and financial info collection |
| **Modbus Droid / Mobile Modbus** [project](https://www.bencatlin.com/sofware-projects/modbus-droid/), [2012 post](https://www.bencatlin.com/2012/12/19/goodbye-modbus-droid-hello-mobile-modbus/) | ⚠️ TCP only | ❌ | ❌ | ❌ | Tap-to-write | Free, open source | Abandoned ~2012; author lost signing keys; no byte-swap support |

### 1c. CLI, libraries, gateways, integration stacks

| Tool | What it is | RTU / TCP | Profiles | Logging | Write safety | License |
|---|---|---|---|---|---|---|
| **mbpoll** [repo](https://github.com/epsilonrt/mbpoll) | CLI master (modpoll-like) | ✅ | ❌ | stdout, frame-loss stats | ❌ | GPLv3; v2 "under development" |
| **modbus-cli** [repo](https://github.com/tallakt/modbus-cli) | Ruby CLI; read/write/dump to YAML for backup/restore | ⚠️ TCP only | YAML dumps | File | README: "there is no protection here — you could easily mess up a running production system" | MIT |
| **pymodbus REPL** [repo](https://github.com/pymodbus-dev/repl) | Interactive Python client/server | ✅ | JSON server config | ❌ | ❌ | MIT; 10 stars |
| **mbusd** [repo](https://github.com/3cky/mbusd) | Modbus TCP→RTU gateway daemon | Gateway | ❌ | Log file | n/a | BSD-3; 686 stars; handles RS-485 local echo |
| **node-red-contrib-modbus** [flows](https://flows.nodered.org/node/node-red-contrib-modbus) | Node-RED nodes (read, flex-getter, write, server) | ✅ | IO-config JSON | Via flows | Via flows | BSD-3; v5.62.1; 3,677 downloads/week; serial on Node 26 is a "known risk" |
| **Telegraf + InfluxDB + Grafana** [Telegraf modbus](https://docs.influxdata.com/telegraf/v1/input-plugins/modbus/) | Collector → TSDB → dashboards | ✅ TCP, RTU, ASCII, RTU over TCP | TOML config per device; request optimisation (`max_insert`) | Historian | Read-only collector | Open source (Telegraf MIT, Grafana AGPL) |
| **Ignition** (Maker Edition) [docs](https://www.docs.inductiveautomation.com/docs/8.3/other-editions/ignition-maker-edition) | SCADA platform | ⚠️ Modbus Driver listed as Modbus-TCP | Address maps / UDTs | Historian | Platform security | Maker: free, non-commercial only, 10k tags, 10 sessions |
| **Waveshare VirCom** [community guide](https://github.com/edchen1240/waveshare-serial-server-vircom-guide) | Vendor config tool for RS485-to-ETH gateways | Gateway config | ❌ | ❌ | ❌ | Free, Windows (portable exe) |

---

## 2. Top 10 user pain points (with evidence)

| # | Pain point | Evidence | What BusLink should do |
|---|---|---|---|
| 1 | **0-based vs 1-based addressing ("Modbus shuffle").** Manuals mix `40001`, `40000`, `0` and protocol offsets; users subtract 1 by trial. | r/PLC "Why do I need to deduct 1": "Some modbus devices addressing are 0 based some are 1 based", "we call it the Modbus Shuffle" ([thread](https://www.reddit.com/r/PLC/comments/rfllc6/why_do_i_need_to_deduct_1_from_modbus_register/)). ModbusLens user misread an F32 offset because of a hidden 0/1-base toggle; dev: "nothing on the tag row itself showing which mode it's in" ([thread](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/)). Ignition users advise "treat everything as zero-based" ([forum](https://forum.inductiveautomation.com/t/best-practices-for-working-with-modbus-tcp/111030)). | Store the protocol offset; always show both "doc address" and "PDU offset" on every row. Make the base part of the profile, never a global hidden toggle. |
| 2 | **Byte/word order and multi-register types.** Floats read as garbage until the right swap is found. | "Little endian, big endian, motorola byte swap, 0 based, 1 based... every device has a different name" ([thread](https://www.reddit.com/r/PLC/comments/rfllc6/why_do_i_need_to_deduct_1_from_modbus_register/)); SO question on combining two registers into a float ([SO](https://stackoverflow.com/questions/64144397/convert-plc-modbus-signals-to-float-value-pair-of-16-bits-integer-into-a-32-bit)); vendor FAQ on swapped floats ([Schneider](https://www.se.com/us/en/faqs/FA401500/)). ModScan Community users ask for float, int32, 64-bit FP views ([thread](https://www.reddit.com/r/PLC/comments/1t48spq/modscan_community_an_opensource_modbus_rtutcp/)). | "Decode explorer": show one register pair in all four orders side by side; one click saves the chosen order into the profile. |
| 3 | **Unknown serial settings / slave ID; no discovery.** Field techs need to find a device before they can talk to it. | Field-tool request: "major bonus if it does scan/discovery of modbus RTU link" ([thread](https://www.reddit.com/r/PLC/comments/1pjoai3/most_user_friendly_modbus_pc_tool_write_to/)). ModbusLens added "Serial Discovery — sweep baud rate, parity, stop bits, and Unit ID" in v2.1 ([thread](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/)). Krohne case: vendors ship different serial defaults; once those are right, "polling the whole address space is quick" ([thread](https://www.reddit.com/r/PLC/comments/1rliyzh/struggling_with_modbus_rtu/)). | Unit-ID scan (1–247) at a chosen serial setting in MVP; full baud/parity sweep later. |
| 4 | **Wiring and bus faults look the same as software faults.** A timeout tells the user nothing about termination, A/B swap, biasing or a second master. | r/PLC "RTU timeouts": star topology, no termination, "I had no idea that rs485 needs to be terminated" ([thread](https://www.reddit.com/r/PLC/comments/1ilhm31/modbus_rtu_on_rs485_timeouts/)). Krohne thread: A/B polarity varies by vendor; "Modbus RTU can handle one and only one master" ([thread](https://www.reddit.com/r/PLC/comments/1rliyzh/struggling_with_modbus_rtu/)). ModbusCloud warns two masters on one bus give garbled answers ([doc](https://www.modbuscloud.com/docs/import-a-manual)). | Classify every failure as timeout / CRC error / framing / exception code / wrong unit ID, with counters per device, plus a short "likely cause" checklist (termination, A/B, baud, second master). |
| 5 | **Cost, trial limits and Windows lock-in for occasional use.** | Modbus Poll trial disconnects every 10 minutes and stops after 30 days ([Witte download](https://www.modbustools.com/download.html)); Alledio lists cost, trial limits and Windows-only as the "three practical walls" ([article](https://www.alledio.com/post/modbus-poll-alternative)). Busloom dev: otherwise "I would probably just pay $129 for modbus poll" ([thread](https://www.reddit.com/r/PLC/comments/1w2b4qp/busloom_modern_tcprtu_modbus_client_with_charts/)). Simply Modbus demo stops after 6 messages ([page](https://simplymodbus.ca/tools-client.html)). | Free and open source with no time limits; Windows + Android from day one. |
| 6 | **Retyping register maps from PDF manuals; no shared device library.** | ModScan Community feedback: "implement a device library… the community could help build a comprehensive database" and keep comm settings per device ([thread](https://www.reddit.com/r/PLC/comments/1t48spq/modscan_community_an_opensource_modbus_rtutcp/)). mbmap exists so users stop "typing addresses from a PDF manual" ([repo](https://github.com/nobrega8/mbmap-library)). QBS sells maps per device ([AppBrain](https://www.appbrain.com/app/modbus-monitor-advanced/com.Bhavan.Galex)). A user asks QBS to add the PZEM-004T to its pre-configured list (same page). | CSV register-map import in MVP; `.mbmap` import for interop; profile files versioned and diffable. PDF import later. |
| 7 | **Unsafe writes on live plants.** | modbus-cli README warns writes can "mess up a running production system" ([repo](https://github.com/tallakt/modbus-cli)). ModScan Community author asks "What safety measures should exist before allowing write operations?" ([thread](https://www.reddit.com/r/PLC/comments/1t48spq/modscan_community_an_opensource_modbus_rtutcp/)). Field crews need to write exactly one register (40002, addressing) ([thread](https://www.reddit.com/r/PLC/comments/1pjoai3/most_user_friendly_modbus_pc_tool_write_to/)). ModbusLens user found negative writes displayed as unsigned 65535 ([thread](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/)). | Read-only by default; writes only to profile-marked writable registers; show the encoded frame and old→new value; mandatory read-back with pass/fail; write journal. |
| 8 | **Inefficient polling and laggy UIs.** One request per tag, separate pollers for table and trend, UI blocked over VPN. | Detailed ModbusLens review: polling on the GUI thread, 150–200 ms per request over VPN "with 6 tags, that was about 1 second lag", asks to merge adjacent registers and "poll each register once and then share the result" ([thread](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/)). Modbus Monitor Advanced review: "there is no way to do a multi register poll… Each point defined is a complete poll / response" ([AppBrain](https://www.appbrain.com/app/modbus-monitor-advanced/com.Bhavan.Galex)). Telegraf ships request optimisers for the same reason ([docs](https://docs.influxdata.com/telegraf/v1/input-plugins/modbus/)). | One Rust poll engine per link: sequential on RTU, block-merged reads with a max-gap setting and per-device "do not span" ranges; all views subscribe to the same samples. |
| 9 | **Stale or frozen values look healthy.** Modbus has no quality field, so a failed read often keeps showing the last value; trends join across outages. | Ignition: a frozen value from an aggregator showed "Good"; IA engineer: "Modbus doesn't have a concept of quality" ([forum](https://forum.inductiveautomation.com/t/modbus-tcp-and-tag-quality/36172)). Node-RED flex-getter skipped remaining slaves after one timed out ([issue #288](https://github.com/BiancoRoyal/node-red-contrib-modbus/issues/288)). Grafana users asked from 2016 to 2023 for an option "to not connect lines over missing data"; it shipped in v10.1 ([issue #7086](https://github.com/grafana/grafana/issues/7086)). | Every sample carries quality (good / stale / timeout / exception / CRC) and age. Trends break the line at gaps; CSV exports include quality columns. A failure on one device never blocks the others. |
| 10 | **Mobile tools are fragile or abandoned.** | Modbus Monitor Advanced: "Changing orientation blows out any points you've created" ([AppBrain](https://www.appbrain.com/app/modbus-monitor-advanced/com.Bhavan.Galex)); QBS FAQ lists OTG not detected, permission dialog not appearing, phones limiting USB power ([FAQ](https://docs.quantumbitsolutions.com/support/faq/)). Modbus Droid: TCP only, abandoned after signing keys were lost ([post](https://www.bencatlin.com/2012/12/19/goodbye-modbus-droid-hello-mobile-modbus/)). Field Tester gates scanning and continuous polling behind Pro and uploads snapshots for AI diagnosis ([Play](https://play.google.com/store/apps/details?hl=en_US&id=com.rs485.tester)). | Persist all session state in the Rust core (survives rotation/process death); clear OTG permission flow with chip name shown; offline, no account. |

Secondary complaints seen but not ranked: scaling settings not saved with tags; no copy/export from raw tables; tiny fonts; need to monitor several targets side by side; PCAP export wanted instead of running Wireshark ([ModbusLens](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/), [Busloom](https://www.reddit.com/r/PLC/comments/1w2b4qp/busloom_modern_tcprtu_modbus_client_with_charts/) threads); Unit ID 0 used on single-device serial lines even though it is "broadcast" in the spec (Busloom thread).

---

## 3. Feature gap table

"Who does it" lists only tools where the source page states the feature.

| Capability | Who does it today | Gap | BusLink angle |
|---|---|---|---|
| Same device profile on PC **and** Android (OTG RTU) | QBS has separate Windows (XPF) and Android products; profile sharing between them UNVERIFIED | **None confirmed** | One profile file, one Rust core, both targets. Strongest differentiator. |
| Per-value quality / staleness flags in UI, logs and trends | Busloom shows "source health"; Grafana can break lines at gaps (v10.1+) | **Few**, and none in a mobile tool | Quality + age on every value, in CSV, and as gap-aware trends. |
| Gap-aware trends (no lines across outages) | Grafana (after config); others UNVERIFIED | **Few** | Default behaviour, not a setting. |
| Read-only LAN share to any browser (no install for viewers) | Ignition/Grafana do it, but need a server stack; desktop tools: none found | **None among commissioning tools** | Built-in HTTP server in the app; QR code / URL; read-only by construction. Good demo for labs and handover. |
| Guided commissioning with write + read-back + pass/fail record | Busloom (optional read-back, scenarios, Pro automation $149); Modbench (confirm, limits) | **Few**, mostly desktop and paid tiers | Simple checklist runner: step → expected value → write → read-back → signed-off report. |
| Open, versioned device profile catalog | mbmap-library (CC0, 621 maps, JSON Schema); Busloom public catalogue; QBS paid maps | Exists, but adoption is thin (mbmap: 14 stars, 18 maps verified on hardware) | Import `.mbmap` instead of inventing a rival format; contribute verified maps back. [INFERENCE] |
| Register-map import from CSV/XLSX | Busloom (conflict preview); ModbusCloud Diagnostics | Few | CSV in MVP with preview + address-base detection. |
| Register-map import from PDF | ModbusCloud Diagnostics (PDF text tables, then on-device check) | **Very few** | Later phase; reuse the "check on device, mark verified" idea. |
| Explicit doc-address vs PDU-offset display | Busloom; Modbus Monitor Free (6-digit address) | Common failure elsewhere | Both columns, always. |
| Failure classification (timeout vs CRC vs exception) with causes | Busloom, Field Tester (AI, cloud), ModbusMate counters | Partial | Offline rule-based hints; no cloud. |
| Shared poll engine with block merging | Telegraf (optimiser); Busloom (native runtime) | Missing in most GUIs | Core design requirement. |
| Built-in simulator for training without hardware | Modbench, ModbusMate (loopback demo), Modbus Slave (paid) | Common | Needed anyway for the hackathon demo and labs. |
| Free, no time limit, open source, Windows + Android | Free desktop tools exist; free Android tools are limited or ad/IAP-gated | **None with full Android RTU** | Licence choice matters for labs. |

---

## 4. Prioritized feature list

Window: 2026-11-11 → 2026-12-04 deadline (≈3 weeks). Priorities assume a small team and a demo judged on impact, implementation, design, innovation and presentation.

### MVP (must ship by Dec 4)

| P | Feature | Why (pain # / gap) | Notes |
|---|---|---|---|
| P0 | Rust poll engine: RTU (serial) and TCP, one sequential queue per link, block-merged reads, per-device timeout/retry, one failing device never blocks others | 8, 9 | All UI views, logger and LAN server read from the same sample stream. |
| P0 | Per-sample quality (good / stale / timeout / exception code / CRC) and age | 9, gap row 2 | Drives colours in the table, gaps in trends, columns in CSV. |
| P0 | Device profile file (JSON, versioned, schema-validated): registers, type, byte/word order, scale, unit, address base, writable flag, min/max, serial defaults | 1, 2, 6 | Same file on Windows and Android. |
| P0 | Android USB OTG serial for CH340/CH343 (plus FTDI/CP210x if the library gives them for free) | 10, gap row 1 | Depends on AndroidUsbAndTauri findings. Persist session in core so rotation does not lose state. |
| P0 | Live table with doc address + PDU offset, decoded value, unit, quality, age | 1 | |
| P0 | Safe write: read-only by default, writable only if profile allows, preview of encoded frame and old→new, read-back verify, write journal | 7 | |
| P0 | Built-in TCP simulator device (and a demo profile) | demo, labs | Lets judges run it without hardware. |
| P1 | Gap-aware live trend (line breaks at stale/failed samples) | 9 | |
| P1 | CSV logging with quality columns and session metadata | 9 | |
| P1 | Read-only LAN dashboard served by the app (browser view of selected tags, quality, trend), URL/QR, off by default, token in URL | gap row 4 | Bind only when user enables it; no write endpoints at all. |
| P1 | CSV register-map import with preview and address-base detection | 6 | |
| P1 | Failure counters per device + offline hint list (termination, A/B, baud, unit ID, second master) | 4 | |
| P2 | Unit-ID scan 1–247 at the current serial settings | 3 | Cheap to add once the engine exists. |
| P2 | Decode explorer (all four byte orders side by side, save choice to profile) | 2 | |
| P2 | `.mbmap` import (read-only converter into BusLink profile) | 6, gap row 6 | Read SPEC.md first; format details UNVERIFIED here. |

### Later (after the hackathon)

- Guided commissioning checklists with pass/fail report (PDF/HTML).
- Baud/parity/stop-bit sweep discovery; TCP subnet scan on port 502 using connect-scan (not ARP; see ModbusLens thread).
- PDF register-table import with on-device verification, modelled on ModbusCloud's flow.
- Public profile catalog repo with "verified on hardware" status; contribute back to mbmap.
- Raw frame log export as PCAP; session record and replay.
- RTU over TCP, ASCII, Modbus/TCP Security (TLS), FC08/FC17/FC43 diagnostics.
- MQTT publish, Home Assistant/Node-RED export.
- Desktop Linux/macOS builds (Tauri supports them; Busloom already ships them).
- Multi-target side-by-side view; alarms.

---

## Contradictions

- QModMaster's community site lists Modbus ASCII in its spec table, while Busscope's comparison lists QModMaster as "TCP and RTU" only ([qmodmaster.com](https://qmodmaster.com/), [Busscope](https://busscope.com/compare)). ASCII support UNVERIFIED.
- ModbusSimulator.com lists Modbus Poll multi-user/site prices ($549/$899/$1,299) that do not appear on Witte's own order page, which shows per-licence tiers only ($129/$119/$109) ([vendor blog](https://modbussimulator.com/blog/modbus-poll-license), [Witte](https://www.modbustools.com/order.html)). Trust Witte's page. The blog is a competitor's marketing.
- ModbusLens users assumed discovery depended on ARP; the developer says a TCP connect-scan already exists but is not visible ([thread](https://www.reddit.com/r/PLC/comments/1ven88w/modbuslens_v210_is_out_txrx_bytes_scanner_serial/)).

## Missing evidence

- Waveshare's own VirCom wiki page returned HTTP 403 (Cloudflare); VirCom details come from a third-party bench guide only.
- No Stack Overflow or PLCtalk thread read in full for write-safety incidents; pain #7 rests on tool docs and r/PLC.
- Prices for Modbus Monitor XPF and ModbusCloud Diagnostics not checked.
- ModbusMechanic licence, CAS Scanner export options, Radzio and Modbus Doctor logging features are not stated on their pages.
- Whether QBS shares profiles between XPF (Windows) and Android Advanced.
- r/IndustrialAutomation was not searched separately; r/PLC carried almost all relevant threads.
- Android review text was read via AppBrain, not Google Play directly.

## Sources

- Kept: vendor pages (Witte, WinTECH, Simply Modbus, Chipkin, KScada, Radzio, QBS, Busloom, Modbench, Alledio, ModbusCloud) for prices and features; GitHub repos (OpenModScan, mbpoll, mbusd, modbus-cli, pymodbus REPL, ModbusMechanic, mbmap-library, node-red-contrib-modbus issue #288, Grafana issue #7086) for licence and behaviour; r/PLC threads and the Ignition forum for user pain; AppBrain for Android reviews and price history.
- Rejected: modbussimulator.com and controlslens.com comparison posts as primary sources (competitor marketing, unverified tiers); scadaprotocols.com "best free tools" list (aggregator, no new facts).

## Next steps

1. Read mbmap `SPEC.md` and the JSON Schema to decide whether BusLink's profile format should be a superset of `.mbmap` v2.
2. Install Busloom Community and Modbench and check, hands-on, their quality/staleness display and write read-back, since they are the nearest competitors.
3. Confirm with AndroidUsbAndTauri which serial library covers CH343G on Android and whether session state can live in the Rust core across activity restarts.
