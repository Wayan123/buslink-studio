# BusLink device profile schema, version 1

Status: draft written during workspace setup (2026-11-11). The spec session should adopt or amend it; `tools/hooks/check_profile.py` is the executable definition and must change together with this page.

A profile is one JSON file in `profiles/` that describes one device model. It is **data, never code**: keys are strict, unknown keys are errors, and there is no script, expression or template field. The only transform is `engineering = raw × scale + offset`.

Instance settings (which COM port or USB adapter, the unit ID actually used on site, location, names) belong to the project file, not to the profile. One profile serves many devices.

## Top level

| Key | Req. | Type / values | Notes |
|---|---|---|---|
| `schema_version` | yes | `1` | |
| `profile_id` | yes | `^[a-z0-9]+([._-][a-z0-9]+)*$`, 3–64 chars | Stable ID, e.g. `bgt.smps-8in1`. |
| `profile_version` | yes | `MAJOR.MINOR.PATCH` | Any mapping change makes a new version. Every sample and export stores it. |
| `vendor`, `model` | yes | text ≤ 80 | |
| `category`, `firmware` | no | text | `firmware` = range the mapping applies to. |
| `description`, `notes` | no | text, or `{ "en", "id" }` | |
| `support_level` | yes | `generic` · `documented` · `simulated` · `hardware_verified` | Overall label shown in the UI. |
| `capabilities` | no | `{ read, write, comm_config, control }`, each a level or `unsupported` | Read support does not imply write or control support. |
| `simulated_only` | no | bool | `true` for devices that exist only in the simulator (VFD, temperature controller). |
| `source` | if documented / hardware_verified | `{ document, revision, url?, pages?, retrieved?, publisher? }` | The manual the mapping came from. |
| `evidence` | if hardware_verified | `{ hardware: [ { date, adapter, platform, record, result, unit_id?, firmware?, notes? } ] }` | At least one `result: "pass"` with a `record` (path or URL of the log or video). |
| `transport` | yes | `{ rtu?, tcp?, simulator? }` | Defaults only; never assume they are universal. |
| `registers` | yes | list, 1–2000 | See below. |
| `commands` | no | list | Control actions. **Only on `simulated_only` profiles in v1.** |
| `comm_config` | no | `{ unit_id?, baud? }` | For the commissioning wizard. |

### Support levels

- `generic`: mapping typed in by a user; no identification claim.
- `documented`: derived from a named manual revision and checked statically.
- `simulated`: codec and operations pass against the built-in simulator. This is not evidence for hardware.
- `hardware_verified`: a specific model + firmware + adapter + platform combination has a recorded passing test. A `simulated_only` profile can never reach this level.

## `transport`

- `rtu`: `unit_id` 1–247 (0 is broadcast and is never a default), `baud` 300–3 000 000 (warning if non-standard), `data_bits` 7|8, `parity` `none`|`even`|`odd`, `stop_bits` 1|2, optional `response_timeout_ms` 50–10 000 and `inter_frame_ms` 0–1000.
- `tcp`: `unit_id` 1–255, `port` 1–65535, optional `response_timeout_ms`.
- `simulator`: `{ "model": "<simulator id>" }`.

## Register entries

