# `swbt-python` からの移行戦略

- 状態: **決定**
- Python 基準断面: `swbt-python@84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- Rust API: [api.md](api.md)
- profile / dependency 前提: [source-baseline.md](source-baseline.md)

この文書は、Python 利用者・profile・テスト資産・実機運用を Rust 版へ段階的に移す方法を定義する。最初から Rust 版へ全面切替せず、同じ入力と同じ profile に対する観測結果を比較してから ownership を移す。

## 1. 移行目標

移行成功は「Rust で同名 method が存在すること」ではなく、次が成立することとする。

- 同じ controller kind と入力状態から、互換な NX HID bytes を生成する
- 同じ output report に対し、互換な reply と session transition を行う
- Periodic / Direct の state commit 条件が一致する
- pairing profile schema v2 を相互に読める
- stored link key の意味を失わない
- connection readiness と close neutral の契約が一致する
- 実機で pairing / reconnect / input / cleanup が再現する
- Rust 固有の error と ownership が利用者に明確である

Python の次の要素は互換対象外である。

- `asyncio` coroutine / task identity
- Python class inheritance
- exception message の完全一致
- import path の内部構造
- mutable object / dataclass の細部
- pytest marker / fixture API
- Bumble Python object の型
- callback scheduling order。ただし wire send order は互換対象

## 2. 互換レベル

| level | 内容 | release gate |
|---|---|---|
| L0 Source observation | Python 断面と fixture provenance を固定 | M1 |
| L1 Pure protocol | report / parser / reply / SPI / conversion が一致 | M1 |
| L2 Runtime semantics | Periodic / Direct / readiness / cleanup が fake 上で一致 | M2 |
| L3 Virtual Bluetooth | Classic / SDP / HID / pairing が Bumble virtual 上で通る | M4 |
| L4 Profile data | schema v2 と key fields を相互読書き | M6 |
| L5 Hardware behavior | target matrix で pairing / input / reconnect | M5-M8 |
| L6 Operational cutover | rollback、docs、probe、monitoring | M9 |

L1 を通らず L5 の manual success だけで移行完了にしない。

## 3. API mapping

### 3.1 controller type

| Python v0.6.0 | Rust |
|---|---|
| `ProController` | `swbt::ProController` |
| `JoyConL` | `swbt::JoyConL` |
| `JoyConR` | `swbt::JoyConR` |
| `DirectProController` | `swbt::DirectProController` |
| `DirectJoyConL` | `swbt::DirectJoyConL` |
| `DirectJoyConR` | `swbt::DirectJoyConR` |
| `SwitchGamepad` ABC | sealed `swbt::SwitchGamepad` trait |
| `PeriodicSwitchGamepad` ABC | sealed `swbt::PeriodicSwitchGamepad` trait |
| `DirectSwitchGamepad` ABC | sealed `swbt::DirectSwitchGamepad` trait |

Rust trait は第三者実装用 extension point ではない。

### 3.2 construction

Python:

```python
pad = ProController(
    adapter="usb:0",
    profile_path="profiles/pro.json",
    report_period_us=8000,
)
```

Rust:

```rust
let mut pad = ProController::builder("usb:0")
    .profile_path("profiles/pro.json")
    .report_period(Duration::from_micros(8_000))
    .build()?;
```

差:

- Rust builder は設定検査と profile load を `Result` で返す
- adapter open は `open()` まで行わない
- `report_period_us: int` は `Duration`
- transport injection はどちらも public API に出さない
- Rust object は `Clone` しない

### 3.3 resource scope

Python:

```python
async with ProController(...) as pad:
    await pad.connect(timeout=30.0)
```

Rust:

```rust
let mut pad = ProController::builder("usb:0")
    .profile_path("profiles/pro.json")
    .build()?;

pad.open()?;
let operation = pad.connect(ConnectOptions {
    timeout: Duration::from_secs(30),
    allow_pairing: false,
});
let close = pad.close();

