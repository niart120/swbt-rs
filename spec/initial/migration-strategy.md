# `swbt-python` からの移行戦略

- 状態: **決定**
- Python 基準断面: `swbt-python@84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- 型関係: [type-modeling.md](type-modeling.md)
- Rust API: [api.md](api.md)
- profile / dependency 前提: [source-baseline.md](source-baseline.md)

この文書は、Python利用者、profile、テスト資産、実機運用をRust版へ段階的に移す方法を定義する。Rust版はPythonのクラス構造を再現せず、controller modelとreporting modeを型で表し、model固有入力の誤用を通常の静的APIから排除する。

## 1. 移行目標

- 同じcontroller modelと入力から互換なNX HID bytesを生成する
- 同じoutput reportに対し互換なreplyとsession transitionを行う
- Periodic / Directのstate commit条件を一致させる
- Python 0.6.0のschema v2 Classic pairing profileを相互に読める
- stored link keyの意味を失わない
- connection readinessとclose neutralの契約を一致させる
- 実機でpairing / reconnect / input / cleanupを再現する
- Pythonで実行時に拒否していたmodel mismatchをRustでは可能な限り型で表す

互換対象外:

- `asyncio` coroutine / task identity
- Python class inheritance
- exception messageの完全一致
- internal import path
- dataclassの実装詳細
- pytest fixture API
- Bumble Python object型
- callback scheduling order。ただしwire send orderは互換対象
- `Button`と`InputState`をmodel非依存の単一型として維持すること

## 2. 互換レベル

| level | 内容 | gate |
|---|---|---|
| L0 Source observation | Python断面とfixture provenance | M1 |
| L1 Type/API model | `Controller<M,R>`、model固有input、共通値型の設計確定 | M0 |
| L2 Model data | model宣言、button集合、wire mapping、動的変換 | M0-M1 |
| L3 Pure protocol | report/parser/reply/SPI/conversion | M1 |
| L4 Runtime semantics | Periodic/Direct/readiness/cleanup | M2 |
| L5 Virtual Bluetooth | Classic/SDP/HID/pairing | M4 |
| L6 Profile data | Python 0.6.0 schema v2 Classic fieldsの相互読書き | M6 |
| L7 Hardware behavior | target matrix | M5-M8 |
| L8 Operational cutover | docs/probe/monitoring/backend rollback | M9 |

L1の型制約について、人工的な不正コードをcompileさせる専用testは作らない。public signatureとgeneric boundを設計の正本とし、domain dataとruntime behaviorを別途検証する。

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
| `SwitchGamepad` ABC | 初期Rust APIでは対応traitなし。generic `Controller<M,R>`を使う |
| `PeriodicSwitchGamepad` ABC | `R = reporting::Periodic` |
| `DirectSwitchGamepad` ABC | `R = reporting::Direct` |

6名は独立newtypeではなくaliasである。

## 4. existing profileのconstruction

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

- Rust builderは`ControllerBuilder<model::Pro, reporting::Periodic>`
- `build()`はexisting profileだけを読む
- nonexistent pathは`ProfileNotFound`
- adapter openは`open()`まで行わない
- profileは`PairingProfile<model::Pro>`として検証する
- Direct builderに`report_period()`はない
- `ControllerKind`やreporting modeを値で指定しない
- Rust controllerは`Clone`しない

`profile_path=None`はephemeral controllerであり、session内pairing keyだけを使う。

## 5. new profile factory mapping

Python:

```python
pad = await ProController.create_profile(
    adapter="usb:0",
    profile_path="profiles/pro.json",
    pair_timeout=60.0,
)
```

Rust:

```rust
let mut pad = ProController::builder("usb:0")
    .profile_path("profiles/pro.json")
    .create_profile(CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout: Duration::from_secs(60),
    })?;
