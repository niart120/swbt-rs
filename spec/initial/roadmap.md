# 実装ロードマップ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係: [type-modeling.md](type-modeling.md)
- API: [api.md](api.md)
- architecture: [architecture.md](architecture.md)
- test gate: [testing.md](testing.md)

この文書は実装順序と milestone ごとの exit criteria を定義する。日付ではなく依存関係と証拠で進捗を判定する。

## 1. 実装順序

```text
M0 repository / dependency / type-model foundation
  ↓
M1 model-valid input + pure protocol parity
  ↓
M2 Controller<M, R> worker + fake transport
  ↓
M3 Bumble external HCI bring-up
  ↓
M4 virtual Classic SDP/HID integration
  ↓
M5 Pro Periodic fresh pairing
  ↓
M6 profile compatibility + reconnect + Pro Direct
  ↓
M7 Joy-Con L/R Periodic + Direct
  ↓
M8 IMU / diagnostics / probe
  ↓
M9 portability / release
```

型モデルを後付けしない。M0で model / reporting / button / state の型関係とcompile-fail harnessを固定し、その後のprotocolとruntimeをtyped path上に実装する。

各milestoneは`spec/wip/unit_連番/`で作業仕様を作り、完了後に`spec/complete/unit_連番/`へ移す。複数milestoneを一つの巨大PRにまとめない。

## 2. release target

### 2.1 `0.1.0-alpha.1`

対象:

- `Controller<model::Pro, reporting::Periodic>`
- alias `ProController`
- `ProButton` / `ProInputState`
- adapter-default identity
- fresh pairing
- Windows 11 + CSR8510 A10 + WinUSB
- UI / protocol / fake / virtual tests

非対象:

- reconnect保証
- Direct
- Joy-Con
- explicit local address
- Linux / macOS保証
- crates.io公開

### 2.2 `0.1.0-alpha.2`

追加対象:

- profile schema v2 read / write
- `PairingProfile<model::Pro>`
- stored link key reconnect
- `DirectProController`
- profile round-trip
- structured diagnostics
- `swbt-probe`

### 2.3 `0.1.0-beta.1`

追加対象:

- Joy-Con L/R Periodic / Direct
- model-specific button/state API
- IMU public conversion / wire parity
- limited Linux bring-up
- dependency / license inventory
- API reviewとsemver freeze候補

### 2.4 `0.1.0`

必要条件:

- required public API docsとexamples
- MSRV / stable CI
- UI compile-pass/fail gate
- model declaration audit
- Windows supported matrix
- profile update interruption test
- severity-high protocol / cleanup defectなし
- license決定
- reproducible build metadata
- unsupported platform / feature明記

## 3. M0: repository、dependency、type-model foundation

### 3.1 repository

- `src/lib.rs`を追加しlibrary target名を`swbt`にする
- placeholder `src/main.rs`を削除または後続CLIへ移す
- `rust-version = "1.87"`
- edition 2024
- `#![forbid(unsafe_code)]`
- `Cargo.lock`をcommit
- Bumble crateをexact revisionに固定
- fmt / clippy / test / docs / MSRV CI
- license方針をmaintainer decisionとして記録

### 3.2 type model

- `Controller<M, R>`
- model marker: Pro / JoyConL / JoyConR
- reporting marker: Periodic / Direct
- `ControllerModel` / `ReportingMode` sealed trait
- `ControllerKind` / `ReportingKind` runtime projection
- model宣言の単一正本
- `ButtonKind` explicit logical code
- `Button<M>`と3つのbutton alias
- `InputState<M>`と3つのstate alias
- `Stick` / `ImuFrame`共通値型
- `HasLeftStick` / `HasRightStick` / `HasDualSticks`
- `ControllerBuilder<M, R>`
- 6 controller alias
- `trybuild` UI harness

### 3.3 exit criteria

- `cargo +1.87 check --all-targets`
- current stable fmt / clippy / test / doc
- dependency sourceが単一Bumble revision
- model宣言からkind、profile名、button集合、stick能力を一意に導出
- `ProButton::A`と`JoyConRButton::A`がcompile-pass
- `JoyConLButton::A`がcompile-fail
- Pro buttonをJoy-Conへ渡すコードがcompile-fail
- Direct `apply()`とPeriodic `send()`がcompile-fail
- model違いの`InputState<M>`適用がcompile-fail
- `ImuFrame`を全modelで共有できる
- Direct builderに`report_period()`がない
- placeholder `Hello, world!`がない
- license未決ならrelease job無効

