# テスト方針

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係: [type-modeling.md](type-modeling.md)
- architecture: [architecture.md](architecture.md)
- roadmap gate: [roadmap.md](roadmap.md)

この文書は、コンパイル時の能力制約、Python 基準断面との wire 差分、generic runtime、Bumble 仮想統合、adapter-only test、Switch 実機 test、CI の必須条件を定義する。

## 1. テスト分類

| 分類 | Switch 実機 | USB adapter | Bumble | 目的 |
|---|---:|---:|---:|---|
| UI compile-pass/fail | 不要 | 不要 | 不要 | `Controller<M, R>` の型能力を固定 |
| Pure unit | 不要 | 不要 | 不要 | 値型、model 宣言、parser、encoder、profile validation |
| Golden differential | 不要 | 不要 | 不要 | Python v0.6.0 と wire / data behavior を比較 |
| Runtime integration | 不要 | 不要 | 不要 | typed worker、scheduler、Direct transaction、cleanup |
| Transport contract | 不要 | 不要 | fake または Bumble | transport の意味と failure mapping |
| Bumble virtual integration | 不要 | 不要 | 必要 | Classic、pairing、SDP、HID を software controller で統合 |
| Adapter-only | 不要 | 必要 | 必要 | USB open、HCI init、close、unplug |
| Hardware | 必要 | 必要 | 必要 | pairing、reconnect、subcommand、入力反映 |
| Fuzz / model | 不要 | 不要 | 原則不要 | malformed input、state machine、race |
| Packaging | 不要 | 不要 | build 時に必要 | MSRV、docs、crate、license、examples |

Default CI は UI、Pure、Golden、Runtime、Transport contract、Bumble virtual、Packaging を必須にする。Adapter-only と Hardware は専用 runner または developer machine で実行する。

## 2. 型制約の UI test

