# M9 workload soak / backend rollback evidence — 2026-08-02

## Scope and privacy

This evidence covers the release-candidate workload soak and the operational switch from the Rust
backend to the pinned `swbt-python` 0.6.0 baseline. The environment is the same Windows 11 25H2
x86_64, CSR8510 A10 `0A12:0001` WinUSB adapter, Switch 2 version 22.5.0, and Pro Controller recorded
in the [unit_012 registry-backend evidence](../../../complete/unit_012/evidence/registry-backend-hardware-windows-20260802/SUMMARY.md).

The test used a private Project_Demi profile only through an ignored copy under `target/`. This
record does not retain the profile path, Bluetooth addresses, link key, profile bytes, raw trace, or
USB serial. The original profile and the working copy both remained byte-identical to the baseline
copy after the attempted and successful reconnects.

## Workload soak

The immediately repeated 60-second horizontal-yaw runs already retained by unit_012 are the M9
workload soak. They exercised the same registry backend selected for the release candidate.

| run | machine result | operator observation |
|---|---|---|
| first 60 s Periodic IMU run | 5,457 non-neutral reports, 1 neutral report, 23.9 us apply latency, 30.308 ms shutdown, profile byte equality and adapter reopen succeeded | not separately retained |
| immediate repeat | 5,382 non-neutral reports, 1 neutral report, 18.2 us apply latency, 21.517 ms shutdown, profile byte equality and adapter reopen succeeded | horizontal movement present, no stutter, no movement or input remained after exit |

This is a two-minute consecutive functional soak. It proves the stated runs and does not establish
long-term reliability, other adapters, other operating systems, or radio-interference tolerance.

## Python profile compatibility regression

The first Rust verification of the copied Python 0.6.0 profile failed as `invalid_profile` before
opening the adapter. A secret-free structural inspection and a successful Python
`PairingProfile.load` isolated the first incompatibility: Bumble serializes a public Classic peer
name as `AA:BB:CC:DD:EE:FF/P`, while the Rust validator and bond-store bridge accepted only the raw
17-character address.

After accepting the `/P` peer suffix, the first reconnect reached the backend and failed as
`invalid_key_store`. Python's `PairingKeys.to_dict()` omits `address_type` for a Classic link key
because the `/P` peer name carries the address kind. Rust had required `address_type: 0` in the bond
object.

The deterministic regressions now cover these behaviors:

- Python 0.6.0 fixtures use `/P` public peer names and parse as every supported controller model.
- namespaces remain raw Bluetooth addresses; `/R`, `/Pextra`, and a `/P` namespace are rejected.
- lookup and listing accept the Python `/P` form; lookup also accepts the earlier raw Rust form.
- upsert writes the Python bond shape, canonicalizes a raw peer to `/P`, and preserves unknown peer
  extensions.
- bond decoding accepts both the Python shape without `address_type` and the earlier Rust
  `address_type: 0` shape.

The focused green commands were:

```powershell
cargo test --test profile_compat --all-features --locked
cargo test profile_key_store --all-features --locked
cargo test public_peer_names_accept_only_the_bumble_public_suffix --all-features --locked
cargo test --lib --all-features --locked
```

The last command reported 273 passed and 1 ignored. The ignored test is the separately invoked
manual cross-language writer gate, not a product regression.

## Backend rollback rehearsal

The tested order was Rust reconnect, Rust process exit, adapter reopen, then pinned Python reconnect.
No production profile was passed to either backend.

| step | machine result | operator observation |
|---|---|---|
| copied-profile verification after the compatibility fixes | `profile_verified`, controller kind `pro` | not applicable |
| Rust Direct reconnect with A tap | `connection_completed`; `neutral_close:true`; shutdown 13.9466 ms | connection and A reflection confirmed; no residual input after exit |
| Rust exit followed by adapter sentinel | `adapter_opened` | not applicable |
| `swbt-python` 0.6.0 at `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`, `examples/tap_a.py` | process exited 0 and printed `!!! no pending response future to set` | connection and A reflection confirmed; no residual input after exit |
| post-run profile comparison | working copy and untouched original both byte-identical to baseline | not applicable |
| Python exit followed by final Rust adapter sentinel | `adapter_opened` | not applicable |

The operator subsequently confirmed the connection, A reflection, and absence of residual input for
both successful runs. The Python warning is retained as machine output and is not treated as evidence
against the observed neutral end state. The final adapter-open sentinel after Python process exit
proved that the process released its exclusive USB ownership. The rollback rehearsal is complete.
