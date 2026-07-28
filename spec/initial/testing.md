# テスト方針

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係: [type-modeling.md](type-modeling.md)
- architecture: [architecture.md](architecture.md)
- roadmap gate: [roadmap.md](roadmap.md)

この文書は、コンパイル時の能力制約、Python 基準断面との wire 差分、typed runtime、profile lifecycle、Bumble 仮想統合、adapter-only test、Switch 実機 test、CI の必須条件を定義する。

## 1. テスト分類

| 分類 | Switch実機 | USB adapter | Bumble | 目的 |
|---|---:|---:|---:|---|
| UI compile-pass/fail | 不要 | 不要 | 不要 | `Controller<M, R>`の型能力 |
| Pure unit | 不要 | 不要 | 不要 | 値型、model宣言、parser、encoder、profile |
| Golden differential | 不要 | 不要 | 不要 | Python v0.6.0とのwire/data比較 |
| Runtime integration | 不要 | 不要 | 不要 | typed worker、scheduler、Direct、cleanup |
| Transport contract | 不要 | 不要 | fakeまたはBumble | transport意味とfailure mapping |
| Bumble virtual | 不要 | 不要 | 必要 | Classic、pairing、SDP、HID統合 |
| Adapter-only | 不要 | 必要 | 必要 | USB open、HCI init、close、unplug |
| Hardware | 必要 | 必要 | 必要 | pairing、reconnect、input反映 |
| Fuzz / model | 不要 | 不要 | 原則不要 | malformed input、state machine、race |
| Packaging | 不要 | 不要 | build時に必要 | MSRV、docs、crate、license、examples |

Default CIはUI、Pure、Golden、Runtime、Transport contract、Bumble virtual、Packagingを必須にする。

## 2. 型制約のUI test

`trybuild`または同等のcompiler UI harnessを使う。

```text
tests/ui/
  pass/
    pro_button_a.rs
    joycon_r_button_a.rs
    common_imu_all_models.rs
    periodic_apply.rs
    direct_send.rs
  fail/
    joycon_l_has_no_a.rs
    joycon_l_rejects_pro_button.rs
    joycon_l_has_no_right_stick.rs
    joycon_r_has_no_left_stick.rs
    joycon_single_has_no_dual_sticks.rs
    direct_has_no_apply.rs
    periodic_has_no_send.rs
    pro_state_to_joycon.rs
    direct_builder_has_no_report_period.rs
    typed_builder_has_no_kind_setter.rs
```

compile-pass:

- `ProButton::A`を`ProController`に渡せる
- `JoyConRButton::A`を`JoyConR`に渡せる
- 同じ`ImuFrame`を全modelの`InputState<M>`に使える
- Proにleft/right/dual stick methodがある
- Joy-Con Lにleft stick、Joy-Con Rにright stickがある
- Periodicに`apply(InputState<M>)`
- Directに`send(InputState<M>)`

compile-fail:

- `JoyConLButton::A`が存在しない
- `ProButton::A`をJoy-Con Lへ渡せない
- Joy-Con Lに`right_stick()`がない
- Joy-Con Rに`left_stick()`がない
- 片側Joy-Conに`sticks()`がない
- Directに`apply()`がない
- Periodicに`send()`がない
- `InputState<Pro>`をJoy-Conへ渡せない
- Direct builderに`report_period()`がない
- typed builderに`ControllerKind` setterがない

compiler stderr全文を不必要に固定せず、意図した型不一致またはmethod absenceであることをレビューする。

## 3. model宣言audit

- markerごとに`ControllerModel::KIND`が一意
- `ControllerKind`とprofile文字列が一対一
- supported button集合に重複なし
- `Button<M>` associated constantsとsupported集合が一致
- `TryFrom<ButtonKind>`成功集合が一致
- stick capability trait実装がmodel表と一致
- `M::SPEC.kind == M::KIND`
- `M::SPEC.profile_name == M::PROFILE_NAME`
- 全supported buttonにwire mappingあり