`trybuild` または同等の compiler UI harness を使う。rustdoc の `compile_fail` だけを唯一の証拠にしない。

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
```

必須 compile-pass:

- `ProButton::A` を `ProController` に渡せる
- `JoyConRButton::A` を `JoyConR` に渡せる
- 同じ `ImuFrame` を Pro、Joy-Con L、Joy-Con R の `InputState<M>` に使える
- Pro に `left_stick()`、`right_stick()`、`sticks()` がある
- Joy-Con L に `left_stick()` がある
- Joy-Con R に `right_stick()` がある
- Periodic に `apply(InputState<M>)` がある
- Direct に `send(InputState<M>)` がある

必須 compile-fail:

- `JoyConLButton::A` が存在しない
- `ProButton::A` を Joy-Con L controller に渡せない
- Joy-Con L に `right_stick()` がない
- Joy-Con R に `left_stick()` がない
- 片側 Joy-Con に `sticks(left, right)` がない
- Direct に `apply()` がない
- Periodic に `send()` がない
- `InputState<model::Pro>` を Joy-Con L/R に渡せない
- Direct builder に `report_period()` がない
- `ControllerKind` を typed builder へ指定できない

compiler stderr の全文を不必要に固定せず、対象 error が意図した型不一致または method absence であることをレビューする。Rust version 更新による文言差と API 退行を区別する。

## 3. model 宣言 audit

model 宣言を単一正本に保つため、生成物または projection の整合を検査する。

- marker type ごとに `ControllerModel::KIND` が一意
- `ControllerKind` variant と profile 文字列が一対一
- model の supported button 集合に重複がない
- `Button<M>` associated constant が supported 集合と一致
- `TryFrom<ButtonKind> for Button<M>` の成功集合が associated constant と一致
- `HasLeftStick` / `HasRightStick` / `HasDualSticks` が model 表と一致
- `M::SPEC.kind == M::KIND`
- `M::SPEC.profile_name == M::PROFILE_NAME`
- 全 supported button に wire mapping がある
- unsupported button に wire mappingが存在しても public `Button<M>` を生成できない

macro expansion の文字列 snapshot だけを品質根拠にしない。生成結果を型と値の両方から検査する。

## 4. fixture の原則

全 golden fixture に provenance を記録する。

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

fixture generator は次を固定する。

- Python dependency lock と Python version
- source commit
- controller kind
- reporting semantics に必要な state
- colors
- typed inputを構成する元の論理 button / stick / IMU
- timer byte
- report mode / IMU mode
- monotonic timestamp input
- output report / subcommand payload

Python は Rust test 実行時の必須 dependency にしない。CI は commit 済み fixture を読む。

## 5. Pure unit tests

### 5.1 `ButtonKind`

- explicit discriminant `0x00..=0x13`
- discriminant の重複なし
- `ButtonKind` の logical order
- numeric code が NX wire bit と暗黙同一視されていないこと
- `(ControllerKind, ButtonKind)` wire mapping table
- reserved bit / unsupported mapping policy
- arbitrary numeric code からの parse error

`ButtonKind as u8` を report offset に使う実装を source audit または API review で拒否する。wire mapping test は byte index と bit mask を明示する。

### 5.2 `Button<M>`

- Pro の supported set
- Joy-Con L の supported set
- Joy-Con R の supported set
- `kind()` が対応する `ButtonKind` を返す
- dynamic `TryFrom` が supported だけ成功
- `Debug` が model と論理名を識別できる
- model 間の変換を提供していない
- duplicate input の bitset 正規化
- stable logical iteration order

### 5.3 `Stick`

- raw `0` / `2048` / `4095`
- out-of-range error
- normalized `-1.0` / `0.0` / `1.0`
- Python と同じ asymmetric center conversion
- half-way rounding
- amount `0.0` / `1.0`
- NaN / infinity reject
- diagonal `tilt(1.0, 1.0)`
- direction helper sign

`Stick` のテストを model ごとに複製しない。capability の有無は UI test で検査する。

### 5.4 `ImuFrame` / `ImuSamples`

- raw i16 min / max
- neutral
- accel / gyro partial constructor
- with-method が反対側を保持
- `0.070 dps/raw` の rad/s round-trip
- `1/4096 G/raw` の G round-trip
- non-finite / conversion overflow
- `ImuSamples::Repeat` が 3 frame へ展開
- `ImuSamples::Frames` が順序を保持
- slice 長 0 / 2 / 4 を受ける API が存在しない

共通 `ImuFrame` が全 model の state に使えることは compile-pass test で固定する。model 固有の wire encoding 差だけを model 別 golden test に置く。

### 5.5 `InputState<M>`

各 model について:

- neutral buttons / sticks / IMU
- model-specific button set
- complete replacement
- semantic candidate generation
- clone / equality
- new session neutral reset
- model 間変換がない
- stable serialization が public contract でない

Pro:

- left / right / dual stick builder

Joy-Con L:

- left stick state だけを保持

Joy-Con R:

- right stick state だけを保持

内部に unsupported stick の `Option` を持つ場合でも、public API と invariant test から不正値を構築不能にする。推奨は model associated `StickState` で不正状態を表現不能にすること。

### 5.6 `ControllerColors` / SPI

- default values
- 24-bit boundary
- `0x6050` bytes
- custom colors
- model profile 固有 range
- color mutation が `InputState<M>` に影響しない

### 5.7 input report `0x30`

- total 49 bytes
- report id
- timer
- battery / connection nibble
- typed button bytes
- model-specific stick packing
- vibrator byte
- 36-byte IMU block
- reserved bytes
- Pro / Joy-Con L / Joy-Con R 差
- disabled / standard / quaternion IMU mode
- deterministic explicit time
- next encoding state
- timer wrap

### 5.8 output report parser / responder

- `0x01` packet id / rumble / subcommand
- `0x10` rumble-only
- malformed short packet
- arbitrary bytes never panic
- raw rumble preservation
- subcommand `0x02` / `0x03` / `0x04` / `0x08` / `0x10` / `0x21` / `0x30` / `0x40` / `0x48`
- model-specific device info / SPI / elapsed button mapping
- report mode + non-zero lights readiness
- unsupported command が session を壊さない
- reply state prefix を sender が typed state から取得

### 5.9 profile document

- exact `format`
- schema v2 only
- identity variants
- controller kind strings
- namespace shape
- malformed UTF-8 / JSON
- address validation
- multiple current peer rejection
- key field parse
- secret-safe Debug
- deterministic normalized JSON
- `PairingProfile<M>` が `M::KIND` mismatch を拒否
- typed profile を別 model controller へ渡す API がない

## 6. Golden differential tests

fixture set:

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

wire bytes 以外に次を fixture 化する。

- Stick normalized result
- IMU physical conversion
- model supported button matrix
- stick capability matrix
- readiness effect
- profile normalized JSON
- PairingKeys field names

差分時は offset、expected、actual、field decode、model を表示する。

## 7. Runtime integration tests

fake transport と fake clock を使い、wall clock wait を避ける。最低限、`ControllerWorker<M, R>` の次の組み合わせを通す。

```text
Pro × Periodic
Pro × Direct
JoyConL × Periodic
JoyConL × Direct
JoyConR × Periodic
JoyConR × Direct
```

同じ generic contract を macro または共通 harness で実行し、6 組の test body を手作業で複製しない。

### 7.1 lifecycle

- build で adapter を開かない
- existing typed profile validation
- absent profile path を create target として保持
- open / repeated open
- close / repeated close
- close without neutral
- reopen
- open failure
- worker panic
- command termination
- Drop best-effort と explicit close の差
- close 中 input reject

### 7.2 handshake

- link だけでは connect 完了しない
- control / interrupt の両 channel
- bootstrap neutral と 1 秒 retry
- valid subcommand 後 retry 停止
- supported `0x03 30`
- non-zero player lights
- reply acceptance failure
- ready 前 disconnect
- timeout
- stale session reject
- Periodic holdoff
- Direct no-holdoff / no-confirmation-report

### 7.3 sender ordering

- reply と input の単一 sequence
- timer acceptance 後更新
- failed send で timer 不変
- IMU state acceptance 後更新
- reply prefix の typed state
- `0x40` ACK ordering
- close neutral と pending reply
- disconnect と queued send

### 7.4 Periodic

- state commit
- exact deadline progression
- overrun skip
- no burst catch-up
- latest state
- holdoff
- disconnect pause
- new session epoch
- 8 ms default
- fake clock 長時間 run

### 7.5 Direct

- no periodic user input
- send success commit
- validation / transport rejection no commit
- accepted then disconnect は commit 済み
- semantic helper report 1 件
- tap press / release transaction
- release failure retains pressed state
- pre-existing buttons preserved

### 7.6 typed command invariant

- Periodic worker command typeに `Send` がない
- Direct worker command typeに `Apply` がない
- command payload が `Button<M>` / `InputState<M>` を保持
- model 無し raw button vector を worker command に渡さない
- status kind は `M::KIND` / `R::KIND` から導出

## 8. Transport contract tests

fake と Bumble adapter が共有する model-independent contract suite を作る。

- open 前 send reject
- repeated open policy
- poll timeout
- control / interrupt routing
- send acceptance の意味
- disconnect exactly once
- close idempotence
- terminal source
- bounded queue
- no key in Debug

transport contract に `Button<M>` や controller-specific入力を入れない。typed state は protocol encoderより上で検証済みとする。

## 9. Bumble virtual integration

```text
ControllerWorker<M, R>
      ↕ TransportPort
