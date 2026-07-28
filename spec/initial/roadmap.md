# 実装ロードマップ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係: [type-modeling.md](type-modeling.md)
- API: [api.md](api.md)
- architecture: [architecture.md](architecture.md)
- test gate: [testing.md](testing.md)

この文書は実装順序とmilestoneごとのexit criteriaを定義する。日付ではなく依存関係と証拠で進捗を判定する。

## 1. 実装順序

```text
M0 repository / dependency / type-model foundation
  ↓
M1 model-valid input + pure protocol parity
  ↓
M2 Controller<M, R> worker + typed profile frontend
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

型モデルを後付けしない。M0でmodel/reporting/button/stateの型関係とcompile-fail harnessを固定し、その後のprotocolとruntimeをtyped path上に実装する。

各milestoneは`spec/wip/unit_連番/`で作業仕様を作り、完了後に`spec/complete/unit_連番/`へ移す。

## 2. release target

### 2.1 `0.1.0-alpha.1`

対象:

- `Controller<model::Pro, reporting::Periodic>` / `ProController`
- `ProButton` / `ProInputState`
- typed builder `create_profile()`
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

- profile schema v2 read/write
- `PairingProfile<model::Pro>`
- stored link key reconnect
- `DirectProController`
- profile round-trip
- diagnostics
- `swbt-probe`

### 2.3 `0.1.0-beta.1`

追加対象:

- Joy-Con L/R Periodic/Direct
- model-specific button/state API
- common IMU conversion / wire parity
- limited Linux bring-up
- dependency/license inventory
- API review / semver freeze候補

### 2.4 `0.1.0`

必要条件:

- public API docs/examples
- MSRV/stable CI
- UI compile-pass/fail gate
- model declaration audit
- profile creation ordering test
- Windows supported matrix
- profile update interruption test
- severity-high defectなし
- license決定
- reproducible build metadata
- unsupported platform/feature明記

## 3. M0: repository、dependency、type model

### 3.1 repository

- `src/lib.rs`追加、library target名`swbt`
- placeholder binary削除または後続CLIへ移動
- `rust-version = "1.87"`
- edition 2024
- `#![forbid(unsafe_code)]`
- `Cargo.lock` commit
- Bumble exact revision
- fmt/clippy/test/docs/MSRV CI
- license方針記録

### 3.2 type model

- `Controller<M,R>`
- model marker: Pro/JoyConL/JoyConR
- reporting marker: Periodic/Direct
- sealed traits
- `ControllerKind` / `ReportingKind` projection
- model宣言単一正本
- `ButtonKind` explicit logical code
- `Button<M>`とbutton aliases
- `InputState<M>`とstate aliases
- common `Stick` / `ImuFrame` / `ImuSamples`
- stick capability traits
- `ControllerBuilder<M,R>`
- 6 controller aliases
- `trybuild` UI harness

### 3.3 exit criteria

- MSRV/stable check、fmt、clippy、test、doc
- dependency sourceが単一Bumble revision
- model宣言からkind/profile名/button/stick能力を一意導出
- ProとJoy-Con RのAがcompile-pass
- Joy-Con LのAがcompile-fail
- model違いbutton/stateがcompile-fail
- Direct apply / Periodic sendがcompile-fail
- common ImuFrameが全modelでcompile-pass
- Direct builderにreport periodなし
- typed builderにkind setterなし
- placeholder `Hello, world!`なし
- license未決ならrelease job無効

### 3.4 stop condition

型制約をruntime validationだけへ戻さないと実装できない問題が見つかった場合、M1へ進まず型仕様を更新する。

## 4. M1: model-valid inputとpure protocol

### 4.1 対象

- `ButtonKind` / `Button<M>` / `ButtonSet<M>`
- `Stick`
- `ImuFrame` / `ImuSamples`
- `InputState<M>`
- `ControllerColors`
- `ModelSpec`
- `0x30` builder
- `0x01` / `0x10` parser
- subcommand responder
- virtual SPI
- protocol session / IMU encoder

### 4.2 作業

- Python fixture generator
- source SHA/model metadata
- model button集合
- `(ControllerKind, ButtonKind)` wire mapping
- logical codeとwire bit分離
- model-specific stick layout
- neutral/button/stick/IMU bytes
- malformed parser corpus
- subcommand `0x02/03/04/08/10/21/30/40/48`
- SPI policy
- protocol moduleのfilesystem/thread/Bumble非依存

### 4.3 exit criteria