macro文字列snapshotだけを根拠にせず、型と値の両方で検査する。

## 4. fixture原則

全golden fixtureにprovenanceを記録する。

```json
{
  "format": "swbt.fixture",
  "schema_version": 1,
  "source_repository": "niart120/swbt-python",
  "source_commit": "84d2723b127f70fc78e12f4496f5c40af0ccfb0a",
  "generator": "tools/generate_python_fixtures.py",
  "case": "pro.neutral.report_0x30"
}
```

fixture generatorが固定するもの:

- Python dependency lock / version
- source commit
- controller kind
- colors
- logical button / stick / IMU input
- timer byte
- report mode / IMU mode
- monotonic timestamp
- output report / subcommand payload

PythonをRust test実行時の必須dependencyにしない。

## 5. Pure unit tests

### 5.1 `ButtonKind`

- explicit discriminant `0x00..=0x13`
- 重複なし
- logical order
- numeric codeとNX wire bitを別契約として検査
- `(ControllerKind, ButtonKind)` mapping
- arbitrary numeric code parse error

`ButtonKind as u8`をreport offsetに使う実装をreviewで拒否する。

### 5.2 `Button<M>`

- Pro / Joy-Con L / Joy-Con Rのsupported set
- `kind()` projection
- dynamic `TryFrom` supportedのみ成功
- model間変換なし
- duplicate normalization
- stable logical iteration

### 5.3 `Stick`

- raw 0 / 2048 / 4095
- out-of-range
- normalized -1 / 0 / 1
- Pythonと同じasymmetric conversion
- rounding
- amount boundary
- NaN / infinity
- diagonal tilt

値型testをmodelごとに複製しない。能力有無はUI testで検査する。

### 5.4 `ImuFrame` / `ImuSamples`

- i16 boundary
- neutral
- partial constructor / with-method
- `0.070 dps/raw` round-trip
- `1/4096 G/raw` round-trip
- non-finite / overflow
- Repeatが3frameへ展開
- Framesが順序保持
- invalid slice lengthを受けるAPIがない

### 5.5 `InputState<M>`

各model:

- neutral
- model-specific buttons
- complete replacement
- semantic candidate
- clone / equality
- new session neutral reset
- model間変換なし

Proはdual sticks、Joy-Con Lはleft only、Joy-Con Rはright only。

### 5.6 colors / SPI

- default
- RGB24 boundary
- `0x6050` bytes
- custom colors
- model profile range
- input state非干渉

### 5.7 input report `0x30`

- 49 bytes
- report ID / timer
- battery / connection nibble
- typed button bytes
- model-specific stick packing
- vibrator byte
- 36-byte IMU block
- reserved bytes
- model差
- IMU mode
- deterministic time
- next encoding state
- timer wrap

### 5.8 parser / responder

- `0x01` / `0x10`
- malformed packet
- arbitrary bytes no panic
- raw rumble preservation
- subcommand `0x02` / `0x03` / `0x04` / `0x08` / `0x10` / `0x21` / `0x30` / `0x40` / `0x48`
- model-specific device info / SPI / elapsed button
- readiness effects
- unsupported command session不変
- reply prefixをtyped stateから取得

### 5.9 profile document / typed profile

- exact format / schema v2
- identity variants
- kind strings
- namespace shape
- malformed UTF-8 / JSON
- address validation
- multiple peer rejection
- key parse
- secret-safe Debug
- deterministic JSON
- `PairingProfile<M>`のkind mismatch
- typed profileを別modelへ渡すAPIがない

## 6. builderとprofile lifecycle tests

### 6.1 `build()`

- `profile_path=None`でephemeral controller
- existing matching profileでconfigured controller
- existing mismatched profileをadapter open前に拒否
- nonexistent pathは`ProfileNotFound`
- `build()`でadapter / worker / USBを作らない
- Direct configにperiod fieldが存在しない