### 3.4 stop condition

型制約をruntime validationだけへ戻さないと実装できない問題が見つかった場合、protocol実装へ進まず型モデル仕様を再検討する。

## 4. M1: model-valid inputとpure protocol parity

### 4.1 対象

- `ButtonKind` / `Button<M>`
- `ButtonSet<M>`
- `Stick`
- `ImuFrame` / `ImuSamples`
- `InputState<M>`
- `ControllerColors`
- `ModelSpec`
- `0x30` input report builder
- `0x01` / `0x10` parser
- subcommand responder
- virtual SPI
- protocol session state
- IMU block encoder

### 4.2 作業

- Python基準断面からgolden fixture generator
- fixtureにsource SHAとmodelを記録
- Pro / Joy-Con L / Joy-Con Rのbutton集合を固定
- `(ControllerKind, ButtonKind)` wire mapping table
- `ButtonKind` discriminantとwire bitを分離
- model-specific stick layout
- neutral / button / stick / IMU report bytes
- output parser valid / malformed corpus
- subcommand `0x02` / `0x03` / `0x04` / `0x08` / `0x10` / `0x21` / `0x30` / `0x40` / `0x48`
- SPI known range / out-of-range policy
- protocol moduleがfilesystem、thread、Bumbleに依存しないことを検査

### 4.3 exit criteria

- neutral `0x30`が49 bytes
- Python fixtureとreport / replyがbyte-for-byte一致
- supported model buttonすべてにwire mapping
- unsupported model buttonを`Button<M>`へ変換できない
- Stick境界とrounding一致
- IMU変換一致
- malformed reportでpanicしない
- protocol testsがBumbleをlinkせず実行可能
- Miri selected test通過
- source audit fixture commit済み

### 4.4 非対象

- worker thread
- USB
- pairing
- profile filesystem write
- realtime scheduler

## 5. M2: `Controller<M, R>` runtimeとfake transport

### 5.1 対象

- generic controller / builder
- `ControllerWorker<M, R>`
- typed command channel
- lifecycle state machine
- `InputStateStore<M>`
- `ReportSender<M>`
- reporting policy
- Periodic scheduler
- Direct transaction
- handshake
- fake `TransportPort`
- deterministic clock

### 5.2 作業

- 6 model×reporting組み合わせを共通harnessで構築
- `PeriodicCommand<M>` / `DirectCommand<M>`分離
- open / close / reopen
- connection event injection
- `0x21` replyと`0x30`の共通送信順
- periodic deadline skip
- direct acceptance transaction
- tap cancellation / release failure
- disconnect / stale session
- worker panic propagation
- bounded queue / backpressure
- status projectionを`M::KIND` / `R::KIND`から生成
- activity wait microbenchmark

### 5.3 exit criteria

- wall-clock sleepなしで周期testが決定的
- Periodic state commit semantics一致
- Direct acceptance前failureでstate不変
- replyがinputに追い越されない
- close neutral / no-neutral test済み
- disconnect後typed stateがneutral reset
- reopenでtimer / IMU / readinessを引き継がない
- queue overflowがbounded error
- worker / thread leakなし
- runtime coreでuntyped button vectorを使わない
- runtime coreが毎操作`ControllerKind`をmatchしない

### 5.4 decision gate

activity wait方式を次で選ぶ。

- idle CPU
- 8 ms p50 / p95 / p99 jitter
- command response latency
- HCI event response latency
- shutdown latency

## 6. M3: Bumble external HCI bring-up

### 6.1 対象

- adapter selector
- no-open discovery
- USB transport open / split
- `ExternalHost`
- `Device` initialization
- model-independent `TransportPort`
- adapter diagnostics
- close

Switch実機は不要。USB Bluetooth adapterは必要。

### 6.2 作業