operation?;
close?;
```

Rust の `Drop` に Python async context manager と同じ保証を持たせない。error path でも `close()` 結果を失わないよう、application helper を用意してよい。

候補:

```rust
fn with_controller<T>(
    pad: &mut impl SwitchGamepad,
    f: impl FnOnce(&mut dyn SwitchGamepad) -> swbt::Result<T>,
) -> swbt::Result<T>;
```

この helper を public API にするかは利用例を見て判断する。初期実装は明示 close を docs で示す。

### 3.4 connection

| Python | Rust |
|---|---|
| `await open()` | `open()` |
| `await pair(timeout=...)` | `pair(Duration)` |
| `await reconnect(timeout=...)` | `reconnect(Duration)` |
| `await connect(timeout=..., allow_pairing=...)` | `connect(ConnectOptions)` |
| `await try_reconnect(...)` | `try_reconnect(Duration)` |
| `await try_connect(...)` | `try_connect(ConnectOptions)` |
| `await close(neutral=True)` | `close()` |
| `await close(neutral=False)` | `close_without_neutral()` |

timeout の float seconds を機械的に `as_millis` へ丸めない。application configuration の単位を明示し、`Duration::from_secs_f64` を使う場合は負値 / NaN を先に拒否する。

### 3.5 input

| Python | Rust |
|---|---|
| `await pad.apply(state)` | `pad.apply(state)` |
| `await pad.send(state)` | `pad.send(state)` |
| `await pad.press(Button.A)` | `pad.press(&[Button::A])` |
| `await pad.release(Button.A)` | `pad.release(&[Button::A])` |
| `await pad.tap(Button.A, duration=0.08)` | `pad.tap(&[Button::A], Duration::from_millis(80))` |
| `await pad.lstick(stick)` | `pad.left_stick(stick)` |
| `await pad.rstick(stick)` | `pad.right_stick(stick)` |
| `await pad.sticks(left=..., right=...)` | `pad.sticks(left, right)` |
| `await pad.imu(frame)` | `pad.imu(&[frame])` |
| `await pad.neutral()` | `pad.neutral()` |
| `pad.snapshot()` | `pad.snapshot()` |
| `pad.status()` | `pad.status()` |

Rust API は variadic positional argument を持てないため、button は slice、IMU は slice を使う。

### 3.6 value type

| Python | Rust |
|---|---|
| `Button.PLUS` | `Button::Plus` |
| `Button.DPAD_UP` | `Button::DpadUp` |
| `IMUFrame` | `ImuFrame` |
| `Stick.raw(x=..., y=...)` | `Stick::raw(x, y)?` |
| `Stick.normalized(x=..., y=...)` | `Stick::normalized(x, y)?` |
| immutable dataclass | private fields + constructor / consuming builder |
| Python `int` validation | bounded integer type + `Result` |
| tuple 3-axis | `[i16; 3]` / `[f32; 3]` |

### 3.7 diagnostics

Python `DiagnosticsConfig(trace_writer=...)` は Rust では `tracing` subscriber setup へ移す。

Python:

```python
with trace_path.open("w") as trace:
    pad = ProController(..., diagnostics=DiagnosticsConfig(trace_writer=trace))
```

Rust candidate:

```rust
let file = std::fs::File::create("trace.jsonl")?;
let subscriber = tracing_subscriber::fmt()
    .json()
    .with_writer(file)
    .finish();

tracing::subscriber::with_default(subscriber, || run_controller())?;
```

library は subscriber の具体型を所有しない。`GamepadStatus` は引き続き public snapshot とする。

## 4. profile schema v2

### 4.1 exact envelope

Python 基準断面の normalized shape:

```json
{
  "controller_kind": "pro",
  "format": "swbt.profile",
  "identity": {
    "kind": "adapter-default"
  },
  "key_store": {
    "namespaces": {}
  },
  "schema_version": 2
}
```

明示 address:

```json
{
  "controller_kind": "joycon_l",
  "format": "swbt.profile",
  "identity": {
    "address": "02:11:22:33:44:55",
    "kind": "exp-local-address"
  },
  "key_store": {
    "namespaces": {
      "02:11:22:33:44:55": {}
    }
  },
  "schema_version": 2
}
```

controller kind values:

- `pro`
- `joycon_l`
- `joycon_r`

identity kind values:

- `adapter-default`
- `exp-local-address`

Rust は schema v1 を推測移行しない。unsupported schema は明示 error とし、新 profile 作成を案内する。

### 4.2 key store shape

`key_store.namespaces`:

```text
local-controller-address
  → peer-address
      → PairingKeys object
