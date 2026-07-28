# `swbt-python` からの移行戦略

- 状態: **決定**
- Python 基準断面: `swbt-python@84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- 型関係: [type-modeling.md](type-modeling.md)
- Rust API: [api.md](api.md)
- profile / dependency 前提: [source-baseline.md](source-baseline.md)

この文書は、Python 利用者、profile、テスト資産、実機運用を Rust 版へ段階的に移す方法を定義する。Rust 版は Python のクラス構造を再現せず、controller model と reporting mode を型で表し、モデル固有入力の誤用をコンパイル時に拒否する。

## 1. 移行目標

移行成功条件:

- 同じ controller model と入力から互換な NX HID bytes を生成する
- 同じ output report に対し互換な reply と session transition を行う
- Periodic / Direct の state commit 条件が一致する
- Python profile schema v2 を相互に読める
- stored link key の意味を失わない
- connection readiness と close neutral の契約が一致する
- 実機で pairing / reconnect / input / cleanup が再現する
- Python で実行時に拒否していたモデル不一致入力を、Rust では可能な限りコンパイル時に拒否する

互換対象外:

- `asyncio` coroutine / task identity
- Python class inheritance
- exception message の完全一致
- internal import path
- dataclass の実装詳細
- pytest fixture API
- Bumble Python object 型
- callback scheduling order。ただし wire send order は互換対象
- `Button` と `InputState` を model 非依存の単一型として維持すること

## 2. 互換レベル

| level | 内容 | gate |
|---|---|---|
| L0 Source observation | Python断面とfixture provenance固定 | M1 |
| L1 Type/API model | model/reporting/input能力のcompile-pass/fail | M0-M1 |
| L2 Pure protocol | report/parser/reply/SPI/conversion一致 | M1 |
| L3 Runtime semantics | Periodic/Direct/readiness/cleanup一致 | M2 |
| L4 Virtual Bluetooth | Classic/SDP/HID/pairing通過 | M4 |
| L5 Profile data | schema v2とkey fields相互読書き | M6 |
| L6 Hardware behavior | target matrixでpairing/input/reconnect | M5-M8 |
| L7 Operational cutover | docs/probe/monitoring/backend rollback | M9 |

manual pairing 1回だけで移行完了にしない。

## 3. controller type mapping

| Python v0.6.0 | Rust |
|---|---|
| `ProController` | `Controller<model::Pro, reporting::Periodic>` / `ProController` |
| `DirectProController` | `Controller<model::Pro, reporting::Direct>` / `DirectProController` |
| `JoyConL` | `Controller<model::JoyConL, reporting::Periodic>` / `JoyConL` |
| `DirectJoyConL` | `Controller<model::JoyConL, reporting::Direct>` / `DirectJoyConL` |
| `JoyConR` | `Controller<model::JoyConR, reporting::Periodic>` / `JoyConR` |
| `DirectJoyConR` | `Controller<model::JoyConR, reporting::Direct>` / `DirectJoyConR` |
| `SwitchGamepad` ABC | 初期Rust APIでは対応traitなし。generic `Controller<M, R>`を使う |
| `PeriodicSwitchGamepad` ABC | `R = reporting::Periodic` |
| `DirectSwitchGamepad` ABC | `R = reporting::Direct` |

Rust の 6 名は独立 newtype ではなく alias である。

## 4. construction mapping

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

- Rust builder は `ControllerBuilder<model::Pro, reporting::Periodic>`
- adapter open は `open()` まで行わない
- existing profile は `PairingProfile<model::Pro>`として検証する
- `report_period()` は Direct builder に存在しない
- `ControllerKind` や reporting mode を値として指定しない
- Rust controller は `Clone` しない

## 5. model-specific button mapping

Python は全モデルで共通 `Button` enum を使い、unsupported input を実行時に拒否する。

```python
await pro.tap(Button.A)
await right.tap(Button.A)
```

Rust は同じ論理名でも model 付き型を使う。

```rust
pro.tap([ProButton::A], Duration::from_millis(80))?;
right.tap([JoyConRButton::A], Duration::from_millis(80))?;
```

`ProButton::A` と `JoyConRButton::A` は内部で `ButtonKind::A` に対応するが、相互代入できない。Joy-Con L には `JoyConLButton::A` が存在しない。

### 5.1 mapping table

| Python `Button` | Pro | Joy-Con L | Joy-Con R |
|---|---|---|---|
| A/B/X/Y | `ProButton::*` | 型として非公開 | `JoyConRButton::*` |
| L/ZL | `ProButton::*` | `JoyConLButton::*` | 型として非公開 |
| R/ZR | `ProButton::*` | 型として非公開 | `JoyConRButton::*` |
| Plus/Home | `ProButton::*` | 型として非公開 | `JoyConRButton::*` |
| Minus/Capture | `ProButton::*` | `JoyConLButton::*` | 型として非公開 |
| D-pad | `ProButton::*` | `JoyConLButton::*` | 型として非公開 |
| LeftStick click | `ProButton::LEFT_STICK` | `JoyConLButton::LEFT_STICK` | 型として非公開 |
| RightStick click | `ProButton::RIGHT_STICK` | 型として非公開 | `JoyConRButton::RIGHT_STICK` |
| SL/SR | 型として非公開 | `JoyConLButton::SL/SR` | `JoyConRButton::SL/SR` |

既存 application が button 名を設定ファイルから読む場合は、まず `ButtonKind` を parseし、選択済み model の `Button<M>::try_from()` を呼ぶ。unsupported input はこの動的境界で `UnsupportedInput` になる。

## 6. input state mapping

Python:

```python
state = InputState.neutral().with_buttons([Button.A])
await pad.apply(state)
```

Rust Pro:

```rust
let state = ProInputState::neutral()
    .with_buttons([ProButton::A]);