- neutral `0x30` 49bytes
- Python fixtureとbyte-for-byte一致
- supported button全てにmapping
- unsupported buttonを`Button<M>`へ変換不能
- Stick/IMU変換一致
- malformed inputでpanicなし
- protocol testがBumbleをlinkしない
- Miri selected通過
- source audit fixture commit済み

### 4.4 非対象

- worker
- USB
- pairing
- profile filesystem
- realtime scheduler

## 5. M2: typed runtimeとprofile frontend

### 5.1 対象

- generic controller/builder
- `ControllerWorker<M,R>`
- typed command channel
- lifecycle
- `InputStateStore<M>`
- `ReportSender<M>`
- reporting policy
- fake `TransportPort` / deterministic clock
- raw profile DTO / `PairingProfile<M>`
- builder `build()` / `create_profile()` frontend

### 5.2 作業

- 6 model×reporting組み合わせを共通harnessで構築
- `PeriodicCommand<M>` / `DirectCommand<M>`分離
- open/close/reopen
- sender ordering
- periodic deadline skip
- direct acceptance transaction
- tap cancellation/release failure
- stale session
- worker panic/backpressure
- statusを`M::KIND`/`R::KIND`から生成
- `build()` existing/ephemeral semantics
- `create_profile()` fake filesystem orchestration
- envelope create-new→transport open→pairing順序
- failure cleanupとempty envelope残存
- activity wait benchmark

### 5.3 exit criteria

- wall-clock sleepなしで決定的test
- Periodic/Direct commit semantics一致
- reply ordering一致
- close neutral/no-neutral
- disconnect/reopen reset
- queue overflow bounded
- thread leakなし
- runtime coreにuntyped button vectorなし
- runtime coreが毎操作ControllerKind matchしない
- nonexistent build pathはProfileNotFound
- create target existingは上書きなし
- envelopeがtransport openより先
- create failureでresource cleanup
- success時Ready controller返却
- controllerにcreate_profile methodなし

### 5.4 decision gate

activity wait方式をidle CPU、8ms jitter、command/HCI latency、shutdown latencyで選ぶ。

## 6. M3: Bumble external HCI bring-up

### 6.1 対象

- adapter selector / no-open discovery
- USB open/split
- `ExternalHost`
- `Device` initialization
- model-independent `TransportPort`
- adapter diagnostics / close

Switch実機は不要。USB adapterは必要。

### 6.2 作業

- `usb:0`、VID/PID、serial
- HCI classification
- reset/capability/local address
- Classic capability
- permission/driver errors
- reader termination
- repeated open/close
- build time/size
- `M::SPEC`→TransportConfig projection

### 6.3 exit criteria

- target adapterで100回open/init/close
- no-open discoveryがclaimを残さない
- error分類
- local address/HCI version trace
- unplug→TransportEnded
- worker join
- Bumble込みMSRV
- license report

### 6.4 upstream gate

activity receiver、accepted Classic channel、discoverable policy、key-store trait、USB cancellationが不足する場合はM4前にupstream対応。

## 7. M4: virtual Classic SDP/HID

### 7.1 対象

- LocalLink/software controllers
- incoming Classic/pairing
- SDP `0x0001`
- HID `0x0011/0x0013`
- `DeviceRuntime`
- HID bridge
- typed protocol

### 7.2 作業

- Switch role peer
- SSP/stored key
- model-specific SDP
- channel order variation
- HIDP control
- NX output injection
- typed input/reply
- malformed PDU/MTU
- disconnect cleanup

### 7.3 exit criteria

- physical adapterなしでpair→SDP→HID→NX handshake
- reverse channel order対応
- SDP continuation
- invalid messageでpanicなし
- virtual reconnect
- transport contract一致
- 6組共通suite通過

### 7.4 stop condition

virtual integration未通過で実機packetを場当たりpatchしない。

## 8. M5: Pro Periodic fresh pairing

### 8.1 環境

- Windows 11
- CSR8510 A10
- WinUSB
- Switch 2 firmware 22.1.0
- adapter-default
- `ProController`

### 8.2 作業

- builder `create_profile()`
- empty envelope persistence before USB open
- discoverable/pairing
- Pro SDP/HID
- bootstrap/subcommands/readiness
- 8ms periodic
- ProButton A、L+R、dual sticks、IMU
- neutral/close/drain
- 20回clean pairing

### 8.3 exit criteria

