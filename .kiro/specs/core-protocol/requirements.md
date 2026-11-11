# core-protocol: requirements

Epic E1 (`docs/bmad/epics-and-sprint.md`). Crate: `crates/buslink-core` (pure Rust, no Tauri dependency).
Covers PRD R3 (framing part), R4, R5, R6, R7, R8, R9, R20/R21 (core part), N1, N2 and architecture decisions D1, D2, D3, D4, D8, D9.
Rules that always apply: `.kiro/steering/safety.md`, `.kiro/steering/modbus.md`.

Acceptance criteria use EARS. "The core" means `buslink-core`.

## Requirement 1: RTU framing and CRC (R5, D1)

**User story:** As a commissioning technician, I want every RTU frame to be built and checked exactly as the Modbus serial line spec says, so that a reading is never taken from a corrupted frame.

1. THE core SHALL compute CRC-16/MODBUS (init `0xFFFF`, reflected polynomial `0xA001`, no final XOR) and transmit it low byte first.
2. THE core SHALL produce `0x4B37` for ASCII `123456789` and the frame `01 03 00 00 00 08 44 0C` for unit 1, FC03, address 0, quantity 8.
3. WHEN an RTU ADU is encoded with a PDU that is empty or longer than 253 bytes, THE core SHALL reject it with an error.
4. THE core SHALL derive character time, t1.5 and t3.5 from the actual serial settings (start + data + parity + stop bits) and SHALL use the fixed 750 µs / 1.75 ms values above 19 200 baud.

## Requirement 2: length-based RTU response reception (R5, D1, research/03 risk table)

**User story:** As a field user on Android OTG, I want responses that arrive in USB-sized fragments to be reassembled reliably, so that fragmentation never shows up as CRC errors or false timeouts.

1. WHEN response bytes arrive, THE core SHALL compute the expected length from the function code and byte count (FC01–04: `5 + byte_count`; exception: 5; fixed echo replies: `4 + data length`) and SHALL accumulate partial reads until that length or the deadline.
2. WHEN the deadline passes before the expected length is received, THE core SHALL report TIMEOUT.
3. WHEN a complete frame has a bad CRC, THE core SHALL report CRC_ERROR and SHALL NOT decode any value from it.
4. WHEN a complete frame has a valid CRC but a different unit ID, an unexpected function code, or a byte count that does not match the request, THE core SHALL report DECODE_ERROR.
5. BEFORE each RTU request, THE core SHALL discard stale input by reading until quiet (no hardware purge), and AFTER each exchange THE core SHALL keep at least the inter-frame gap (≥ t3.5, ≥ 5 ms floor, configurable floor for Android) before the next request.
6. WHEN arbitrary bytes are fed to the response framer or any decoder, THE core SHALL NOT panic.

## Requirement 3: exception responses (R7)

1. WHEN a response function code equals the request function code with bit 7 set, THE core SHALL report EXCEPTION with the exception code kept (01, 02, 03, 04, 05, 06, 08, 0A, 0B and unknown codes).
2. THE core SHALL provide a name for every standard exception code so that the UI can show "EXCEPTION 02 illegal data address".

## Requirement 4: Modbus TCP MBAP (R3)

1. THE core SHALL encode the MBAP header (transaction ID, protocol ID 0, length = unit + PDU bytes, unit ID), big-endian.
2. WHEN an MBAP header has a protocol ID other than 0 or a length below 2 or above 254, THE core SHALL reject it as a framing error.
3. THE core SHALL frame TCP responses by the length field and SHALL handle partial and coalesced reads.
4. WHEN a TCP response carries a transaction ID other than the one outstanding, THE core SHALL discard it and keep waiting until the deadline.
5. WHEN the MBAP stream can no longer be framed, THE core SHALL report the link as lost instead of guessing a frame boundary.
6. THE core SHALL keep RTU-over-TCP as a separate framing kind (RTU frames with CRC over a stream), never confused with MBAP.

## Requirement 5: read PDUs and spec limits (R5, R8)

1. THE core SHALL encode and decode FC01, FC02, FC03 and FC04 requests and responses.
2. WHEN a read request has quantity 0, more than 2000 bits (FC01/02) or more than 125 registers (FC03/04), or `address + quantity > 65 536`, THE core SHALL reject it before anything is sent.
3. THE core SHALL address registers by `table + zero-based PDU offset` only; a manual label (`40001`) SHALL never be used as an address. The table SHALL be chosen by the function code.
4. Write PDUs (FC05/06/15/16) SHALL be encoded only inside `crates/buslink-core/src/command/` (safety rule 2).