pad.apply(state)?;
```

Rust Joy-Con R:

```rust
let state = JoyConRInputState::neutral()
    .with_buttons([JoyConRButton::A]);
right.apply(state)?;
```

`InputState<model::Pro>` を Joy-Con controller へ渡せない。application 内で model を抽象化していた箇所は、model generic function にするか、動的入口で `ControllerKind` を分岐する。

## 7. common value mapping

次はPython版と同じ概念をmodel非依存値として移す。

| Python | Rust | 備考 |
|---|---|---|
| `Stick` | `Stick` | 座標値は共通 |
| `IMUFrame` | `ImuFrame` | 六軸センサー値は共通 |
| 1 frame / 3 frames | `ImuSamples::Repeat` / `Frames` | invalid slice lengthをAPIから除去 |
| `ControllerColors` | `ControllerColors` | model specがwire利用を決定 |
| float seconds | `Duration` | 単位を型で明示 |

### 7.1 stick API

Pythonでは共通methodを呼び、unsupported sideを実行時に拒否する。

Rustではmethod自体を能力で制限する。

```rust
pro.left_stick(stick)?;
pro.right_stick(stick)?;
pro.sticks(left, right)?;

left.left_stick(stick)?;
right.right_stick(stick)?;
```

Joy-Con Lの`right_stick()`、Joy-Con Rの`left_stick()`、片側Joy-Conの`sticks()`はコンパイル不能。

### 7.2 IMU API

```rust
pad.imu(ImuFrame::neutral())?;
pad.imu([
    frame0,
    frame1,
    frame2,
])?;
```

`ImuFrame`をmodelごとに分けない。wire packing差はprotocol encoderが`M::SPEC`から選ぶ。

## 8. reporting semantics mapping

| Python | Rust |
|---|---|
| Periodic `apply(state)` | `Controller<M, Periodic>::apply(InputState<M>)` |
| Direct `send(state)` | `Controller<M, Direct>::send(InputState<M>)` |
| Periodicに`send`なし | methodが存在しない |
| Directに`apply`なし | methodが存在しない |
| Periodic `report_period_us` | Periodic builderの`report_period(Duration)` |
| Directにperiodなし | builder methodが存在しない |

Periodic:

- local state commit後に成功
- 未接続中もstate更新可能
- next tickでlatest state送信
- send failureでrollbackしない

Direct:

- 接続済み必須
- transport acceptance後だけcommit
- acceptance前failureでprevious state維持
- user-input schedulerなし

## 9. resource scope mapping

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

Rust `Drop` にPython context managerと同じcleanup保証を持たせない。error pathでも明示`close()`結果を処理する。

## 10. connection mapping

| Python | Rust |
|---|---|
| `await open()` | `open()` |
| `await pair(timeout=...)` | `pair(Duration)` |
| `await reconnect(timeout=...)` | `reconnect(Duration)` |
| `await connect(...)` | `connect(ConnectOptions)` |
| `await try_reconnect(...)` | `try_reconnect(Duration)` |
| `await try_connect(...)` | `try_connect(ConnectOptions)` |
| `await close(neutral=True)` | `close()` |
| `await close(neutral=False)` | `close_without_neutral()` |

正常接続はlink/HID channel openだけでなく、report mode、player lights、reply acceptance、handshake回収、Periodic holdoffを含むreadinessまで待つ。

## 11. profile schema v2

JSON envelopeはPython基準断面と互換にする。

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

raw parserは`ControllerKind`を読む。typed controllerは次に変換する。

```text
ProfileDocument { controller_kind: ControllerKind }
  ↓ compare with M::KIND