- `usb:0`、VID/PID、serial selector
- no-open USB HCI classification
- controller reset / capabilities
- local address read
- Classic capability
- permission / driver error mapping
- reader terminationとworker shutdown
- repeated open / close
- dependency build time / size測定
- `M::SPEC`からtransport configへのprojection test

### 6.3 exit criteria

- Windows target adapterで100回open / initialize / close
- no-open discoveryがhandle claimを残さない
- invalid selectorとpermission errorを区別
- local addressとHCI versionをtrace
- unplugを`TransportEnded`へ変換
- workerがunplug後join
- MSRV buildがBumble込みで通る
- dependency license report生成

### 6.4 upstream gate

次が不足する場合はM4前にupstream issue / PR。

- activity receiver access
- accepted Classic channel API
- discoverable / connectable policy
- key-store trait compatibility
- USB close / cancellation

## 7. M4: virtual Classic SDP/HID integration

### 7.1 対象

- `bumble-controller::LocalLink`
- Classic incoming connection / pairing
- SDP `0x0001`
- HID control `0x0011`
- HID interrupt `0x0013`
- `bumble_hid::DeviceRuntime`
- `SwbtHidChannelBridge`
- typed model protocol

### 7.2 作業

- Switch roleのvirtual peer
- SSP sequence
- stored link key reconnect
- model-specific SDP service record
- HID channel open order variation
- HIDP control request
- NX output report injection
- typed input / reply receive
- malformed HIDP / SDP
- MTU edge
- disconnect cleanup

### 7.3 exit criteria

- physical adapterなしでpair→SDP→HID→NX handshake
- channel open順が逆でも両方揃うまでreadyにしない
- SDP continuation
- HIDP request response
- invalid messageでpanicしない
- virtual reconnect
- fakeとBumble virtualのtransport contract一致
- 6 model×reporting組み合わせが共通suiteを通る

### 7.4 stop condition

virtual integrationを通せない状態で実機packetを手作業patchしない。

## 8. M5: Pro Periodic fresh pairing

### 8.1 対象環境

- Windows 11
- CSR8510 A10
- WinUSB
- Nintendo Switch 2
- firmware 22.1.0
- adapter-default
- `ProController`

### 8.2 作業

- discoverable / connectable window
- fresh pairing
- Pro SDP identity
- HID channels
- bootstrap neutral
- subcommand sequence / replies
- readiness
- 8 ms periodic input
- `ProButton::A`
- L+R hold
- dual sticks
- IMU sample
- close neutral / ACL drain
- 20回以上のclean pairing

### 8.3 exit criteria

- fresh pairing成功率と失敗理由記録
- control / interrupt / subcommand / ready trace
- A tapがUI反映
- L+Rを500 ms以上保持
- left/right stick反映
- neutral後入力残存なし
- close後adapter再open
- 20 successful run中hang / leaked handle / stale input 0
- hardware metadata保存
- alpha.1 release note draft

## 9. M6: profile compatibility、reconnect、Pro Direct

### 9.1 profile

- schema v2 raw DTO
- `PairingProfile<model::Pro>`
- Python fixture read
- Rust write→Python read
- key field preservation
- atomic create / replace
- lock contention
- model mismatch
- adapter-default namespace
- multiple peer rejection

### 9.2 reconnect

- stored Classic link key
- active / incoming bonded reconnect
- no-bond
- timeout
- stale / rejected bond
- explicit re-pair
- clean close

### 9.3 Direct

- `DirectProController`
- `ProInputState`
- `send`
- semantic helper transaction
- tap transaction
- no periodic user input
- close neutral exception
- Periodic profile reuse

### 9.4 exit criteria

- Python profileをtyped Rust profileがlossless read
- Rust profileをPythonがread
- Pro profileをPeriodic / Directで再利用
- power cycle reconnect
- invalid bondを暗黙削除しない
- Direct idle中periodic `0x30`なし
- Direct failureでsnapshotが前state
- update interruptionで旧または新fileがvalid
- key materialがlogへ出ない
- alpha.2 criteria達成

## 10. M7: Joy-Con L/R

### 10.1 型とprotocol

- `JoyConLButton` / `JoyConRButton`
- `JoyConLInputState` / `JoyConRInputState`
- left-only / right-only stick capability
- SL / SR
- model-specific device info / SPI / colors
- Periodic / Direct

