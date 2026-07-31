# Pro profile reconnect and Direct Windows hardware evidence — 2026-07-31

## Environment

- OS: Windows 11 25H2, build 26200.8875
- adapter: CSR8510 A10, USB VID/PID `0A12:0001`
- driver: WinUSB
- console: Nintendo Switch 2
- console system version: `22.5.0` (user-reported)
- controller model: Pro Controller
- identity: adapter-default
- Bumble fork: `niart120/bumble-rs:fix/external-host-reader-lifecycle`
- Bumble revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`

The environment metadata reuses the adapter inspection retained by M5. The user reported the
current console system version before these runs. The runner marks the normal, post-power-cycle,
and stale-bond operator setup claims as not machine-verified.

Raw profile documents, profile paths, peer addresses, and link keys are not stored in this
directory. The successful seed profile remained under the OS temporary directory while the runs
were performed. The stale-bond target was a separate create-new copy with one link-key nibble
changed; it was deleted after run 14. Runner records contain only the document size and boolean
preflight/postflight results.

## Results

Runs 1–2 established the seed profile. Runs 3–14 retain the reconnect attempts made while the
implementation was diagnosed. A failed attempt was not replaced after a fix.

| run | evidence | mode/setup | Ready or expected failure (ms) | final result | total (ms) | UI record |
|---:|---|---|---:|---|---:|---|
| 1 | `seed-pairing.ndjson` | fresh Periodic seed | timeout at 60000 | failed seed attempt | 60696 | none |
| 2 | `seed-pairing-02.ndjson` | fresh Periodic seed | Ready 5301 | success | 11871 | none |
| 3 | `periodic-reconnect-03.ndjson` | Periodic/normal | connection failed | failed before authentication fix | 6255 | none |
| 4 | `diagnostic-periodic-reconnect-04.ndjson` | Periodic/normal | connection failed | diagnostic failure | 6273 | none |
| 5 | `diagnostic-periodic-reconnect-05.ndjson` | Periodic/normal | connection failed | ACL without authentication/encryption | 6190 | none |
| 6 | `periodic-reconnect-06.ndjson` | Periodic/normal | connection failed | encryption without outgoing HID channels | 6155 | none |
| 7 | `diagnostic-periodic-reconnect-07.ndjson` | Periodic/normal | connection failed | outgoing HID diagnostic failure | 6119 | none |
| 8 | `periodic-reconnect-08.ndjson` | Periodic/normal | Ready 3227 | success | 9101 | user |
| 9 | `periodic-power-cycle-09.ndjson` | Periodic/post-power-cycle | Ready 3765 | success | 9651 | user |
| 10 | `direct-reconnect-10.ndjson` | Direct/normal | timeout at 60000 | failed before Direct readiness fix | 60351 | none |
| 11 | `direct-reconnect-11.ndjson` | Direct/normal | timeout at 60000 | reproduced after renewed operator setup | 60351 | none |
| 12 | `direct-diagnostic-12.ndjson` | Direct/normal | timeout at 60000 | protocol readiness diagnostic failure | 60357 | none |
| 13 | `direct-reconnect-13.ndjson` | Direct/normal | Ready 3160 | success | 9367 | user |
| 14 | `stale-bond-14.ndjson` | Periodic/stale-bond | expected failure 756 | success | 1444 | not applicable |

Runs 8, 9, and 13 completed the positive reconnect sequence. Each ended with a neutral snapshot,
clean close, adapter reopen, and exact 417-byte profile equality. Run 9 used the operator's
post-power-cycle setup. The machine record does not independently prove that the console was
power-cycled.

Direct run 13 accepted three bootstrap input reports before Ready. Its 500 ms Ready-state idle
window accepted zero additional user input reports. A, L+R, and each direction of both sticks then
accepted an explicit press/state and neutral pair. Explicit neutral added one report and close
added one final neutral report. The profile remained byte-for-byte unchanged.

Stale-bond run 14 changed one nibble only in a separate target profile, reached the expected
`connection_failed` result with authentication reason `0x05`, and accepted zero input reports.
Both the correct source and stale target remained byte-for-byte unchanged through close and adapter
reopen. The failure did not delete the bond or fall back to fresh pairing.

The three UI records are user observations, not runner assertions. For Periodic run 8,
post-power-cycle Periodic run 9, and Direct run 13, the user reported A, L+R, both sticks, and no
residual input after neutral. Every machine runner record keeps `ui_observed: null`.

## Diagnosis and fixes

Three production gaps were isolated. `diagnosis-comparison.ndjson` is a reduced record of the
temporary secret-free stderr observations; the raw diagnostic stderr was not retained.

1. Active stored-key reconnect established the expected ACL and found the profile key, but did not
   request authentication or enable encryption. Run 5 then disconnected with reason `0x13`.
   Active reconnect now requests authentication and enables Classic encryption after successful
   authentication. Incoming reconnect keeps the peer-driven path.
2. After authentication and encryption, active reconnect did not initiate the outgoing HID L2CAP
   channels. Run 6 still disconnected without Control or Interrupt. The implementation now opens
   Control after encryption, opens Interrupt after Control, reports each open transition once, and
   still accepts an incoming channel that races the outgoing attempt. Periodic run 8 was the first
   complete hardware Green after this change.
3. Direct run 12 completed stored-key lookup, ACL, authentication, encryption, both HID channels,
   bootstrap acceptance, and a subcommand reply, but remained pending on protocol readiness. The
   first subcommand stopped Direct bootstrap before the console sent nonzero player lights.
   Direct handshake now retries bootstrap until protocol readiness, while Periodic retains its
   existing first-subcommand completion rule. Run 13 reached Ready and then demonstrated zero
   automatic Direct reports during its idle window.

The Direct regression test reproduces the hardware order: bootstrap, report-mode request, a
readiness-only bootstrap retry, player-lights request, Ready, and no later idle send. The Classic
regression tests cover authentication, encryption, the first outgoing HID channel, and an
already-encrypted ACL that must skip redundant authentication. The virtual same-profile test
continues to cover Periodic then Direct reuse and Direct transactional input.

## Evidence boundary and residual risk

The positive result is limited to three successful reconnect runs on one Windows host, one adapter,
one Pro profile, and one Switch 2 system version. It is not a long-run reliability rate. Runs 3–7
and 10–12 are retained because they show the evolving implementation, not independent repeated
trials of the final implementation.

Joy-Con L/R, other adapters, other console system versions, Linux, macOS, long-run timing, and
explicit local Bluetooth addresses were not tested. No upstream Bumble pull request was created;
the permitted public fork and existing branch revision remained pinned without an additional fork
change.
