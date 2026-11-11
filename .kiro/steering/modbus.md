---
inclusion: fileMatch
fileMatchPattern: "crates/**/*.rs"
---

# Modbus rules for the Rust core

Primary sources: Modbus Application Protocol V1.1b3, Modbus over Serial Line V1.02, Modbus Messaging on TCP/IP Implementation Guide V1.0b (modbus.org). Writes are only encoded in `crates/buslink-core/src/command/` (see safety.md).

## RTU framing

- ADU = `unit_id (1) | PDU (function + data, ≤ 253) | CRC (2)`; max 256 bytes.
- **CRC-16/MODBUS**: init `0xFFFF`, reflected poly `0xA001` (shift right, XOR when LSB is 1), no final XOR, transmitted **low byte first**. Check value: CRC of ASCII `123456789` = `0x4B37`. Known frame: `01 03 00 00 00 08 44 0C` (unit 1, FC03, addr 0, qty 8).
- Character time = (start + data bits + parity + stop bits) / baud; the spec assumes 11 bits (8E1, 8O1, 8N2). 8N1 (10 bits) is common and must work; compute from the real settings.
- **t3.5** (inter-frame silence) = 3.5 characters; **t1.5** (max gap inside a frame) = 1.5 characters. Above 19 200 baud use fixed t1.5 = 750 µs, t3.5 = 1.75 ms.
- Over USB-serial, bytes arrive in USB packets, not at character timing. Do **not** delimit frames by silence alone: read the header, compute the expected length from function code and byte count, accumulate partial reads until a deadline, then validate unit ID, CRC, function and length.
  - FC01/02 response: `5 + ceil(qty/8)` bytes; FC03/04: `5 + 2·qty`; FC05/06/15/16: 8; exception: 5.
- Before each request: drain stale input (read-and-discard until quiet; the CH340 driver on Android throws on hardware purge). After a response, wait at least t3.5 (≥ 5 ms floor; Android ≥ 50 ms) before the next request.
- Response timeout scales with baud (frame time + device turnaround); default ≥ 200 ms, typically 500–1000 ms at 9600. A late response that arrives after the timeout belongs to the old request: discard it, never match it to the next one.
- A response from another unit ID, a bad CRC or a wrong length is an error for this request (CRC_ERROR or DECODE_ERROR), never a value.

## Modbus TCP (MBAP)

- Header 7 bytes: transaction ID (2), protocol ID (2, must be 0), length (2 = unit ID + PDU bytes, ≤ 254), unit ID (1). Max ADU 260 bytes. Big-endian.
- TCP is a stream: frame by the length field, handle partial and coalesced reads, reject length 0 or > 254.
- Match responses by transaction ID; one outstanding request per connection unless a device is known to pipeline. Reconnect with backoff; a dropped connection marks all its devices TIMEOUT/STALE.
- Default port 502. Unit ID: direct TCP devices usually ignore it or expect 1 or 255; through a gateway it selects the serial slave. Never 0 for writes.
- **RTU over TCP** (raw RTU frames with CRC through a transparent serial server) is a separate transport kind, never confused with MBAP.

## Function codes and limits

| FC | Name | Quantity limit | Notes |
|---|---|---|---|
| 01 | Read Coils | 1–2000 | bit-packed, LSB first |
| 02 | Read Discrete Inputs | 1–2000 | |
| 03 | Read Holding Registers | 1–125 | |
| 04 | Read Input Registers | 1–125 | |
| 05 | Write Single Coil | 1 | value only `0xFF00` (on) or `0x0000` (off) |
| 06 | Write Single Register | 1 | response echoes the request |
| 15 (0x0F) | Write Multiple Coils | 1–1968 | byte count = ceil(qty/8) |
| 16 (0x10) | Write Multiple Registers | 1–123 | byte count = 2·qty |

Also `address + quantity ≤ 65 536`. Devices may impose smaller limits (profile or per-device setting). Optional later: FC22/23, FC43/14 device identification. Validate before sending; never send a request the spec forbids.

## Exceptions

Response function = request function | `0x80`, followed by one exception code: `01` illegal function, `02` illegal data address, `03` illegal data value, `04` server device failure, `05` acknowledge (long operation), `06` server busy, `08` memory parity error, `0A` gateway path unavailable, `0B` gateway target failed to respond. Map to quality `EXCEPTION` with the code kept and shown ("EXCEPTION 02 illegal data address").

## Addressing

- The PDU address is a **zero-based offset** 0–65535 inside one of four tables. The table is chosen by the function code, not by the number.
- Manual labels: `0xxxx` coils, `1xxxx` discrete inputs, `3xxxx` input registers, `4xxxx` holding registers (6-digit `4xxxxx` variants exist). `40001` → holding register, offset 0. Some manuals already print zero-based offsets; some write `40000` for offset 0. The base is a per-profile import choice, never a hidden global toggle.
- **Never send a label like 40001 as an address.** Store `table + address (offset)`, keep the manual label as `doc_address`, and show both on every row.

## Data types and word order

Registers are 16-bit big-endian on the wire. Multi-register values (32/64-bit, float) are a device convention, so `word_order` is an explicit profile field. Letters name bytes of the value from most significant (A) to least significant:

| word_order | Wire registers for f32 123.456 (`0x42F6E979`) | Meaning |
|---|---|---|
| ABCD | `42F6 E979` | big-endian words and bytes |
| CDAB | `E979 42F6` | word swap |
| BADC | `F642 79E9` | byte swap inside each word |
| DCBA | `79E9 F642` | both swapped (little-endian) |

For 64-bit types apply the same two rules: word swap reverses the 4 registers, byte swap swaps bytes inside each register. Never pick an order because the result "looks plausible"; the register inspector shows all four next to the raw hex.

Decode order: raw registers → type (with word order) → sentinel check → `value × scale + offset` → range check. Non-finite floats, sentinels and out-of-range values give `DECODE_ERROR` with the raw value kept. Preserve 64-bit integers exactly (serialize as strings in JSON to the UI).

## Quality and polling

- Sample quality: `GOOD`, `TIMEOUT`, `CRC_ERROR`, `EXCEPTION(code)`, `STALE`, `DECODE_ERROR`. Link state (connected, disconnected, permission denied) is separate.
- A failed read keeps the last good value **and its original timestamp**; age keeps growing. Default STALE threshold: age > max(3 × poll interval, 3 s), configurable per device.
- One worker per bus, strictly sequential on RTU. Merge adjacent registers into one read only when every address in the block is valid for the device (respect "do not span" ranges and a max gap). All views share the same samples; never poll twice for two screens.
- A failing device gets backoff and never blocks other devices on the bus. Retries are bounded (default 1) for reads only.
- Record per-device counters: requests, GOOD, timeouts, CRC errors, exceptions, last error, actual cycle time.

## Tests

Byte-level fixtures for: CRC check value, every read FC request/response, exception frames, partial frames split at every byte, bad CRC, wrong unit ID, late response after timeout, MBAP length errors, all four word orders for i32/u32/f32/f64, sentinels and scale. Write-frame tests live under `crates/*/tests/` or `crates/buslink-core/src/command/`.
