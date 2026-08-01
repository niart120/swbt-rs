# Explicit local-address Windows hardware evidence — 2026-08-01

## Evidence boundary

This directory retains only closed machine results and user-reported Switch UI observations. It does
not retain the explicit or adapter-default Bluetooth address, private profile path, profile JSON,
link key, USB serial, raw HCI/HID packet, or error source chain. The private profile, trace, and
six-byte adapter-default comparison baseline stay under ignored `target/` storage during the run and
are removed after the final recovery check.

Machine results and Switch UI observations are recorded separately. A successful command does not
prove what appeared on the console, and a UI observation does not prove profile persistence,
namespace selection, neutral cleanup, adapter release, or identity recovery.

## Environment

- operating system: Windows 11 25H2, kernel build `26200`
- architecture: `x86_64`
- adapter: CSR8510 A10, USB VID/PID `0A12:0001`; one candidate found without opening it
- driver: WinUSB according to the same-host M8 evidence; the current administrative driver query was
  denied, so this value was not independently re-read in T08
- console: Nintendo Switch 2
- console system version: `22.5.0` (user-reported)
- controller: Pro Controller
- Rust: `rustc 1.96.1 (31fca3adb 2026-06-26)`
- swbt implementation before T08 evidence edits: `5234823becb4`
- Bumble public-fork revision: `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`
- pair/reconnect timeout: 60 seconds

## Safety sequence

The adapter-default address was recorded before the first write by an ignored `adapter-tests` test.
The test writes six private bytes to create-new storage under `target/`, emits only the action and
match result, and closes the adapter before returning.

Each connection run follows this sequence:

1. start only after the operator has prepared the corresponding Switch screen;
2. run one bounded connection operation using the private explicit-address profile;
3. retain only the closed completion and trace-derived counts, never the raw identity or profile;
4. obtain the Switch UI observation separately;
5. physically power-cycle the dongle;
6. compare its address read-only with the private adapter-default baseline;
7. do not start the next write unless the comparison succeeds.

If a run returns `adapter_identity_recovery_required` or otherwise fails after a possible write, the
run stops at step 4. No retry is allowed until steps 5 and 6 succeed.

## Run status

| run | operation | reporting | machine result | UI observation | power-cycle recovery |
|---:|---|---|---|---|---|
| B00 | read-only adapter-default baseline | none | six-byte baseline recorded and immediately matched without displaying the address | not applicable | not applicable |
| P01 | fresh profile pair | Periodic | stopped with `adapter_identity_recovery_required` before pairing | not observed; no pairing/input event reached | physical cycle completed; private adapter-default baseline matched read-only |
| P02 | fresh profile pair after deferred-close fix | Periodic | stopped with `adapter_identity_recovery_required` before pairing | not observed; no pairing/input event reached | physical cycle completed; private adapter-default baseline matched read-only |
| D03 | identity-preparation-only stage diagnostic | none | stopped at closed internal stage `WarmReset` after successful SETREQ response | not applicable; no advertising or pairing | physical cycle completed; private adapter-default baseline matched read-only |
| D04 | identity preparation after warm-reset settlement fix | none | `Rewritten`; re-enumeration, exact target read-back, and close completed in 2.07 seconds | not applicable; no advertising or pairing | physical cycle completed; private adapter-default baseline matched read-only |
| P05 | fresh profile pair after warm-reset settlement fix | Periodic | completed in 8.5 seconds; local-address identity, bond 1, namespace 1, 16 observed subcommands, 21 accepted reports, neutral close | A reflected; no residual input after exit | physical cycle completed; private adapter-default baseline matched read-only |
| R02 | stored-key reconnect | Periodic | completed in 5.5 seconds; local-address identity, profile bytes unchanged, 16 observed subcommands, 19 accepted reports, neutral close, 15.6 ms shutdown | A reflected; no residual input after exit | physical cycle completed; private adapter-default baseline matched read-only |
| R03 | stored-key reconnect | Direct | completed in 5.8 seconds; local-address identity, profile bytes unchanged, 16 observed subcommands, 6 accepted reports, neutral close, 4.47 ms shutdown | A reflected; no residual input after exit | physical cycle completed; private adapter-default baseline matched read-only |

Machine results and operator observations are separated in the table. Profile bytes, identity kind,
namespace and bond counts, neutral close, and shutdown are machine evidence; Switch UI input response
and residual-input absence are operator observations.

## P01 stopped attempt

