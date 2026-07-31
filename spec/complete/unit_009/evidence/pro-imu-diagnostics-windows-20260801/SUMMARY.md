# Pro IMU and diagnostics Windows hardware evidence — 2026-08-01

## Environment

- operating system: Windows 11 25H2 (same host as the M6/M7 evidence); the current process reported
  kernel build `26200`
- architecture: `x86_64`
- adapter: CSR8510 A10, USB VID/PID `0A12:0001`
- driver: WinUSB
- console: Nintendo Switch 2
- console system version: `22.5.0` (user-reported)
- controller model and reporting: Pro Controller, Periodic
- identity: adapter-default, schema v2 stored bond
- Rust: `rustc 1.87.0 (17067e9ac 2025-05-09)`
- Bumble fork revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`
- probe implementation: `2d93d53` for run 04; horizontal-yaw adjustment `a86d414` for run 05

The profile path, Bluetooth addresses, link key, USB serial, raw HID/HCI packets, and Bumble error
source chain are not retained in this directory.

## Run record

| run | purpose | result | retained evidence |
|---:|---|---|---|
| 01 | reconnect with the older Pro profile | failed during authentication with disconnect reason `0x05`; no Ready state | trace, classified stderr, CPU wrapper record |
| 02 | create a fresh Pro profile after run 01 | Ready, input reports, neutral close, and profile persistence succeeded | pair trace and completion |
| 03 | first reconnect attempt with the fresh profile | connection timeout after 60 seconds; no Ready state | trace, classified stderr, CPU wrapper record |
| 04 | 60-second IMU/diagnostics/timing run after renewed operator setup | succeeded | trace, completion, CPU record, analyzer summary, UI record |
| 05 | 15-second pure horizontal-yaw behavior run using the stored profile | succeeded | trace, completion, analyzer summary, UI record |

Runs 01 and 03 are retained as attempts, not as repetitions of the successful implementation. The
empty completion files show that those invocations did not produce a success record. Run 02 changed
the console-side bond. The pre-existing profile used by run 01 was still valid JSON but could no
longer authenticate after the adapter-default identity had been paired as another controller model.
This is a diagnosis from the observed disconnect and pairing history, not a direct inspection of the
console's stored bond database.

## Run 04: 60-second machine and timing evidence

The probe reached Ready in `3,506,787,900 ns`, observed report mode `0x30` and committed IMU mode
`0x02`, applied a fixed non-neutral IMU frame for 60 seconds, accepted 5,343 non-neutral reports,
accepted a neutral report, closed neutrally, preserved the profile byte-for-byte, and reopened the
adapter. Shutdown latency was `16,483,100 ns`.

The analyzer parsed 5,411 closed-schema records. It found no forbidden field and observed the
expected subcommands `0x02`, `0x03`, `0x04`, `0x08`, `0x10`, `0x21`, `0x30`, `0x40`, and `0x48`.
For 5,359 subscriber-observed input-report intervals, the distribution was:

| metric | p50 | p95 | p99 | maximum |
|---|---:|---:|---:|---:|
| interval | 8.5495 ms | 17.0223 ms | 17.4667 ms | 321.9072 ms |
| absolute error from 8 ms | 1.0148 ms | 9.0223 ms | 9.4667 ms | 313.9072 ms |

There were 1,231 intervals at or above 16 ms, an estimated 1,306 missed 8 ms periods, and two
intervals below 4 ms. The two intervals above 100 ms were `321.9072 ms` and `312.4126 ms`; both
occurred in the startup/reply portion rather than as a repeated steady-state class. The process used
`1,078,125,000 ns` of CPU during a `60,449,467,200 ns` window, or 1.784% of one logical core.

The user observed the A tap and no residual input. IMU movement was not observed because the console
was not on a screen where gyro movement was visible. This is recorded as unobserved, not as a
no-movement result.

## Run 05: horizontal-yaw behavior evidence

The run used the Project_Demi diagnostic pattern as a behavioral reference without starting
Project_Demi or using its bond: neutral acceleration `(0, 0, 1 g)` and a constant positive Z-axis
angular rate of `1.0 rad/s`. The probe reached Ready in `2,255,435,400 ns`, accepted 1,364
non-neutral reports during 15 seconds, accepted a neutral report, closed neutrally, preserved the
profile byte-for-byte, and reopened the adapter. Shutdown latency was `20,247,100 ns`.

For 1,366 subscriber-observed intervals, p50 was `8.5060 ms`, p95 `16.6487 ms`, p99 `17.1433 ms`,
and maximum `18.0418 ms`. There were 299 intervals at or above 16 ms and one interval below 4 ms.
The user observed horizontal movement, no visible stutter, and no residual movement or other input
after the probe neutralized and closed.

## Evidence boundary and release risk

`trace_elapsed_ns` measures when the trace subscriber observed an event after runtime status
projection. It is not an HCI completion time, radio-delivery time, or console-render time. The
60-second run and 15-second behavior run are separate observations: run 04 measured CPU but did not
have an observable gyro screen; run 05 had an observable screen but did not collect CPU.

Both successful traces have p95 interval error greater than one 8 ms target period. The run 05 UI
observation found no visible stutter, but that does not remove the measured scheduler/transport
variation. Under the roadmap severity table this remains an S2 release limitation for M9 to accept,
reduce, or document. One successful 60-second run and one successful 15-second run do not establish
a production reliability rate.

The machine analyzer confirmed valid NDJSON, monotonic timestamps, the closed event-field contract,
and absence of the configured forbidden fields. It cannot prove that arbitrary secret byte patterns
are absent; the implementation prevents profile, address, key, serial, raw packet, and source-chain
values from entering the stable event schema.
