# Pro Periodic Windows hardware evidence — 2026-07-30

## Environment

- OS: Windows 11 25H2, build 26200.8875
- adapter: CSR8510 A10, USB VID/PID `0A12:0001`
- driver: WinUSB
- local Bluetooth address: `00:1B:DC:F9:9F:7D`
- HCI version: `0x06`
- LMP version: `0x06`
- company identifier: `0x000A`
- HCI/LMP subversion: `0x22BB`
- console: Nintendo Switch 2
- console system version: `22.5.0` (user-reported before the hardware run)
- Rust Bumble revision: `48f1bc36169b2692d2a61e87eda4223b126dca2b`

Raw profile documents and link keys are not stored in this directory. Each fresh attempt used a
new profile under the OS temporary directory. Diagnostic stderr containing the temporary profile
path was reduced to `acl-drain-comparison.ndjson` and was not retained.

## Results

`run-01` and `diagnostic-02` through `diagnostic-13` used new profile paths.
`diagnostic-reconnect-14` through `diagnostic-reconnect-16` reused the profile created by
`diagnostic-10` to isolate close behavior; they do not count as fresh-pairing successes.

| evidence | pairing mode | pair timeout (ms) | Ready (ms) | final result | total (ms) |
|---|---|---:|---:|---|---:|
| `run-01.ndjson` | fresh | 60000 | — | create-profile connection failed | 3275 |
| `diagnostic-02.ndjson` | fresh | 60000 | — | create-profile connection failed | 5772 |
| `diagnostic-03.ndjson` | fresh | 60000 | — | create-profile connection failed | 3151 |
| `diagnostic-04.ndjson` | fresh | 60000 | — | create-profile connection failed | 3111 |
| `diagnostic-05.ndjson` | fresh | 60000 | — | create-profile connection failed | 3249 |
| `diagnostic-06.ndjson` | fresh | 60000 | — | create-profile connection failed | 4540 |
| `diagnostic-07.ndjson` | fresh | 60000 | — | create-profile connection failed | 3260 |
| `diagnostic-08.ndjson` | fresh | 60000 | — | create-profile timeout | 60709 |
| `diagnostic-09.ndjson` | fresh | 10000 | — | create-profile timeout | 10701 |
| `diagnostic-10.ndjson` | fresh | 60000 | 5298 | Ready and input succeeded; close failed | 12866 |
| `diagnostic-11.ndjson` | fresh | 60000 | — | create-profile timeout | 60681 |
| `diagnostic-12.ndjson` | fresh | 60000 | — | create-profile connection failed | 12149 |
| `diagnostic-13.ndjson` | fresh | 60000 | — | create-profile timeout | 60694 |
| `diagnostic-reconnect-14.ndjson` | existing profile | 60000 | 4527 | Ready and input succeeded; close failed | 12095 |
| `diagnostic-reconnect-15.ndjson` | existing profile | 60000 | — | pairing timeout | 60683 |
| `diagnostic-reconnect-16.ndjson` | existing profile | 60000 | 5499 | success | 12174 |

No failed run was replaced by a successful rerun. `diagnostic-10` proves that a fresh SSP session
reached NX Ready with report mode `0x30`, player lights, 16 accepted subcommand replies, and a valid
Pro profile. Its explicit close failed after the input sequence, but the adapter reopened.

## Diagnosis and fixes

Three independent gaps were observed.

1. Without Classic default link policy `0x0005` (role switch and sniff mode), the Switch closed the
   ACL link with reason `0x13` before sending the NX subcommand sequence.
2. After accepting report mode `0x30`, automatic Periodic input did not start until full protocol
   readiness. The Switch stopped its initialization sequence while waiting for those reports.
3. Bumble accepted reports into an internal ACL queue even when the controller's eight-packet HCI
   window was full. Before transport backpressure, run 14 entered close with 251 pending ACL
   packets and retained 163 after the one-second drain deadline. After automatic reports skipped
   full-window ticks, run 16 entered close with 11 pending packets, drained to zero, closed in
   131 ms, and reopened the adapter.

The queue comparison is stored in `acl-drain-comparison.ndjson`. Run 16 accepted 553 input reports,
completed the typed input sequence, retained a neutral snapshot, closed cleanly, and ended with
`runner_complete success=true`.

## Reference control and evidence boundary

The pinned `swbt-python` revision `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` with
Bumble `0.0.233` passed its fresh-pairing hardware control on the same Switch and adapter
(`1 passed in 5.91s`). Raw Python trace and profile artifacts were not copied because they contain
pairing material.

All runner records retain `ui_observed: null`. Report acceptance, typed command completion, neutral
snapshot, close, and adapter reopen are machine observations. The separate
`ui-observation-run-16.ndjson` records the user's post-run observation: A, L+R, left stick, and
right stick were reflected in the Switch UI, and no residual input remained after neutral.