Bumble Device + LocalLink / software controller
      ↕
Switch test peer
```

physical HCI transportを使わず、Classic、pairing、SDP、HIDP、NX handshakeを通す。

- inquiry / connection request / role
- authentication / encryption
- stored link key reconnect
- SDP PSM `0x0001`
- HID control `0x0011` / interrupt `0x0013`
- reverse channel open order
- malformed SDP / HIDP
- model-specific service record
- typed inputから生成した report
- disconnect cleanup

全 model × reporting mode を通す。pairingやSDPが reporting modeに依存しないこと、input schedulingだけが modeに依存することを確認する。

## 10. Adapter-only tests

```text
cargo test --features adapter-tests --test adapter_open -- --ignored
```

環境変数:

```text
SWBT_ADAPTER=usb:0
SWBT_ADAPTER_RUNS=100
```

検証:

- no-open discovery
- primary selector / aliases
- USB claim
- HCI reset / capability query
- local address
- Classic capability
- repeated open / close
- unplug
- permission denied
- wrong driver
- process exit後の再open

model型はadapter-only testの結果を変えない。必要なlocal name等は `M::SPEC` からtransport configへ投影する単体testで確認する。

## 11. Hardware tests

初期matrix:

| ID | OS | dongle | driver | console | firmware | model | reporting | identity | status |
|---|---|---|---|---|---|---|---|---|---|
| W11-CSR-S2-P-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Pro | Periodic | adapter-default | 要検証 |
| W11-CSR-S2-P-D | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Pro | Direct | adapter-default | 要検証 |
| W11-CSR-S2-L-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con L | Periodic | adapter-default | 要検証 |
| W11-CSR-S2-R-P | Windows 11 | CSR8510 A10 | WinUSB | Switch 2 | 22.1.0 | Joy-Con R | Periodic | adapter-default | 要検証 |

stages:

1. adapter open
2. discoverability
3. pairing
4. HID channels
5. subcommand readiness
6. model-supported button
7. model-supported stick
8. IMU
9. neutral
10. close
11. reconnect
12. Direct

ProとJoy-Con RのAは別typed APIから同じ論理入力を生成する。実機では両方を個別に確認する。Joy-Con LでAを試すhardware testは作らず、compile-failとdynamic conversion rejectionで保証する。

「report accepted」と「Switch UI反映」を区別する。

## 12. Profile compatibility tests

| writer | reader | expectation |
|---|---|---|
| Python v0.6.0 | Rust `PairingProfile<M>` | matching modelだけ成功 |
| Rust | Python v0.6.0 | same kind / identity / keys |
| Rust | Rust | stable normalized JSON |
| future schema | current Rust | explicit unsupported error |

filesystem failure:

- target exists on create
- parent absent
- temp create / write / sync failure
- atomic replace failure
- lock contention
- process kill point simulation
- read-only / permission
- Windows rename semantics
- orphan temp cleanup

自動backupのtestは作らない。更新中断では更新前または更新後のvalid fileが残ることを検査する。

## 13. Fuzz / model testing

fuzz target:

- output report parser
- HIDP message parser adapter
- SDP boundary
- profile JSON parser
- PairingKeys hex conversion
- SPI range
- `ButtonKind` dynamic parser

invariant:

- no panic
- no unbounded allocation
- deterministic error category
- no secret echo
- parser out-of-boundsなし
- unsupported dynamic buttonから`Button<M>`を生成しない

concurrency model:

- close vs send
- disconnect vs tap release
- worker failure vs response wait
- queue full vs shutdown
- status update vs worker exit
- Drop vs explicit close

## 14. timing test

実時間testは専用jobに分ける。

- 8 ms period
- accepted timestamp
- p50 / p95 / p99 / max jitter
- overrun count
- command / reply latency
- idle CPU
- shutdown latency

Default CIはfake clock invariantを必須にし、CI VMのhard timing thresholdは使わない。

## 15. CI

required jobs:

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

代表command:

```bash
cargo fmt --all --check
cargo +1.87 check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --test ui
cargo doc --no-deps
cargo deny check
```

optional jobs:

```text
adapter-windows
hardware-windows
timing-dedicated
fuzz-nightly
linux-experimental
```

## 16. release evidence

release candidateごとに保存する。

- source commit
- Bumble revision
- Cargo.lock hash
- CI run
- UI type test result
- model declaration audit
- fixture provenance audit
- hardware matrix
- adapter-only run
- known failures
- license report
- crate checksum
- profile round-trip result

compile successだけでmodel capabilityを保証したと扱わない。意図した不正コードがコンパイル不能であることをrelease gateに含める。