```

Rustの`create_profile()`はcontroller methodではなく、typed builderを消費する複合操作である。

順序:

1. builder設定とtarget pathを検査
2. `M::KIND`を持つvalid empty `PairingProfile<M>`を生成してcreate-new
3. 同じ`PairingProfile<M>`をruntime configへ移譲
4. controllerを構築
5. adapter / workerをopen
6. pairingとprotocol readiness
7. Ready controllerを返す

理由:

- profile identityを永続化する前にadapterをpower onしない
- 保存bytesとruntime configのcontroller kind / identityを同じ型付き値から作る
- 後続のbuild/open/reconnectでは保存済みprofileを再読込して検証する
- pairing failure後もvalid empty envelopeから明示retryできる
- partial controller objectを利用者へ返さない

create-newが既存pathと競合した場合は`ProfileAlreadyExists`、path未指定は`ProfilePathRequired`。
target existenceは事前検査しない。pairing失敗ではenvelopeを残し、内部controllerをcleanupする。

既存empty profileからのretry:

```rust
let mut pad = ProController::builder("usb:0")
    .profile_path("profiles/pro.json")
    .build()?;
pad.open()?;
pad.pair(Duration::from_secs(60))?;
```

## 6. model-specific button mapping

Pythonは全modelで共通`Button` enumを使い、unsupported inputを実行時に拒否する。

```python
await pro.tap(Button.A)
await right.tap(Button.A)
```

Rust:

```rust
pro.tap([ProButton::A], Duration::from_millis(80))?;
right.tap([JoyConRButton::A], Duration::from_millis(80))?;
```

`ProButton::A`と`JoyConRButton::A`は内部で`ButtonKind::A`に対応するが型は別。`JoyConLButton::A`は存在しない。

| Python `Button` | Pro | Joy-Con L | Joy-Con R |
|---|---|---|---|
| A/B/X/Y | `ProButton::*` | 非対応 | `JoyConRButton::*` |
| L/ZL | `ProButton::*` | `JoyConLButton::*` | 非対応 |
| R/ZR | `ProButton::*` | 非対応 | `JoyConRButton::*` |
| Plus/Home | `ProButton::*` | 非対応 | `JoyConRButton::*` |
| Minus/Capture | `ProButton::*` | `JoyConLButton::*` | 非対応 |
| D-pad | `ProButton::*` | `JoyConLButton::*` | 非対応 |
| LeftStick click | Pro | Joy-Con L | 非対応 |
| RightStick click | Pro | 非対応 | Joy-Con R |
| SL/SR | 非対応 | Joy-Con L | Joy-Con R |

button名を設定ファイルから読む場合は`ButtonKind`をparseし、選択済みmodelの`Button<M>::try_from()`を呼ぶ。この動的変換は実行時testの対象である。

## 7. button wire mapping

`ButtonKind`の数値は論理IDであり、NX reportのbyte/bit位置ではない。

```text
(ControllerKind, ButtonKind)
    → byte index
    → bit mask
```

Python基準断面からmodel別mapping fixtureを作る。特にJoy-Con L/Rの`SL`と`SR`はmodelを含めて比較する。

移行時に検査するもの:

- supported button全てにmappingがある
- unsupported buttonへmappingを公開しない
- reserved bitを立てない
- Pythonのreport bytesと一致する
- `ButtonKind as u8`をwire offsetへ直接使わない

## 8. input state mapping

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

`InputState<model::Pro>`をJoy-Conへ渡せない。modelを抽象化する箇所はgeneric functionにするか、動的入口で`ControllerKind`を分岐する。

## 9. common value mapping

| Python | Rust | 備考 |
|---|---|---|
| `Stick` | `Stick` | 座標値は共通 |
| `IMUFrame` | `ImuFrame` | 六軸センサー値は共通 |
| 1 frame / 3 frames | `ImuSamples::Repeat` / `Frames` | invalid slice lengthを除去 |
| `ControllerColors` | `ControllerColors` | model specがwire利用を決定 |
| float seconds | `Duration` | 単位を型で明示 |

stick method:

```rust
pro.left_stick(stick)?;
pro.right_stick(stick)?;
pro.sticks(left, right)?;
left.left_stick(stick)?;
right.right_stick(stick)?;
```

六軸入力:

```rust
pad.imu(ImuFrame::neutral())?;
pad.imu([frame0, frame1, frame2])?;
```

`ImuFrame`をmodelごとに分けない。wire packing差は`M::SPEC`から選ぶ。

## 10. reporting semantics mapping

| Python | Rust |
|---|---|
| Periodic `apply(state)` | `Controller<M, Periodic>::apply(InputState<M>)` |
| Direct `send(state)` | `Controller<M, Direct>::send(InputState<M>)` |
| `report_period_us` | Periodic builderの`report_period(Duration)` |

Periodic:

- local state commit後成功
- 未接続中もstate更新可能
- next tickでlatest state
- send failureでrollbackしない

Direct:

- connected必須
- transport acceptance後だけcommit
- acceptance前failureでprevious state維持
- user-input schedulerなし

## 11. resource scope mapping

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

Rust`Drop`にPython context managerと同じcleanup保証を持たせない。

## 12. connection mapping

| Python | Rust |
|---|---|
| `await open()` | `open()` |
| classmethod `create_profile()` | builder `create_profile()` |
| `await pair(timeout=...)` | `pair(Duration)` |
| `await reconnect(timeout=...)` | `reconnect(Duration)` |
| `await connect(...)` | `connect(ConnectOptions)` |
| `await try_reconnect(...)` | `try_reconnect(Duration)` |
| `await try_connect(...)` | `try_connect(ConnectOptions)` |
| `await close(neutral=True)` | `close()` |
| `await close(neutral=False)` | `close_without_neutral()` |

正常接続はlink/HID channel openだけでなく、report mode、player lights、reply acceptance、handshake回収、Periodic holdoffを含むreadinessまで待つ。

## 13. profile schema v2

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

```text
ProfileDocument { controller_kind, identity, key_store }
  ↓ compare with M::KIND