P01 created a fresh schema v2 Pro profile with `identity_kind=local_address`, zero namespaces, and
zero bonds. It then returned `adapter_identity_recovery_required` in about one second. The trace
contains only the closed environment event; pairing, readiness, input, neutral close, and console UI
observation did not occur. The error record and trace contain no address, profile path, selector,
packet, key, or backend source text.

The elapsed time rules out the 10-second re-enumeration timeout but does not identify the closed
internal stage. This attempt is retained as a stopped safety result, not as evidence of successful
identity preparation. The operator physically power-cycled the dongle, after which the private
adapter-default baseline comparison succeeded without displaying either address.

Code comparison found that the Python baseline ignores the expected transport-close exception after
successfully enqueueing CSR warm reset, then decides success from re-enumeration and read-back. Rust
instead returned recovery-required immediately from that close failure. A new fake-backend test
reproduced the Rust result as `RecoveryRequired / Close`; the implementation now ignores only this
post-reset close result and still requires bounded re-enumeration, CSR metadata, target read-back, and
read-back-session close. This diagnosis is code- and test-backed, but the next hardware attempt must
confirm that it explains P01.

## P02 stopped attempt

P02 used a newly built probe containing the deferred-close fix and a fresh profile/trace pair. It
still returned `adapter_identity_recovery_required` in about one second. The resulting schema v2 Pro
profile has `identity_kind=local_address`, zero namespaces, and zero bonds. Its trace again contains
only the closed environment event, so pairing, readiness, input, neutral close, and console UI
observation did not occur.

P02 disproves the hypothesis that the first post-reset close result was the only cause. A new ignored
`adapter-tests` diagnostic now exercises only the same target identity preparation and reports either
success or the closed internal failure stage; it never displays the baseline or target address. It
was run once after physical recovery and stopped at `WarmReset` in 0.08 seconds. No advertising or
pairing was part of this diagnostic.

The Rust USB sink performs a synchronous control transfer with a one-second timeout, while the Python
baseline enqueues the same warm-reset command asynchronously and settles the outcome from later
re-enumeration and read-back. The target adapter can disappear before the synchronous transfer reports
success. A new test reproduced this as `RecoveryRequired / WarmReset` (red). The implementation now
continues from that old-handle result but still requires bounded re-enumeration, CSR metadata, exact
target read-back, and read-back-session close; the new test and all eight identity tests pass (green).
The D04 hardware run then returned `Rewritten` in 2.07 seconds without exposing either address. This
confirms identity preparation itself on the target adapter. Advertising, pairing, input, and physical
recovery are separate evidence; the operator subsequently completed a physical cycle and the private
adapter-default baseline matched read-only.

## R03 completed Direct reconnect

R03 reused the same P05 profile and stored bond. The machine result completed with
`operation=reconnect`, `reporting_kind=direct`, `identity_kind=local_address`,
`neutral_close=true`, and 4.47 ms shutdown latency. The profile bytes again exactly matched the
private post-pair comparison copy, with one namespace and one bond. The trace contains 16 observed
subcommands and accepted replies, six accepted reports, and one session start/end. The operator
observed the A input and no residual input after exit. After the final physical cycle, the private
adapter-default baseline matched read-only.

After the final recovery check, the private baseline, profile, post-pair comparison copy, and all
three connection traces were removed from ignored `target/` storage. The retained evidence contains
only the closed results and operator observations above.

## P05 completed pair

P05 used a fresh schema v2 Pro profile and trace. The machine result completed with
`operation=pair`, `reporting_kind=periodic`, `identity_kind=local_address`, and
`neutral_close=true`. Profile inspection reported one namespace and one bond. The trace contains one
session start/end, 16 observed subcommands and accepted replies, 21 accepted reports, and no public
address or key material. The operator observed the A input on the Switch UI and no residual input
after exit. After a physical cycle, the private adapter-default baseline matched read-only.

## R02 completed Periodic reconnect

R02 reused the P05 profile and stored bond. The machine result completed with
`operation=reconnect`, `reporting_kind=periodic`, `identity_kind=local_address`,
`neutral_close=true`, and 15.6 ms shutdown latency. The profile bytes exactly matched the private
post-pair comparison copy, with one namespace and one bond. The trace contains 16 observed
subcommands and accepted replies, 19 accepted reports, and one session start/end. The operator
observed the A input and no residual input after exit. After a physical cycle, the private
adapter-default baseline matched read-only.