```

current local namespace の peer は最大 1 件とする。

PairingKeys object で保持する field:

- `address_type`
- `ltk`
- `ltk_central`
- `ltk_peripheral`
- `irk`
- `csrk`
- `local_csrk`
- `link_key`
- `link_key_type`

各 key object:

- `value`: hex string
- `authenticated`: bool
- optional `ediv`
- optional `rand`: hex string
- optional `sign_counter`

Rust Bumble の current stored type と Python Bumble `PairingKeys.to_dict()` の field compatibility は fixture で確認する。確認前に unknown field を serde の既定動作で捨てない。

### 4.3 parser policy

初期 parser は次を行う。

- required top-level field の type / value validation
- identity consistency
- controller kind validation
- namespace map shape validation
- key object validation
- address normalization
- multiple current peer rejection
- secret-safe error

unknown top-level field の policy:

**決定:** 読み取り時に preserve できる raw extension map を保持する。write 時に既知 field と衝突しない unknown field を残す。これにより future Python minor version が metadata を追加しても Rust が黙って削除しない。

ただし unknown `format`、`schema_version`、`identity.kind`、`controller_kind` は拒否する。

### 4.4 normalized write

- UTF-8
- two-space indent
- key sort
- trailing newline
- uppercase colon local address
- hex representation は基準 fixture と一致
- key material object の field omission policy を一致
- `authenticated = false` の serialize policy を fixture で固定

JSON whitespace の完全一致より semantic round-trip を主契約とする。ただし normalized writer test は deterministic output を要求する。

## 5. profile 安全移行

### 5.1 read-only first

Rust profile support の導入順:

1. Python fixture を Rust が read
2. Rust domain を diagnostics へ secret-free 表示
3. Rust は profile を変更せず virtual reconnect に使う
4. synthetic copy へ write
5. Rust output を Python が read
6. backup を伴う real profile update
7. target hardware reconnect

L4 完了前の Rust build は existing Python profile を read-only で開く feature gate を使ってよい。pairing key update が必要な operation は拒否する。

### 5.2 backup

最初に Rust が既存 Python profile を更新する前に、同じ directory に backup を作る。

候補:

```text
switch-pro.json
switch-pro.json.swbt-python-v0.6.0.bak
```

backup は create-new で作り、既存 backup を上書きしない。backup hash と source profile hash を migration record に残す。key material を log せず hash だけを記録する。

安定後、毎回 backup を作るか初回だけにするかは運用 data で決める。`0.1.0` までは初回 backup を既定にする。

### 5.3 atomic update

- same-directory temp
- flush + sync
- atomic replace
- parent sync where supported
- lock
- cleanup
- interruption test

Windows と Unix の replace semantics を別 test で確認する。

### 5.4 concurrent access

Python process と Rust process が同じ profile を同時使用しない。Rust は lock file を使い、lock metadata に PID / process start / hostname を含める。stale lock の自動削除は、process existence と age を安全に判断できる場合だけ行う。

Python v0.6.0 は同じ Rust lock protocol を知らないため、dual-run 期間は operator が exclusive ownership を切り替える。profile の copy を分けて A/B test し、同じ file に同時書きしない。

### 5.5 rollback

Rust run 後に reconnect / profile parse が失敗した場合:

1. controller process を停止
2. current Rust-written profile を別名保存
3. backup hash を確認
4. backup を atomic restore
5. Python v0.6.0 で profile load
6. adapter identity を確認
7. reconnect
8. Rust-written file と trace を secret-safe location で調査

bond が console / adapter 側で変わった場合、file rollback だけで reconnect が戻るとは限らない。fresh pairing を最後の復旧手段として明記する。

## 6. dual implementation strategy

### 6.1 Phase A: observation

Python を production / hardware基準とし、Rust は fixture consumer。

成果:

- exact source SHA
- protocol fixture
- profile fixture
- hardware trace vocabulary
- expected subcommand sequence
- supported input matrix

### 6.2 Phase B: shadow protocol

同じ input / output sequence を Python と Rust pure core に流し、bytes / effects を比較する。Bluetooth は Python だけが所有する。

比較:

- input report
- output parse
- reply
- readiness
- IMU
- SPI

### 6.3 Phase C: virtual Bluetooth

Rust が Bumble virtual link を所有し、Python fixture peer とやり取りする。実 adapter はまだ Python 運用に残す。

### 6.4 Phase D: hardware canary

専用 adapter と専用 profile copy で Rust fresh pairing を行う。既存 Python profile を使わない。

canary criteria:

- clean pairing
- input reflection
- neutral
- close
- repeated run
- no adapter identity mutation
- trace redaction

### 6.5 Phase E: profile interoperability

synthetic profile、専用 hardware profile、既存 profile copy の順に Rust read / write / reconnect を試す。

profile ownership window を記録する。

### 6.6 Phase F: workload cutover

application の controller operation を Python から Rust へ置換する。

- operation log を同じ semantic vocabulary で比較
- failure rate
- connect latency
- report jitter
- close latency
- reconnect success
- profile update count

rollback command / config switch を維持する。

### 6.7 Phase G: Python retirement

次を満たした機能だけ Python dependency を外す。

- Rust supported release
- target hardware matrix
- profile backup
- operational docs
- equivalent diagnostics
- no unresolved S1
- rollback rehearsal

未移植 feature を使う application は Python を残す。repository の存在を消すことを migration completion としない。

## 7. application 移行パターン

### 7.1 command boundary

既存 Python application と Rust library の言語境界が必要な場合、初期選択肢は次の順。

1. application 自体を Rust へ移す
2. Rust CLI を subprocess として使う
3. narrow IPC daemon を別仕様で追加
4. FFI

PyO3 binding を最初に作らない。Python から Rust を呼ぶこと自体より、Bluetooth resource ownership、shutdown、callback thread、wheel packaging が追加問題になる。

### 7.2 subprocess bridge

移行期間の subprocess bridge を作る場合も、旧 daemon IPC の完全互換を目標にしない。

最低 protocol:

- version handshake
- controller config
- connect / input / close command
- typed result
- status event
- graceful shutdown
- command id / idempotency
- profile exclusive lock

raw HCI / HID bytes を IPC で公開しない。

### 7.3 configuration

Python float seconds / integer microseconds から Rust `Duration` へ移す際、設定 file の単位を field 名に含める。

例:

```toml
connect_timeout_ms = 30000
report_period_us = 8000
tap_duration_ms = 80
```

同じ field に seconds と milliseconds を混在させない。

## 8. error migration

mapping:

| Python exception / result | Rust |
|---|---|
| `AdapterDiscoveryError` | `ErrorKind::AdapterDiscovery` |
| `TransportOpenError` | `ErrorKind::TransportOpen` |
| `ClosedError` | `TransportClosed` |
| `ConnectionTimeoutError` | `ConnectionTimeout` |
| `ConnectionFailedError` | `ConnectionFailed` |
| `InvalidInputError` | `InvalidInput` |
| `UnsupportedInputError` | `UnsupportedInput` |
| `InvalidProfileError` | `InvalidProfile` |
| `ProfileControllerMismatchError` | `ProfileControllerMismatch` |
| `InvalidKeyStoreError` | `InvalidKeyStore` |
| `AdapterIdentityRecoveryRequired` | `UnsupportedCapability` または identity-specific recovery error |
| `ConnectionResult.status` | `ConnectionStatus` |

application は error message string を parse せず `ErrorKind` を match する。

Rust error source chain に Bumble error を保持するが、upstream enum variant を application logic に使わせない。

## 9. semantic parity checklist

### 9.1 Periodic

- pre-connect state update
- latest state on next tick
- default 8 ms
- no burst catch-up
- reply holdoff
- readiness start point
- tap sends press and release
- close neutral
- disconnect neutral reset

### 9.2 Direct

- no periodic user input
- connected required
- one accepted report per successful helper
- acceptance-before-commit
- send failure rollback
- tap transaction
- close neutral exception
- same shape profile reuse

### 9.3 connection

- profile mismatch before adapter open
- reconnect first when bond exists
- no bond + pairing flag
- no implicit bond delete
- link != ready
- report mode + lights
- same session
- timeout cleanup
- close idempotence

### 9.4 protocol

- report lengths / ids
- timer semantics
- button / stick mapping
- SPI
- device info
- subcommand ACK
- IMU mode
- reply prefix
- send order
- unsupported command

### 9.5 Joy-Con

- shape identity
- side-specific input reject
- SL / SR
- left / right sticks
- colors
- Periodic / Direct
- profile mismatch

## 10. deliberate differences

### 10.1 synchronous public API

理由:

- Bumble Rust core が同期
- public executor dependency を避ける
- single owner worker で send order を固定
- Rust application は必要なら own async runtime の blocking task へ移せる

async application 例:

```rust
let result = tokio::task::spawn_blocking(move || run_controller(config)).await??;
```

controller reference を async task 間で共有する設計は推奨しない。command ownership を一 task に固定する。

### 10.2 `Drop` guarantee

Python async context manager は awaitable cleanup を持つ。Rust `Drop` は error を返せず、無期限 block も不適切である。したがって explicit close を authoritative とする。

### 10.3 tracing

Python library-owned diagnostics writer をそのまま移植せず、Rust ecosystem の subscriber model を使う。event vocabulary と redaction は互換対象にする。

### 10.4 concrete newtype

Python inheritance を generic alias で模倣せず、concrete newtype + sealed trait を使う。public method capability は型で分ける。

## 11. feature gap management

各 gap を次の表で管理する。

| feature | Python v0.6.0 | Rust initial | cutover rule |
|---|---|---|---|
| Pro Periodic | 対応 | M5 | hardware gate 後 |
| Pro Direct | 対応 | M6 | transaction + hardware |
| Joy-Con L/R | 対応 | M7 | side別 evidence |
| profile v2 | 対応 | M6 | round-trip + backup |
| reconnect | 対応 | M6 | power-cycle evidence |
| adapter-default | 対応 | M5 | first production path |
| explicit local address | 限定対応 | 後続 gate | recovery verified 後 |
| IMU | 対応 | M8 | fixture + hardware trace |
| diagnostics | 対応 | M8 | event / redaction |
| adapter discovery | 対応 | M3 | no-open test |
| raw HID | 非公開 | 非公開 | migration不要 |
| NFC / IR / rumble high-level | 非公開 | 非公開 | 別仕様 |
| JoyConPair | 非公開 | 非公開 | 別仕様 |

Rust が未対応の行は application cutover 対象から除外する。stub success を返さない。

## 12. performance comparison

同じ environment で測定する。

- fresh pairing latency
- reconnect latency
- readiness latency
- 8 ms report jitter
- CPU idle / active
- memory
- close latency
- repeated run failure rate
- binary / environment startup
- profile write latency

Python と Rust の数値差だけで pass/fail にしない。stale input や ordering regression は平均性能より優先して修正する。

## 13. operational rollback

application config candidate:

```toml
controller_backend = "python" # or "rust"
profile_path = "profiles/pro.json"
```

同じ process で両 backend が adapter を同時 open しない。backend switch は process restart を伴う。

rollback checklist:

- Rust controller close
- worker process exit
- adapter re-enumeration
- profile hash
- backup availability
- Python environment lock
- Python reconnect
- input neutral
- incident trace 保存

## 14. migration completion criteria

controller kind / reporting mode ごとに別判定する。

完了:

- L0-L5
- target hardware evidence
- profile data safety
- error / diagnostics mapping
- workload soak
- rollback rehearsal
- application docs
- maintainer sign-off

未完了:

- unit test だけ
- single manual pairing
- report accepted trace だけ
- Python profile を Rust が parse しただけ
- compile success
- Bumble README の capability 記述だけ

## 15. Python 基準断面の将来更新

Rust 実装中に `swbt-python` が進んだ場合、current main を自動的に new source of truth にしない。

1. v0.6.0 baseline に対する Rust milestone を完了
2. Python の new release / commit diff を分類
3. bug fix / protocol discovery / API addition を分ける
4. compatibility fixture version を追加
5. Rust API へ取り込むか明示決定
6. old fixture を削除しない

security / adapter damage prevention fix は例外として優先 backport する。

## 16. 移行記録

hardware / profile cutover ごとに、秘密情報を除いて次を記録する。

```text
date
application
controller kind
reporting mode
old backend version
new swbt-rs commit
Bumble revision
OS / adapter / driver / console firmware
profile source hash
backup hash
fresh pairing or reconnect
test result
rollback result
known limitations
```

記録は `dev-journal/` または external operation log に置き、link key、full profile、raw sensitive packet を含めない。