PairingProfile<M>
```

`PairingProfile<model::Pro>`をJoy-Conへ渡すAPIは作らない。

key fieldsは`link_key.value` 16-byte hex、`link_key.authenticated` bool、
`link_key_type` u8だけとする。peer名はpublic addressの`XX:XX:XX:XX:XX:XX/P`、namespaceは
local Bluetooth addressとする。unknown field、旧Rustのraw peer名、`address_type`、LE key fieldを
profile入力境界で拒否し、key materialをlogしない。

write:

- UTF-8
- two-space indent
- deterministic key order
- trailing newline
- same-directory temporary file
- flush / `sync_all`
- create-newはno-replace
- updateはatomic replace
- 同一pathの複数live writerは非対応。lock、CAS、競合検出は提供しない

自動backup、世代管理、復元機能は実装しない。更新中断では旧または新のvalid fileが残ることを保証する。

## 14. dynamic application migration

modelが設定やCLIで決まるapplicationは入口で一度だけ分岐する。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後は`Button<M>`と`InputState<M>`を維持する。core全体を`ControllerKind + ButtonKind + untyped state`へ戻さない。

異なるmodelを同じcollectionに置く必要がある場合も、初期libraryに`AnyController`を追加せずapplication側enumで必要操作を明示する。

## 15. diagnostics mapping

Python `DiagnosticsConfig` writerはRustでは`tracing` subscriberへ移す。

`GamepadStatus`:

- lifecycle
- connected
- `controller_kind` (`M::KIND`)
- `reporting_kind` (`R::KIND`)
- report mode
- accepted counters
- last subcommand
- disconnect reason
- worker failure

運用logにはruntime projectionを記録する。

## 16. error mapping

| Python | Rust `ErrorKind` |
|---|---|
| `AdapterDiscoveryError` | `AdapterDiscovery` |
| `TransportOpenError` | `TransportOpen` |
| `ClosedError` | `TransportClosed` |
| new profile path未指定 | `ProfilePathRequired` |
| existing profile path不存在 | `ProfileNotFound` |
| create target existing | `ProfileAlreadyExists` |
| `ConnectionTimeoutError` | `ConnectionTimeout` |
| `ConnectionFailedError` | `ConnectionFailed` |
| `InvalidInputError` | `InvalidInput` |
| `UnsupportedInputError` | `UnsupportedInput` |
| `InvalidProfileError` | `InvalidProfile` |
| `ProfileControllerMismatchError` | `ProfileControllerMismatch` |
| `InvalidKeyStoreError` | `InvalidKeyStore` |
| `AdapterIdentityRecoveryRequired` | `UnsupportedCapability`またはidentity-specific error |

静的Rust APIでは多くのunsupported inputが型として表現不能になる。`UnsupportedInput`は動的境界に残る。

## 17. 段階移行

### Phase A: observation

source SHA、protocol fixture、profile fixture、hardware trace、supported input matrixを固定する。

### Phase B: type/API foundation

- `Controller<M,R>`とaliasを実装
- model固有`Button<M>` / `InputState<M>`を実装
- common `Stick` / `ImuFrame`を実装
- model宣言を単一正本にする
- public examplesとrustdocを通常のbuildで確認
- model集合とwire mappingをfixtureで検証

compilerが保証する不正な型の拒否を、専用compile-fail suiteで再検査しない。

### Phase C: pure protocol shadow

同じsemantic inputをPythonとRustへ流しbytes/effectsを比較する。Python共通`Button`をfixture generatorでmodel付きlogical inputへ変換する。

### Phase D: virtual Bluetooth

Bumble virtual linkでClassic、SDP、HID、pairing、typed inputを通す。

### Phase E: hardware canary

専用adapterと新規profileでbuilder `create_profile()`を使う。

criteria:

- envelopeがadapter open前に作成される
- clean pairing
- model-supported input reflection
- neutral / close
- repeated run
- no identity mutation
- trace redaction

### Phase F: profile interoperability

synthetic profile、専用hardware profile、既存profile copyの順にread/write/reconnectする。自動backup機能は使わない。

### Phase G: workload cutover

- failure rate
- connect latency
- report jitter
- close latency
- reconnect success
- profile update count
- model / mapping audit

backend config switchを維持する。

### Phase H: Python retirement

model/reportingごとに判定する。

- supported release
- hardware matrix
- public API review
- model / mapping audit
- profile compatibility
- operational docs
- diagnostics
- unresolved S1なし
- backend rollback rehearsal

## 18. application boundary

優先順:

1. applicationをRustへ移す
2. Rust CLIをsubprocess利用
3. narrow IPC daemonを別仕様で追加
4. FFI

PyO3 bindingを最初に作らない。Bluetooth ownership、shutdown、callback thread、wheel packagingが追加問題になる。

## 19. configuration単位

```toml
controller = "pro"
reporting = "periodic"
connect_timeout_ms = 30000
report_period_us = 8000
tap_duration_ms = 80
```

controller/reporting文字列を入口でmarker typeへ分岐する。同じfieldにsecondsとmillisecondsを混在させない。

## 20. backend rollback

```toml
controller_backend = "python" # or "rust"
profile_path = "profiles/pro.json"
```

同じprocessまたは複数processで両backendがadapterを同時openしない。switchはprocess restartを伴う。

checklist:

- Rust controller close
- worker exit
- adapter re-enumeration
- current profile hash / parse result
- Python environment lock
- Python profile load
- Python reconnectまたはfresh pairing
- input neutral
- incident trace保存

libraryはprofileの自動複製・復元を行わない。

## 21. 移行完了判定

完了:

- L0-L7
- target hardware evidence
- model / mapping audit
- profile create ordering / data safety
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
- profileをparseしただけ
- Bumble README capabilityだけ

ここでいう「compile successだけ」はBluetooth、protocol、profile互換の証拠にならないという意味である。型制約について別途compile-fail証拠を要求する意味ではない。

## 22. Python基準断面の将来更新

1. v0.6.0 baseline milestone完了
2. new release diff分類
3. bug fix / protocol discovery / API addition分離
4. fixture version追加
5. model能力表への影響確認
6. Rust APIへ取り込むか明示決定
7. old fixtureを削除しない

新しいPython機能がmodel固有能力を増やす場合、共通`Button`へ追加するだけで終えず、model宣言、`Button<M>`、wire mapping、migration表を同じ変更で更新する。

## 23. 移行記録

hardware / profile cutoverごとに、秘密情報を除いて次を記録する。

```text
date
application
controller model
reporting mode
old backend version
new swbt-rs commit
Bumble revision
OS / adapter / driver / console firmware
profile source hash
fresh pairing or reconnect
test result
backend rollback result
known limitations
```

link key、full profile、raw sensitive packetを含めない。