## Requirement 6: data codec (R6)

1. THE core SHALL decode and encode `bool`, `u16`, `i16`, `bitfield`, `u32`, `i32`, `f32`, `u64`, `i64`, `f64` and `string` values from 16-bit big-endian registers.
2. THE core SHALL support the word orders ABCD, CDAB, BADC and DCBA for 32- and 64-bit types, applying word swap (register reversal) and byte swap (inside each register) as defined in `modbus.md`; strings SHALL support ABCD and BADC.
3. WHEN the number of registers does not match the type width, THE core SHALL return DECODE_ERROR instead of panicking.
4. THE core SHALL keep 64-bit integers exact (no conversion through `f64` for the raw value).

## Requirement 7: engineering values, range and sentinels (R6, R7)

1. THE core SHALL compute `engineering = raw × scale + offset` and SHALL support no other expression.
2. WHEN the raw value equals an `invalid_sentinels` entry, the result is not finite, or the engineering value is outside `valid_range`, THE core SHALL report DECODE_ERROR and keep the raw value.
3. WHEN an engineering value is converted back to raw for a command, THE core SHALL reject values that are not exactly representable in the register type (never round or clamp silently).

## Requirement 8: profile schema and validation (R9, N2)

1. THE core SHALL ship a JSON Schema (`profiles/schema/buslink-profile.schema.json`) for profile schema v1 and Rust types for every field.
2. WHEN a profile is loaded, THE core SHALL validate it with the same rules as `tools/hooks/check_profile.py`: strict keys, duplicate JSON keys rejected, support levels `generic | documented | simulated | hardware_verified`, `source` required for documented/hardware-verified, passing hardware evidence required for `hardware_verified`, unit ID 0 never a default, word order explicit for multi-register types, write bounds inside `valid_range`, commands only on `simulated_only` profiles.
3. THE Rust validator and the Python hook validator SHALL agree (same valid/invalid verdict and the same error paths) on every bundled profile and on a shared fixture set.
4. WHEN a profile file is larger than 1 MB, THE core SHALL reject it without parsing.
5. Profiles SHALL be data only: no scripts, no expressions beyond `raw × scale + offset`.

## Requirement 9: device instance versus profile (R9, R4)

1. THE core SHALL keep site settings (device ID, name, link, unit ID, poll interval, stale threshold, block-merging limits) in a device instance that references a profile by ID and version.
2. WHEN a device instance has unit ID 0, or a unit ID above 247 on an RTU link, THE core SHALL reject it.
3. WHEN a device instance references a profile with a different ID or version than the one supplied, THE core SHALL reject the pairing.

## Requirement 10: quality and age (R7)

1. Every sample SHALL carry one quality: GOOD, TIMEOUT, CRC_ERROR, EXCEPTION(code), STALE, DECODE_ERROR or DISCONNECTED, plus `age_ms`, a receive timestamp and an optional source timestamp.
2. WHEN a read fails, THE core SHALL keep the last good value with its original timestamp, and its age SHALL keep growing; the failed sample SHALL NOT carry a fresh timestamp for that value or a substitute zero.
3. WHEN a GOOD value is older than the stale threshold (default `max(3 × poll interval, 3 s)`) and no newer result exists, THE core SHALL emit STALE.
4. THE core SHALL map qualities to the OPC UA classes Good / Uncertain / Bad.

## Requirement 11: transports (R3, D2)

1. THE core SHALL define a `Transport` trait with write, read-with-timeout (a zero-byte timed read is a timeout, not end of stream), drain and close, and a disconnected error for detach.
2. THE core SHALL provide a scripted `FakeTransport` driven by a manual clock so protocol and scheduler tests are deterministic and do not sleep.
3. THE core SHALL provide a Modbus TCP client transport over `std::net`.
4. THE core SHALL refuse to open a second worker for a link key that is already owned (one master per bus).

## Requirement 12: one worker per link with a priority queue (R4, D3)