| Key | Req. | Values | Notes |
|---|---|---|---|
| `id` | yes | `^[a-z][a-z0-9_]{0,47}$`, unique | |
| `name` | yes | text ≤ 80 or `{ "en", "id" }` | Both languages required when bilingual. |
| `table` | yes | `coil` · `discrete_input` · `input_register` · `holding_register` | |
| `address` | yes | integer 0–65535 | **Zero-based PDU offset.** Never the 4xxxx label. |
| `doc_address` | no | string or int | The label exactly as printed in the manual (e.g. `"40001"`). |
| `type` | yes | `bool` · `u16` · `i16` · `bitfield` · `u32` · `i32` · `f32` · `u64` · `i64` · `f64` · `string` | `bool` only in coil/discrete tables, never in register tables. |
| `length` | string only | registers, 1–125 | |
| `word_order` | 32/64-bit: yes | `ABCD` · `CDAB` · `BADC` · `DCBA` | Strings may use `ABCD` or `BADC`. Not allowed on 16-bit types. See `.kiro/steering/modbus.md`. |
| `scale`, `offset` | no | number (scale ≠ 0) | Defaults 1 and 0. |
| `unit`, `decimals` | no | text ≤ 16, int 0–9 | |
| `valid_range` | no | `[min, max]` in engineering units | Out of range → `DECODE_ERROR`. |
| `display_range` | no | `[min, max]` in engineering units, min < max | Scale of the dashboard analog bar when `valid_range` is absent. Display only; never changes decoding or quality. With neither range the bar is hidden. |
| `invalid_sentinels` | no | list of raw ints | E.g. `32767` for "sensor open". |
| `bits` | bitfield only | `{ "0".."15": label }` | |
| `poll` | no | `fast` · `normal` · `slow` | Scheduling hint. |
| `source_page` | no | text ≤ 40 | Page or table in `source.document`. |
| `access` | no | `read` (default) · `read_write` | `read_write` only in `coil` and `holding_register`. |
| `write` | if read_write | `{ min, max, step?, allowed_values?, risk, role, confirm?, readback? }` | `risk`: `low`/`medium`/`high`; `role`: `operator`/`commissioner`. Numeric writes need `min`+`max` (inside `valid_range`) or `allowed_values`. `confirm` and `readback` may only be `true`. |

Registers that overlap in the same table produce a warning (aliases are allowed but must be deliberate).

## `commands` (simulated devices only in v1)

`{ id, kind, label, register, readback_register, value?, expected?, timeout_ms?, feedback? }`

- `kind`: `run` · `stop` · `setpoint` · `reset_fault` · `mode`.
- `register` must be a `read_write` register of the same profile; `readback_register` is required.
- `setpoint` has no fixed `value`: the operator enters it and the register's `write` bounds apply. Other kinds need a numeric `value`.
- `feedback` (added 2026-11-13, commissioning-control): `{ register, tolerance, timeout_ms, value?, bit? }`. The live register that shows whether the **process** reached the target, which is a different question from the read-back of the written register. `register` is any register of the profile; `tolerance` ≥ 0 in engineering units; `timeout_ms` 100–3 600 000. `value` is the target; it may be omitted only for `setpoint` commands (target = the entered value). `bit` 0–15 compares one bit of a bitfield register with `value` 0 or 1.
- The command service still applies arming, confirmation, read-back and audit (`.kiro/steering/safety.md`).

## `comm_config`

`{ "unit_id": { register, apply }, "baud": { register, apply, codes: { "<raw>": <baud> } } }` with `apply` = `immediate` · `after_reboot` · `unknown`. The register must be `read_write`. If `apply` is `unknown`, the wizard stops at "needs verification" instead of retrying.

## Validation

```sh
python3 tools/hooks/check_profile.py profiles/<file>.json        # one file
find profiles -name '*.json' -exec python3 tools/hooks/check_profile.py {} +
```

The `profile-schema-check` hook runs the same check when a file under `profiles/` is saved.

Three validators implement this page and must stay in step:

- `profiles/schema/buslink-profile.schema.json` (JSON Schema 2020-12) covers structure, enums and ranges, for editors. Files under `profiles/schema/` are not profiles and the hook skips them.
- `tools/hooks/check_profile.py` (hook, stdlib only). `--json` prints one JSON verdict per file.
- `crates/buslink-core/src/profile/validate.rs`, used by the app when loading a profile. `crates/buslink-core/tests/profile_agreement.rs` checks that it gives the same verdict and the same error and warning paths as the Python script, on the bundled profiles, the fixtures in `crates/buslink-core/tests/fixtures/profiles/` and several hundred mutated documents.

Strictness rules added with the Rust validator (2026-11-12), applied by both:

- `null` is never a value: omit the key instead.
- Enum values are type-strict: `8.0` is not `8` and `true` is not `1`.
- IDs, versions, dates, bit indexes and baud codes are matched over the whole string, ASCII only (so `"1.0.0\n"` and non-ASCII digits are rejected).
- `NaN` and `Infinity` are not JSON; integers must fit in 64 bits.
- `$schema` and `$comment` must be strings; command `expected` must be a number; `source.publisher` and evidence `firmware` and `notes` are checked as text.