PairingProfile<M>
```

`PairingProfile<model::Pro>`をJoy-Con controllerへ渡すAPIは作らない。

### 11.1 key store

保持field:

- `address_type`
- `ltk`
- `ltk_central`
- `ltk_peripheral`
- `irk`
- `csrk`
- `local_csrk`
- `link_key`
- `link_key_type`

unknown fieldを黙って捨てない。key materialをlogしない。

### 11.2 write

- UTF-8
- two-space indent
- deterministic key order
- trailing newline
- same-directory temporary file
- flush / `sync_all`
- create-newはno-replace
- updateはatomic replace
- concurrent writerはlockで拒否

自動backup、世代管理、復元機能は実装しない。更新中断では更新前または更新後のvalid fileが残ることを保証する。

## 12. dynamic application migration

controller modelが設定ファイルやCLIで決まるapplicationは、入口で一度だけ分岐する。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後は`Button<M>`と`InputState<M>`を維持する。core application全体を`ControllerKind + ButtonKind + untyped state`で動かさない。

異なるmodelを同じcollectionへ格納する必要がある場合も、初期libraryに`AnyController`を追加せず、application側のenumで必要操作とfailure semanticsを明示する。

## 13. diagnostics mapping

Python `DiagnosticsConfig` writerはRustでは`tracing` subscriberへ移す。

`GamepadStatus`は次を含む。

- lifecycle
- connected
- `controller_kind` (`M::KIND`から導出)
- `reporting_kind` (`R::KIND`から導出)
- report mode
- accepted counters
- last subcommand
- disconnect reason
- worker failure

model markerや`Button<M>`の型名だけに依存せず、運用ログにはruntime projectionを記録する。

## 14. error mapping

| Python | Rust `ErrorKind` |
|---|---|
| `AdapterDiscoveryError` | `AdapterDiscovery` |
| `TransportOpenError` | `TransportOpen` |
| `ClosedError` | `TransportClosed` |
| `ConnectionTimeoutError` | `ConnectionTimeout` |
| `ConnectionFailedError` | `ConnectionFailed` |
| `InvalidInputError` | `InvalidInput` |
| `UnsupportedInputError` | `UnsupportedInput` |
| `InvalidProfileError` | `InvalidProfile` |
| `ProfileControllerMismatchError` | `ProfileControllerMismatch` |
| `InvalidKeyStoreError` | `InvalidKeyStore` |
| `AdapterIdentityRecoveryRequired` | `UnsupportedCapability`またはidentity-specific error |

静的Rust APIでは多くのunsupported inputがコンパイル不能になる。`UnsupportedInput`はCLI、config、trace replay、`ButtonKind -> Button<M>`変換などの動的境界に残る。

applicationはmessage文字列をparseせず`ErrorKind`をmatchする。

## 15. 段階移行

### Phase A: observation

Pythonを実機基準とし、source SHA、protocol fixture、profile fixture、hardware trace、supported input matrixを固定する。

### Phase B: type/API shadow

RustのUI testで次を固定する。

- model-specific button集合
- stick capability
- model-specific state
- Periodic / Direct method集合
- common `ImuFrame`

### Phase C: pure protocol shadow

同じsemantic inputをPythonとRustへ流し、bytesとeffectsを比較する。Pythonの共通`Button`をfixture generatorでmodel付きlogical inputへ変換する。

### Phase D: virtual Bluetooth

RustがBumble virtual linkを所有し、Classic、SDP、HID、pairing、typed inputを通す。

### Phase E: hardware canary

専用adapterと専用profileでRust fresh pairingを行う。既存Python processと同時にadapterを開かない。

criteria:

- clean pairing
- model-supported input reflection
- neutral
- close
- repeated run
- no adapter identity mutation
- trace redaction

### Phase F: profile interoperability

synthetic profile、専用hardware profile、既存profile copyの順にRust read/write/reconnectを試す。自動backup機能は使わない。

### Phase G: workload cutover

applicationのcontroller操作をPythonからRustへ置換する。

- failure rate
- connect latency
- report jitter
- close latency
- reconnect success
- profile update count
- compile-time model guarantees

backend config switchを維持する。

### Phase H: Python retirement

controller model / reporting modeごとに判定する。

- Rust supported release
- target hardware matrix
- type/API gate
- profile compatibility
- operational docs
- equivalent diagnostics
- unresolved S1なし
- backend rollback rehearsal

未移植featureを使うapplicationはPythonを残す。

## 16. application boundary

言語境界が必要な場合の優先順:

1. application自体をRustへ移す
2. Rust CLIをsubprocessとして使う
3. narrow IPC daemonを別仕様で追加
4. FFI

PyO3 bindingを最初に作らない。Bluetooth ownership、shutdown、callback thread、wheel packagingが追加問題になる。

subprocess protocolを作る場合も、raw HID / HCI bytesを公開しない。controller kindとbutton kindは動的値になるため、CLI内部でtyped pathへ変換する。

## 17. configuration単位

```toml
controller = "pro"
reporting = "periodic"
connect_timeout_ms = 30000
report_period_us = 8000
tap_duration_ms = 80
```

Rust側はcontroller/reporting文字列を入口でmarker typeへ分岐する。同じfieldにsecondsとmillisecondsを混在させない。

## 18. backend rollback

```toml
controller_backend = "python" # or "rust"
profile_path = "profiles/pro.json"
```

同じprocessまたは複数processで両backendがadapterを同時openしない。backend switchはprocess restartを伴う。

checklist:

- Rust controller close
- worker exit
- adapter re-enumeration
- current profile hash / parse result
- Python environment lock
- Python profile load
- Python reconnect、または必要時fresh pairing
- input neutral
- incident trace保存

libraryはprofileの自動複製・復元を行わない。

## 19. 移行完了判定

model / reporting組み合わせごとに判定する。

完了:

- L0-L6
- UI type tests
- target hardware evidence
- profile data safety
- error / diagnostics mapping
- workload soak
- backend rollback rehearsal
- application docs
- maintainer sign-off

未完了:

- compile successだけ
- unit testだけ
- single manual pairing
- report accepted traceだけ
- Python profileをparseしただけ
- Bumble READMEのcapability記述だけ

## 20. Python基準断面の将来更新

current Python mainを自動的にnew source of truthにしない。

1. v0.6.0 baselineに対するmilestone完了
2. new release diff分類
3. bug fix / protocol discovery / API addition分離
4. fixture version追加
5. model能力表への影響確認
6. Rust APIへ取り込むか明示決定
7. old fixtureを削除しない

新しいPython機能がmodel固有能力を増やす場合、共通`Button`へ追加するだけで終えず、model宣言、`Button<M>`、UI test、wire mapping、migration表を同じ変更で更新する。