### 10.2 作業順

1. UI compile-pass/fail
2. pure protocol fixtures
3. fake runtime
4. virtual Bluetooth
5. Joy-Con L hardware
6. Joy-Con R hardware
7. profile reuse
8. Direct

### 10.3 exit criteria

- Joy-Con LにAが存在しない
- Joy-Con RにD-padが存在しない
- Joy-Con Lにright stick methodがない
- Joy-Con Rにleft stick methodがない
- Joy-Con L D-pad / left stick / SL+SR実機確認
- Joy-Con R ABXY / right stick / SL+SR実機確認
- model mismatchをadapter open前に拒否
- colors SPI bytes一致
- Periodic / Direct readiness一致
- left / right evidenceを別runで保存
- `JoyConPair`を追加していない

## 11. M8: IMU、diagnostics、probe

### 11.1 IMU

- common `ImuFrame` / `ImuSamples`
- standard / quaternion mode
- same-mode re-request reset
- ACK ordering
- long-run stability
- model-specific encoder fixture

### 11.2 diagnostics

- stable event names
- `GamepadStatus`
- `M::KIND` / `R::KIND` projection
- environment snapshot
- accepted counters
- disconnect reason
- unsupported dynamic button
- session ID
- redaction

### 11.3 probe

```text
swbt-probe adapters
swbt-probe open --adapter usb:0
swbt-probe pair --controller pro --profile path --trace trace.jsonl
swbt-probe reconnect --controller pro --profile path --trace trace.jsonl
swbt-probe profile inspect path
swbt-probe profile verify path
```

CLIは`ControllerKind`をparseし、入口で`run::<M>()`へ分岐する。core操作をuntyped controllerへ戻さない。

### 11.4 exit criteria

- common IMU API compile-pass
- model別IMU fixture parity
- ACK ordering
- hardware traceでmode確認
- key / sensitive data非出力
- probe exit code / error category文書化
- JSONL sample
- beta.1 criteria達成

## 12. M9: portabilityとrelease

### 12.1 Windows

- driver setup
- device claim / release
- unplug
- package installation
- release artifact
- troubleshooting

### 12.2 Linux

- libusb permission / udev
- driver detach / reattach
- adapter open / close
- virtual tests
- hardware test
- supported / experimental label

### 12.3 macOS

初期対象外。USB transport、permission、driver ownershipを調査後に追加する。

### 12.4 release engineering

- public generic API review
- alias rustdoc
- compile-fail docs / UI tests
- examples compile
- `cargo package`
- license / SBOM
- changelog / semver
- security contact
- hardware matrix
- known limitations
- reproducible source baseline

### 12.5 exit criteria

- `0.1.0` checklist完了
- supported matrixでfresh pairing / reconnect
- clean install再現
- crateに秘密fixtureなし
- docs commandがcurrent codeで実行可能
- release commitとBumble revision記録
- application backend rollback手順

## 13. explicit local address milestone

M6以降の独立milestone。完了までproduction supportは`UnsupportedCapability`。

- adapter identity backend
- CSR8510 A10 semantics
- expected-address guard
- partial failure recovery
- power cycle test
- profile namespace compatibility
- duplicate address guidance

## 14. issue優先度

| severity | 条件 | 対応 |
|---|---|---|
| S0 | key漏えい、adapter永続破損、危険な副作用 | 開発停止、artifact無効化 |
| S1 | stale input、close hang、key loss、profile corruption、型保証の抜け道 | milestone blocker |
| S2 | reconnect failure、unsupported subcommand、high jitter | release blockerまたは制限 |
| S3 | diagnostics、ergonomics、build size | roadmap管理 |
| S4 | cosmetic docs / naming | 通常修正 |

## 15. progress evidence

milestone完了PRに含める。

- 対象仕様
- scope / non-goals
- test command / result
- UI compile-pass/fail result
- model declaration audit
- fixture provenance
- hardware matrix row
- 未検証事項
- cleanup / backend rollback
- Bumble revision / upstream issue

「コードがcompileする」だけで型能力、Bluetooth、protocol milestoneを完了扱いにしない。