### 6.2 `create_profile()`

fake filesystemとfake transportを使い、順序をevent logで検査する。

必須順序:

```text
profile_validate_target
profile_create_empty
profile_reopen_typed
transport_open
pairing_start
protocol_ready
return_controller
```

検証:

- profile path未指定は`ProfilePathRequired`
- target existingは`ProfileAlreadyExists`、上書きなし
- invalid identityはtransport open前に失敗
- empty envelope persistenceがtransport openより先
- envelopeのcontroller kindが`M::KIND`
- pairing failureでもvalid empty envelopeが残る
- failure時にworker / transportをcleanup
- success時のreturn controllerは`Ready`
- success後にtyped button operationを即時実行可能
- `Controller<M,R>`に`create_profile()` methodが存在しない
- existing empty profileのretryはbuild→open→pairで行う

explicit local addressが未対応の間は、`UnsupportedCapability`を返しadapter identity変更eventがないことを検査する。

### 6.3 persistence failure

- parent create failure
- temp create / write / flush / sync failure
- create-new race
- atomic replace failure
- lock contention
- orphan temp cleanup
- Windows rename semantics

自動backup testは作らない。更新中断では旧または新fileのどちらかがvalidであることを検査する。

## 7. Golden differential tests

```text
tests/fixtures/python-v0.6.0/
  protocol/
    pro/
      neutral-0x30.*
      button-a.*
      buttons-l-r.*
      sticks-boundaries.*
      imu-disabled.*
      imu-standard.*
      imu-quaternion.*
    joycon-l/
      dpad.*
      left-stick.*
      sl-sr.*
    joycon-r/
      button-a.*
      right-stick.*
      sl-sr.*
    output-report-parse.*
    subcommand-replies.*
    spi-ranges.*
  profile/
    pro.*
    joycon-l.*
    joycon-r.*
```

semantic fixture:

- Stick conversion
- IMU conversion
- model supported button matrix
- stick capability matrix
- readiness effect
- profile JSON
- PairingKeys field names

差分時はoffset、expected、actual、field decode、modelを表示する。

## 8. Runtime integration tests

fake transport / fake clockで、次の6組を共通harnessに通す。

```text
Pro × Periodic
Pro × Direct
JoyConL × Periodic
JoyConL × Direct
JoyConR × Periodic
JoyConR × Direct
```

### 8.1 lifecycle

- open / repeated open
- close / repeated close
- close without neutral
- reopen
- open failure
- worker panic / command termination
- Drop vs explicit close
- close中input reject

### 8.2 handshake

- linkだけではconnect完了しない
- control / interrupt両channel
- bootstrap neutral / retry
- valid subcommand後retry停止
- supported report mode
- non-zero lights
- acceptance failure
- ready前disconnect
- timeout
- stale session
- Periodic holdoff
- Direct no-holdoff

### 8.3 sender ordering

- reply / input単一sequence
- timer acceptance後更新
- failed sendでtimer不変
- IMU state acceptance後更新
- reply prefix typed state
- `0x40` ACK ordering
- close neutral / pending reply

### 8.4 Periodic

- state commit
- deadline progression
- overrun skip / no burst
- latest state
- holdoff
- disconnect pause
- new session
- 8ms default

### 8.5 Direct

- no periodic user input
- success commit
- validation / rejection no commit
- accepted then disconnectはcommit済み
- semantic helper report 1件
- tap transaction
- release failure retains pressed

### 8.6 typed command invariant

- Periodic command型にSendなし
- Direct command型にApplyなし
- command payloadが`Button<M>` / `InputState<M>`
- worker commandにprofile create-newなし
- runtime coreにuntyped button vectorなし
- status kindを`M::KIND` / `R::KIND`から導出

## 9. Transport contract tests

model-independent suite:

