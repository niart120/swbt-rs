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
- Rust Bumble base revision for runs 1–18:
  `48f1bc36169b2692d2a61e87eda4223b126dca2b`
- Rust Bumble fork revision for runs 19–20:
  `b8c7cd625bc2ac2f58a4beb4ade1264426969819`

Raw profile documents and link keys are not stored in this directory. Each fresh attempt used a
new profile under the OS temporary directory. Diagnostic stderr containing the temporary profile
path was reduced to `acl-drain-comparison.ndjson` and was not retained.

## Results

The fresh-attempt sequence is `run-01`, `diagnostic-02` through `diagnostic-13`, and `run-14`
through `run-20`. Every attempt used a distinct absent profile path. The diagnostic filenames are
retained because failures were not replaced after each fix.

| attempt | evidence | pair timeout (ms) | Ready (ms) | final result | total (ms) |
|---:|---|---:|---:|---|---:|
| 1 | `run-01.ndjson` | 60000 | — | create-profile connection failed | 3275 |
| 2 | `diagnostic-02.ndjson` | 60000 | — | create-profile connection failed | 5772 |
| 3 | `diagnostic-03.ndjson` | 60000 | — | create-profile connection failed | 3151 |
| 4 | `diagnostic-04.ndjson` | 60000 | — | create-profile connection failed | 3111 |
| 5 | `diagnostic-05.ndjson` | 60000 | — | create-profile connection failed | 3249 |
| 6 | `diagnostic-06.ndjson` | 60000 | — | create-profile connection failed | 4540 |
| 7 | `diagnostic-07.ndjson` | 60000 | — | create-profile connection failed | 3260 |
| 8 | `diagnostic-08.ndjson` | 60000 | — | create-profile timeout | 60709 |
| 9 | `diagnostic-09.ndjson` | 10000 | — | create-profile timeout | 10701 |
| 10 | `diagnostic-10.ndjson` | 60000 | 5298 | Ready and input succeeded; close failed | 12866 |
| 11 | `diagnostic-11.ndjson` | 60000 | — | create-profile timeout | 60681 |
| 12 | `diagnostic-12.ndjson` | 60000 | — | create-profile connection failed | 12149 |
| 13 | `diagnostic-13.ndjson` | 60000 | — | create-profile timeout | 60694 |
| 14 | `run-14.ndjson` | 60000 | 5332 | Ready and input succeeded; close failed | 12903 |
| 15 | `run-15.ndjson` | 60000 | 4578 | success | 11245 |
| 16 | `run-16.ndjson` | 60000 | 7044 | Ready and input succeeded; close failed | 18606 |
| 17 | `run-17.ndjson` | 60000 | 5222 | success | 11892 |
| 18 | `run-18.ndjson` | 60000 | 5288 | Ready and input succeeded; close failed | 12857 |
| 19 | `run-19.ndjson` | 60000 | 5261 | success | 11837 |
| 20 | `run-20.ndjson` | 60000 | 6547 | success | 13099 |

The historical success rate across the evolving implementation is 4/20 (20%). Eight attempts
reached same-session NX Ready (40%); eight ended with a connection failure, four timed out during
profile creation, and four reached Ready but failed explicit close. Runs 19–20, after the final ACL
flush fix, succeeded 2/2. This two-run result is evidence for the diagnosed close condition, not a
long-run reliability claim.

All 20 attempts emitted `runner_complete`, retained a valid typed profile, and reopened the adapter.
Total runtime ranged from 3111 to 60709 ms. None of the eight Ready attempts had a non-neutral
pre-close snapshot. User observations exist for fresh runs 16–20: all 5 reflected A, L+R, and both
sticks, with no residual input after neutral. The other attempts do not have a human UI record.

`diagnostic-reconnect-14` through `diagnostic-reconnect-16` reused the profile created by
`diagnostic-10` to isolate close behavior. They are retained as diagnostic evidence and are not
part of the 20 fresh attempts:

| evidence | Ready (ms) | final result | total (ms) |
|---|---:|---|---:|
| `diagnostic-reconnect-14.ndjson` | 4527 | Ready and input succeeded; close failed | 12095 |
| `diagnostic-reconnect-15.ndjson` | — | pairing timeout | 60683 |
| `diagnostic-reconnect-16.ndjson` | 5499 | success | 12174 |

## Diagnosis and fixes

Four independent gaps were observed.

1. Without Classic default link policy `0x0005` (role switch and sniff mode), the Switch closed the
   ACL link with reason `0x13` before sending the NX subcommand sequence.
2. After accepting report mode `0x30`, automatic Periodic input did not start until full protocol
   readiness. The Switch stopped its initialization sequence while waiting for those reports.
3. Bumble accepted reports into an internal ACL queue even when the controller's eight-packet HCI
   window was full. Before transport backpressure, run 14 entered close with 251 pending ACL
   packets and retained 163 after the one-second drain deadline. After automatic reports skipped
   full-window ticks, run 16 entered close with 11 pending packets, drained to zero, closed in
   131 ms, and reopened the adapter.
4. The close drain predicate waited for every in-flight packet to receive controller flow-control
   credit. On the observed CSR controller, final credit remained outstanding after every host-side
   packet had entered its window. Fresh run 16 still had 2 of 11 credits after 5 seconds; run 18 had
   4 of 10 after 1 second. The fork adds a distinct host-queue-flushed predicate without changing
   the existing controller-acknowledged predicate. Run 19 flushed one host-waiting packet from 11
   pending to the 10-packet controller window and closed in 15 ms. Run 20 started and ended the
   flush at 10 in-flight packets and closed in 12 ms.

The queue comparisons are stored in `acl-drain-comparison.ndjson` and
`acl-flush-comparison.ndjson`. The fork change is commit
`b8c7cd625bc2ac2f58a4beb4ade1264426969819` on
`niart120/bumble-rs:fix/external-host-reader-lifecycle`. No upstream pull request was created.

## Reference control and evidence boundary

The pinned `swbt-python` revision `84d2723b127f70fc78e12f4496f5c40af0ccfb0a` with
Bumble `0.0.233` passed its fresh-pairing hardware control on the same Switch and adapter
(`1 passed in 5.91s`). Raw Python trace and profile artifacts were not copied because they contain
pairing material.

All runner records retain `ui_observed: null`. Report acceptance, typed command completion, neutral
snapshot, close, and adapter reopen are machine observations. `ui-observation-run-16.ndjson`
contains the stored-profile diagnostic observation. `ui-observation-fresh-run-16.ndjson` through
`ui-observation-fresh-run-20.ndjson` contain the five fresh-run observations. Each records that A,
L+R, left stick, and right stick were reflected in the Switch UI and no residual input remained
after neutral.