1. THE core SHALL run one worker per link that owns the transport; every read and write for that link SHALL go through its single queue.
2. THE worker SHALL serve commands before alarm reads and alarm reads before polling.
3. WHEN a command frame is executed, THE worker SHALL send it exactly once and SHALL NOT retry it, whatever the outcome.
4. WHEN the transport reports a lost link, THE worker SHALL mark every device on it DISCONNECTED, fail queued requests, and stop polling.

## Requirement 13: block merging (R5)

1. THE core SHALL merge registers of the same table into one read when they are contiguous (or within a configured maximum gap) and the block stays within 125 registers / 2000 bits, or a smaller device limit.
2. THE core SHALL never split one tag across two reads and SHALL never read an address that no tag covers unless the device's maximum gap explicitly allows it.

## Requirement 14: backoff and fairness (R4)

1. WHEN a device times out, THE core SHALL abort the rest of its cycle and back off exponentially (poll interval × 2ⁿ, capped, default cap 30 s); a successful cycle SHALL reset the backoff.
2. Reads MAY be retried a bounded number of times (default 1, only on timeout/CRC error, only when the device is not already in backoff). Writes are never retried.
3. WHILE one or more devices on a link are dead, every healthy device SHALL still be polled within `poll interval + Σ(other devices' blocks × (retries + 1) × timeout)`.
4. THE core SHALL count requests, GOOD results, timeouts, CRC errors, exceptions, decode errors, the last error and the achieved cycle time per device.

## Requirement 15: sample bus (D4)

1. THE core SHALL publish every sample on one broadcast bus so that UI, historian, LAN view and MQTT see identical values and qualities.
2. WHEN a subscriber lags behind, THE bus SHALL report the number of lost samples to that subscriber instead of dropping them silently.

## Requirement 16: command service (N1, R18, D8)

1. THE command service SHALL be the only code that builds FC05/06/15/16 PDUs.
2. THE command service SHALL follow `Idle → Armed → Previewed → Confirmed → Sent → ReadBack → Done | Failed`, with a single attempt per confirmation.
3. WHEN an operator arms a device, THE arming SHALL expire after 60 s idle (configurable) and SHALL be dropped on disconnect, reconnect, profile change or app background.
4. WHEN a command is previewed, THE service SHALL check, in order, arming, role, writability, bounds (`min`/`max`/`step`/`allowed_values`, reject never clamp), unit ID ≠ 0, and SHALL return a preview with device, unit ID, register doc address and PDU offset, old value, new value in engineering units, raw value and the encoded frame.
5. THE service SHALL send only after `confirm` with the ID of the current preview, and SHALL send at most once per confirmation.
6. AFTER sending, THE service SHALL read back and report verified, mismatch or unknown; a timed-out write SHALL be reported as effect unknown (read back, never retried).
7. Every transition SHALL append an audit record (time UTC, session, user, role, device, profile ID + version, function, address, old value, requested value, raw request, raw response or exception, read-back result); WHEN the audit record cannot be stored before sending, THE service SHALL NOT send.
8. Named profile commands SHALL only run on `simulated_only` profiles connected to a simulator link.
9. Acknowledgement and read-back SHALL be reported as separate fields (write acknowledged ≠ process reached target).

## Requirement 17: historian (R20, D9)

1. THE core SHALL store sessions, devices, tags, samples, events, command audit and outbox tables in SQLite (bundled) in WAL mode.
2. Non-GOOD samples SHALL be stored with their quality and a NULL value so that trends break at gaps.
3. THE command audit table SHALL be append-only (updates and deletes rejected by the database).
4. THE core SHALL insert samples in batches, apply a logging interval separate from the poll interval (quality changes always stored), and compute per-minute min/max/sum/count rollups from GOOD samples only.
5. Raw values SHALL be stored losslessly (registers as bytes, 64-bit integers exact).

## Requirement 18: export (R21, N2)

1. THE core SHALL export CSV (UTF-8, ISO-8601 UTC, quality, exception code and age columns), XLSX (sheets Measurements, Devices, Configuration, Events, Commands; numbers as numbers) and JSON (64-bit integers as strings).
2. WHEN a text cell starts with `=`, `+`, `-`, `@`, tab or carriage return, THE export SHALL prefix it with `'` in CSV and XLSX.
3. Every exported measurement SHALL include the profile ID and version.
4. Exports SHALL never contain secrets.