- open前send reject
- repeated open
- poll timeout
- control / interrupt routing
- acceptance意味
- disconnect exactly once
- close idempotence
- terminal source
- bounded queue
- key非出力

transport contractに`Button<M>`やcontroller-specific stateを入れない。

## 10. Bumble virtual integration

```text
ControllerWorker<M, R>
      ↕ TransportPort
Bumble Device + LocalLink
      ↕
Switch test peer
```

- inquiry / connection / role
- authentication / encryption
- stored link key reconnect
- SDP `0x0001`
- HID `0x0011` / `0x0013`
- reverse channel order
- malformed SDP / HIDP
- model-specific service record
- typed report
- disconnect cleanup

6組を通し、pairing / SDPがreporting modeに依存しないことを確認する。

## 11. Adapter-only tests

```text
cargo test --features adapter-tests --test adapter_open -- --ignored
```

- no-open discovery
- selector / aliases
- USB claim
- HCI reset / capabilities
- local address
- Classic capability
- repeated 100 runs
- unplug
- permission / driver error
- process exit後再open

## 12. Hardware tests

初期matrix:

| ID | OS | dongle | driver | console | firmware | model | reporting | identity | status |
|---|---|---|---|---|---|---|---|---|---|
| W11-CSR-S2-P-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Pro | Periodic | adapter-default | 要検証 |
| W11-CSR-S2-P-D | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Pro | Direct | adapter-default | 要検証 |
| W11-CSR-S2-L-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con L | Periodic | adapter-default | 要検証 |
| W11-CSR-S2-R-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con R | Periodic | adapter-default | 要検証 |

stages:

1. adapter open
2. profile create-newまたはexisting load
3. discoverability
4. pairing
5. HID channels
6. subcommand readiness
7. model-supported button
8. model-supported stick
9. IMU
10. neutral
11. close
12. reconnect
13. Direct

「report accepted」と「Switch UI反映」を区別する。

## 13. Profile compatibility tests

| writer | reader | expectation |
|---|---|---|
| Python v0.6.0 | Rust `PairingProfile<M>` | matching modelのみ成功 |
| Rust | Python v0.6.0 | same kind / identity / keys |
| Rust | Rust | stable normalized JSON |
| future schema | current Rust | explicit unsupported error |

actual hardware keyをfixtureにcommitしない。

## 14. Fuzz / concurrency

fuzz:

- output parser
- HIDP adapter
- SDP boundary
- profile JSON
- PairingKeys hex
- SPI range
- ButtonKind parser

invariant:

- no panic
- no unbounded allocation
- deterministic error category
- no secret echo
- unsupported dynamic buttonから`Button<M>`を生成しない

concurrency model:

- close vs send
- disconnect vs tap release
- worker failure vs response wait
- queue full vs shutdown
- status update vs worker exit
- Drop vs explicit close

## 15. timing

専用jobで測定:

- 8ms period
- accepted timestamp
- p50 / p95 / p99 / max jitter
- overrun
- command / reply latency
- idle CPU
- shutdown latency

Default CIはfake clock invariantを必須にする。

## 16. CI

required:

```text
fmt
check-msrv
check-stable
clippy
ui-types
unit
golden
runtime-integration
transport-contract
bumble-virtual
doc
examples
miri-selected
cargo-deny
fixture-audit
model-audit
```

```bash
cargo fmt --all --check
cargo +1.87 check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --test ui
cargo doc --no-deps
cargo deny check
```

optional:

```text
adapter-windows
hardware-windows
timing-dedicated
fuzz-nightly
linux-experimental
```

## 17. release evidence

- source commit
- Bumble revision
- Cargo.lock hash
- CI run
- UI test result
- model declaration audit
- fixture provenance
- profile creation ordering test
- hardware matrix
- adapter-only run
- known failures
- license report
- crate checksum
- profile round-trip

compile successだけで型能力を保証した扱いにしない。意図した不正コードがコンパイル不能であることをrelease gateに含める。