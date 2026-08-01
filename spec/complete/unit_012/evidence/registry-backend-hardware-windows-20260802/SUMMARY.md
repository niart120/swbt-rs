# Registry backend Windows hardware evidence — 2026-08-02

## Scope and privacy

This evidence covers the `swbt-rs@55d4ccd` integration against the
`swbt-bumble-backend` fixes at `fa40553`. The tested backend source was supplied through a temporary
local Cargo patch because crates.io still contained 0.1.0. The published 0.1.1 source at
`0a4a2d99bc3ed3807464d4f902c20d9fd16b188a` adds only version, lockfile, and changelog metadata after
`fa40553`. Its crates.io checksum is
`1cc2c8d7d9c8cecfd203cd039fb3c3f8a9c39b072230f977b1e12e526b1bc667`, and the final `swbt-rs`
lockfile resolves that registry artifact without a path patch.

The environment was Windows 11 25H2 x86_64, CSR8510 A10 `0A12:0001` with WinUSB, Switch 2 system
version 22.5.0 as reported by the operator, and one Pro Controller. This summary does not retain the
explicit or adapter-default Bluetooth address, profile path, profile JSON, link key, USB serial, or
raw HCI/HID packets. Machine evidence and UI observations are reported separately.

## Regressions found before the successful matrix

| backend source | machine result | diagnosis |
|---|---|---|
| crates.io 0.1.0 | fresh explicit-address pair stopped at `transport_open` | USB open/claim succeeded, but HCI `0x2001` returned status `0x12` |
| `122a685` | fresh pair reached Ready, stored one bond in one namespace, accepted 19 reports, and closed; probe returned `internal` after the operation | HCI version 6 now used the legacy LE event mask; A was visible and no residual input remained |
| `122a685` | stored-key Periodic reconnect reached Ready; A tap returned `internal`, close returned `worker_failed`, profile remained byte-identical, and adapter reopen succeeded | source chain ended in backend `SendRejected` while sending a trailing neutral report |

`122a685` restored the original Bumble behavior of using the legacy LE event mask for HCI version 6
or earlier. `fa40553` restored the previous explicit-send contract: automatic Periodic reports still
observe the ACL capacity hint, while tap release and close neutral reports can enter the host-side ACL
queue behind in-flight controller credit. Scripted regressions were red before each fix and green
afterward.

## Successful matrix at backend `fa40553`

| operation | machine result | UI observation |
|---|---|---|
| stored-key Periodic reconnect after physical dongle power-cycle | Ready in 2.018 s; A, L+R, four left-stick directions, four right-stick directions, explicit neutral, and trailing neutral all completed; 479 reports accepted; no worker failure; profile byte-identical; adapter reopen succeeded | A, L+R, both sticks reflected; no residual input |
| stored-key Periodic reconnect with 60 s horizontal-yaw IMU | 5,457 non-neutral reports; 1 neutral report; command latency 23.9 us; shutdown 30.308 ms; neutral close, profile equality, and adapter reopen succeeded | not separately retained |
| immediate repeat of the 60 s IMU run using the already-active explicit identity | 5,382 non-neutral reports; 1 neutral report; command latency 18.2 us; shutdown 21.517 ms; neutral close, profile equality, and adapter reopen succeeded | horizontal movement present; no stutter; no movement or input remained after exit |
| stored-key Direct reconnect after physical dongle power-cycle | Ready in 2.759 s; Direct idle emitted 0 reports; A, L+R, four left-stick directions, four right-stick directions, explicit neutral, and trailing neutral completed; 25 reports accepted; no worker failure; profile byte-identical; adapter reopen succeeded | A, L+R, both sticks reflected; no residual input |

The operator reported each requested physical power-cycle. The runner labels that step as operator
setup rather than machine verification. After the final reported power-cycle, an adapter-default
open/initialize/close command succeeded. This proves the adapter remained reusable, but this unit did
not retain or compare the private adapter-default address. Exact address recovery was already covered
by unit_011 and is not inferred from this open result. The final operator report after the input run
confirmed A, L+R, both sticks, no residual input, and completion of the requested physical
power-cycle.

After 0.1.1 was published and resolved directly from crates.io, the first adapter-only
open/initialize/close sentinel returned `transport_open` while another controller application was
still running. Descriptor-only discovery still found the adapter. After the operator closed that
application, the process scan was empty and the same registry-backed binary emitted
`adapter_opened`. This controlled retry is consistent with the documented exclusive WinUSB owner;
it is not counted as an additional pairing or input run.

## Cleanup and retained evidence

The successful trace summaries contained one Ready and one Closed session each, zero `worker_failed`
events, and the expected final report counters. The fresh-pair trace also ended Closed with zero
`worker_failed` events. After this summary was written, the temporary profiles and raw traces were
deleted. Temporary diagnostic examples and Cargo path patches are not product changes and are removed
before unit completion.