- event順序がprofile create→transport open→pairing
- fresh pairing成功率記録
- protocol trace
- A UI反映
- L+R 500ms
- sticks反映
- neutral残存なし
- close後再open
- 20 runでhang/leak/stale input 0
- failure時empty profileがvalid
- hardware metadata
- alpha.1 note draft

## 9. M6: profile compatibility、reconnect、Pro Direct

### 9.1 profile

- schema v2 DTO / typed profile
- Python fixture read
- Rust write→Python read
- key preservation
- atomic create/replace
- lock contention
- model mismatch
- adapter-default namespace
- multiple peer reject

### 9.2 reconnect

- stored Classic key
- active/incoming reconnect
- no-bond/timeout/stale bond
- explicit re-pair
- clean close

### 9.3 Direct

- `DirectProController`
- `ProInputState`
- send/helper/tap transaction
- no periodic input
- close neutral
- Periodic profile reuse

### 9.4 exit criteria

- Python profileをtyped Rustがlossless read
- Rust profileをPythonがread
- same Pro profileをPeriodic/Direct利用
- power-cycle reconnect
- invalid bondを暗黙削除しない
- Direct idle periodicなし
- send failureでsnapshot維持
- update interruptionで旧/新file valid
- key非出力
- alpha.2 criteria

## 10. M7: Joy-Con L/R

### 10.1 型とprotocol

- JoyCon button/state aliases
- left-only/right-only capabilities
- SL/SR
- model-specific device info/SPI/colors
- Periodic/Direct

### 10.2 順序

1. UI tests
2. protocol fixtures
3. fake runtime
4. virtual Bluetooth
5. Joy-Con L hardware
6. Joy-Con R hardware
7. profile reuse
8. Direct

### 10.3 exit criteria

- Joy-Con LにAなし
- Joy-Con RにD-padなし
- side違いstick methodなし
- L: D-pad/left/SL+SR
- R: ABXY/right/SL+SR
- mismatchをopen前にreject
- SPI colors一致
- reporting readiness一致
- left/right別evidence
- JoyConPair未追加

## 11. M8: IMU、diagnostics、probe

### 11.1 IMU

- common ImuFrame/ImuSamples
- standard/quaternion
- same-mode reset
- ACK ordering
- long-run
- model-specific encoder fixture

### 11.2 diagnostics

- stable events
- status projection
- environment
- counters/reason/session
- dynamic unsupported button
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

CLIはControllerKindをparseし入口で`run::<M>()`へ分岐する。

### 11.4 exit criteria

- common IMU compile-pass
- model別fixture
- mode ordering
- hardware trace
- sensitive dataなし
- probe error/exit code/docs
- beta.1 criteria

## 12. M9: portabilityとrelease

### 12.1 Windows

- driver setup
- claim/release/unplug
- package/release/troubleshooting

### 12.2 Linux

- libusb/udev
- driver detach/reattach
- adapter/hardware test
- support label

### 12.3 macOS

初期対象外。USB transportとdriver ownership調査後に追加。

### 12.4 release engineering

- generic API review
- alias rustdoc
- UI tests
- examples compile
- `cargo package`
- license/SBOM
- changelog/semver/security
- hardware matrix/limitations
- source baseline

### 12.5 exit criteria

- 0.1.0 checklist
- supported matrix pairing/reconnect
- clean install
- crateに秘密fixtureなし
- docs command実行可能
- release commit/Bumble revision記録
- backend rollback手順

## 13. explicit local address milestone

M6以降の独立milestone。完了まで`UnsupportedCapability`。

- identity backend
- CSR semantics
- expected-address guard
- partial failure recovery
- power cycle
- namespace compatibility
- duplicate address guidance

## 14. issue優先度

| severity | 条件 | 対応 |
|---|---|---|
| S0 | key漏えい、adapter永続破損、危険な副作用 | 開発停止 |
| S1 | stale input、close hang、key loss、profile corruption、型保証の抜け道 | blocker |
| S2 | reconnect failure、unsupported subcommand、high jitter | release blocker/制限 |
| S3 | diagnostics、ergonomics、build size | roadmap |
| S4 | cosmetic docs/naming | 通常修正 |

## 15. progress evidence

- 対象仕様
- scope/non-goals
- test command/result
- UI test result
- model audit
- fixture provenance
- profile creation ordering
- hardware matrix
- 未検証事項
- cleanup/backend rollback
- Bumble revision/upstream issue

compileするだけで型能力、Bluetooth、protocol milestoneを完了扱いにしない。