# アーキテクチャ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係の正本: [type-modeling.md](type-modeling.md)
- 公開契約: [api.md](api.md)

この文書は `swbt-rs` のmodule境界、型所有権、runtime駆動、Bumble統合、profile persistence、接続状態機械を定義する。

## 1. 設計目標

優先順位:

1. model固有入力能力とreporting modeの差を型で表す
2. NX HID wire behaviorと送信順序をPython基準断面と一致させる
3. cleanup、neutral、state commitの意味を予測可能にする
4. Bumble依存をtransport境界へ閉じ込める
5. 実機なしで決定的に検証できる構造にする
6. 利用者にasync runtimeを要求しない
7. 性能と依存量の削減はcorrectness確定後に行う

モデル差を共通enumと実行時validationだけへ畳み込まず、public controller、入力状態、worker、protocol sessionが`M`と`R`を保持する。

型として表現されたmethod absenceやgeneric不一致を再確認するためのcompiler UI testはarchitectureに含めない。テストはdomain mapping、動的境界、runtime、wire behaviorを対象とする。

## 2. 全体構成

```text
利用者 thread
  │
  │ Controller<M, R>
  │ Button<M> / InputState<M>
  ▼
ControllerBuilder<M, R>
  ├─ build(): existing typed profile or ephemeral controller
  ├─ create_profile(): envelope → open → pair
  └─ immutable configuration
             │ typed command / response
             ▼
      ControllerWorker<M, R>
        ├─ lifecycle state machine
        ├─ InputStateStore<M>
        ├─ ReportingPolicy<R>
        ├─ ReportSender<M>
        ├─ ProtocolHandshake<M>
        ├─ SwitchHidProtocol<M>
        ├─ ProfileStore<M>
        └─ TransportPort
               │ model-independent events / bytes
               ▼
        BumbleTransportAdapter
          ├─ bumble_transport::ExternalHost
          ├─ bumble_host::Device
          ├─ Classic pairing / key-store bridge
          ├─ SDP channel service
          ├─ HID control/interrupt bridge
          └─ USB HCI transport
               │
               ▼
        USB Bluetooth adapter
               │
               ▼
        Nintendo Switch
```

Bumble、protocol state、report timer、connection stateはcontrollerごとの単一worker threadが所有する。複数threadから同じ`Device`、channel manager、`InputState<M>`をmutateしない。

## 3. 型軸と値軸

### 3.1 型として保持するもの

- model: `model::Pro` / `model::JoyConL` / `model::JoyConR`
- reporting: `reporting::Periodic` / `reporting::Direct`
- model-valid button: `Button<M>`
- model-valid state: `InputState<M>`
- typed profile: `PairingProfile<M>`
- worker / protocol: `ControllerWorker<M, R>` / `SwitchHidProtocol<M>`

### 3.2 共通値として保持するもの

- `Stick`
- `ImuFrame` / `ImuSamples`
- `ControllerColors` / `Rgb24`
- Bluetooth address / peer identity
- HCI / L2CAP / HIDP bytes

共通物理量をmodelごとに複製しない。能力差はmethodのtrait bound、wire差は`M::SPEC`とencoderで表す。

### 3.3 runtime projection

`ControllerKind`と`ReportingKind`はprofile DTO、diagnostics、CLIの動的境界で使う。core controllerに重複fieldとして保持せず、`M::KIND`と`R::KIND`から導出する。

## 4. model宣言の単一正本

次を1箇所の宣言から生成するか、機械的に整合させる。

- marker type
- `ControllerKind` variant
- profile文字列
- supported `ButtonKind`
- `Button<M>` associated constants
- `TryFrom<ButtonKind> for Button<M>`
- `HasLeftStick` / `HasRightStick` / `HasDualSticks`
- local name、class of device、device info、SPI seed、SDP policyを持つ`ModelSpec`

```rust
pub struct ModelSpec {
    pub kind: ControllerKind,
    pub profile_name: &'static str,
    pub local_name: &'static str,
    pub class_of_device: u32,
    pub supported_buttons: ButtonKindSet,
    pub has_left_stick: bool,
    pub has_right_stick: bool,
    pub protocol: &'static ProtocolProfile,
}
```

`ModelSpec`は読み取り専用で、public customization pointにしない。

## 5. crate / module構成

```text
src/
  lib.rs

  adapter/
    mod.rs
    discovery.rs
    selector.rs

  model/
    mod.rs
    declaration.rs
    capability.rs
    spec.rs

  reporting/
    mod.rs
    periodic.rs
    direct.rs

  controller/
    mod.rs
    controller.rs
    builder.rs
    aliases.rs

  input/
    mod.rs
    button.rs
    button_kind.rs
    button_set.rs
    stick.rs
    imu.rs
    state.rs

  connection/
    mod.rs
    options.rs
    result.rs

  diagnostics/
    mod.rs
    event.rs
    status.rs

  protocol/
    mod.rs
    input_report.rs
    imu_report.rs
    output_report.rs
    session.rs
    handshake.rs
    subcommand.rs
    spi.rs
    rumble.rs
    profile.rs

  profile/
    mod.rs
    document.rs
    typed.rs
    key_store.rs
    local_address.rs
    store.rs

  runtime/
    mod.rs
    command.rs
    worker.rs
    lifecycle.rs
    state_store.rs
    report_sender.rs
    report_scheduler.rs
    reporting_policy.rs
    status.rs
    clock.rs

  transport/
    mod.rs
    port.rs
    event.rs
    bumble/
      mod.rs
      open.rs
      host.rs
      device.rs
      pairing.rs
      sdp.rs
      hid.rs
      key_store.rs

  error.rs

  bin/
    swbt-probe.rs

tests/
  common/
  fixtures/
  model_mapping/
  protocol/
  runtime/
  transport_contract/
  bumble_virtual/
  profile_compat/
  hardware/
```

compiler UI test専用の`tests/ui/`は作らない。

公開moduleは`adapter`、`connection`、`controller`、`diagnostics`、`input`、`model`、`profile`、`reporting`、`error`に限定する。`protocol`、`runtime`、`transport`はcrate-privateとする。

## 6. 依存方向

```text
controller<M, R>
  → model
  → reporting
  → input<M>
  → profile<M>
  → runtime<M, R>
  → error

runtime<M, R>
  → model
  → reporting policy
  → input<M>
  → protocol<M>
  → profile<M>
  → transport::port
  → error

protocol<M>
  → model::ModelSpec
  → input<M>
  → error

profile<M>
  → model
  → profile DTO
  → error

transport::bumble
  → transport::port
  → neutral key-store interface
  → bumble-* crates
  → error
```

禁止する依存:

- `input`からBumble、transport、runtime
- `ButtonKind` discriminantから暗黙にwire byte / bitを導くこと
- profile DTOからpublic controller aliasへ依存すること
- controllerから`transport::bumble` concrete typeへ依存すること
- public signatureにBumble typeを出すこと
- Bumble adapterから`Button<M>`や`InputState<M>`を扱うこと
- core runtimeで`ControllerKind`を毎操作`match`してmodel型を再現すること

## 7. builderとprofile lifecycle

### 7.1 `build()`

`ControllerBuilder<M, R>::build()`はside-effect-freeな構築境界とする。

```text
builder validation
  ├─ profile_path = None
  │    → ephemeral Controller<M, R>
  └─ profile_path = Some(existing path)
       → parse raw ProfileDocument
       → validate schema
       → compare controller_kind with M::KIND
       → PairingProfile<M>
       → configured Controller<M, R>
```

存在しない`profile_path`を`build()`へ渡した場合は`ProfileNotFound`。adapter、worker、Bumble deviceは作らない。

### 7.2 `create_profile()`

新規profile作成はbuilder frontendが所有する複合操作である。worker commandではない。

```text
ControllerBuilder<M, R>::create_profile(options)
  1. profile_path必須を検査
  2. pathが存在しないことを検査
  3. M::KINDとidentityを持つvalid empty PairingProfile<M>を生成してcreate-new
  4. 同じPairingProfile<M>をruntime configへ移譲
  5. Controller<M, R>を構築
  6. worker / adapterをopen
  7. pair to normal-input readiness
  8. Ready Controller<M, R>を返す
```

重要な順序:

- envelope persistenceはadapter openより先
- 同じcreate呼出しでは保存直後に再読込せず、保存bytesとruntime configを同じ型付き値から作る
- 後続のbuild/open/reconnectは保存済みprofileを再読込し、persisted identityを使う
- explicit local addressを実装する場合も、identity確定前にcontrollerをpower onしない
- pairing failureでもempty envelopeは残す
- failure時は内部controllerをcleanupし、partial objectは返さない
- pathが既に存在すれば`ProfileAlreadyExists`

`Controller<M, R>`に`create_profile()` methodは置かない。existing empty profileからのpairing再試行は`build()`→`open()`→`pair()`を使う。

### 7.3 config

```rust
struct ControllerConfig<M: ControllerModel, R: ReportingMode> {
    adapter: AdapterSelector,
    profile: ProfileConfig<M>,
    colors: ControllerColors,
    report_period: R::PeriodConfig,
}

enum ProfileConfig<M: ControllerModel> {
    Ephemeral,
    Persistent {
        path: PathBuf,
        profile: PairingProfile<M>,
    },
}
```

`R::PeriodConfig`はPeriodicではvalidated `Duration`、Directではunit型とする。Direct configに無効な`Option<Duration>`を残さない。

## 8. public controller所有権

`Controller<M, R>`が所有するもの:

- validated `ControllerConfig<M, R>`
- typed worker command sender
- worker join handle
- read-only status snapshot
- lifecycle client-side guard

所有しないもの:

- USB handle
- `ExternalHost`
- `bumble_host::Device`
- L2CAP CID
- pairing key runtime copy
- timer byte
- report scheduler
- Switch session state

これらはworkerに閉じ込める。

## 9. model-valid input

### 9.1 `ButtonKind`と`Button<M>`

```text
static Rust code
  → ProButton::A / JoyConRButton::A
  → Button<M>
  → ButtonKind
  → explicit wire mapping table
```

動的境界:

```text
CLI / config string
  → ButtonKind
  → TryFrom<ButtonKind> for Button<M>
  → supported / UnsupportedInput
```

`Button<M>`のprivate constructorはmodel declaration moduleに閉じる。

### 9.2 wire mapping

`ButtonKind`は論理IDであり、NX wire位置を表さない。

```rust
pub(crate) struct ButtonWirePosition {
    pub byte_index: usize,
    pub mask: u8,
}

pub(crate) fn button_wire_position(
    kind: ControllerKind,
    button: ButtonKind,
) -> Option<ButtonWirePosition>;
```

mappingは`M::KIND`と`ButtonKind`から明示的に引く。Joy-Con L/Rの`SL` / `SR`を同一位置と仮定しない。

`ButtonKind as u8`をreport offset、shift count、bit maskへ直接使わない。

### 9.3 `InputState<M>`

内部候補:

```rust
struct InputState<M: ControllerModel> {
    buttons: ButtonSet<M>,
    sticks: M::StickState,
    imu: [ImuFrame; 3],
}
```

```text
Pro      → DualStickState { left, right }
JoyConL  → LeftStickState { left }
JoyConR  → RightStickState { right }
```

public APIが不正な`Option<Stick>`の組み合わせを作って送信時に検査する構造は採らない。

### 9.4 共通センサー値

`ImuFrame`はmodel非依存。wire encoderが`M::SPEC.protocol`とconnection sessionのIMU modeからbytesを生成する。

## 10. typed worker command

```rust
pub(crate) enum CommonCommand<M: ControllerModel> {
    Open,
    Pair { timeout: Duration },
    Reconnect { timeout: Duration },
    Connect(ConnectOptions),
    Press(Vec<Button<M>>),
    Release(Vec<Button<M>>),
    Tap {
        buttons: Vec<Button<M>>,
        duration: Duration,
    },
    SetImu(ImuSamples),
    Neutral,
    Close { send_neutral: bool },
}
```

profile fileのcreate-newはfrontend完了済みであり、worker commandに含めない。

reporting固有command:

```rust
pub(crate) enum PeriodicCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Apply(InputState<M>),
}

pub(crate) enum DirectCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Send(InputState<M>),
}
```

stick commandはcapability-bound frontendで作られ、workerへ渡す時点でもmodel-validなprivate commandにする。

command channelはbounded。report tickはcommand queueに積まず、worker schedulerが所有する。

## 11. `ControllerWorker<M, R>`

```rust
struct ControllerWorker<M: ControllerModel, R: ReportingMode> {
    lifecycle: LifecycleStateMachine,
    state: InputStateStore<M>,
    reporting: R::RuntimeState,
    sender: ReportSender<M>,
    protocol: SwitchHidProtocol<M>,
    profile: ProfileStore<M>,
    transport: Box<dyn TransportPort>,
}
```

`R::RuntimeState`:

```text
Periodic → ReportScheduler + period + holdoff integration
Direct   → no user-input scheduler + acceptance transaction state
```

workerのmodel/reporting genericはpublic invariantを内部へ保つために維持する。transport I/Oはmodel非依存の共通実装を共有する。

### 11.1 loop

```text
while lifecycle != Closed:
    1. reporting policyとhandshakeの最短deadlineを計算
    2. typed command channelをnon-blocking drain
    3. Bumble Deviceをpoll
    4. inbound transport eventをHIDP / NX protocolへ渡す
    5. 生成replyをReportSender<M>へ渡す
    6. due scheduler eventを処理
    7. typed stateからstatus projectionを更新
    8. command、HCI activity、deadlineのいずれかまで待つ
```

busy loopは禁止する。

## 12. reporting policy

### 12.1 Periodic

- state commandはvalidation完了後に`InputState<M>`をcommit
- wire send failureでlocal stateをrollbackしない
- readiness成立後にscheduler開始
- absolute monotonic deadline
- overrun時にmissed tickをburst送信しない
- each tickでlatest stateを1件送る

### 12.2 Direct

- connected必須
- candidate `InputState<M>`をencode
- transport acceptance後だけcommit
- acceptance前failureではprevious stateを維持
- user-input schedulerなし
- `tap()`押下から解放まで同じtransaction

## 13. protocol component

### 13.1 `SwitchHidProtocol<M>`

責務:

- `InputState<M>`から`0x30` report生成
- `0x01` / `0x10` parse
- subcommand effectと`0x21` reply生成
- model固有device info / SPI / colorsを`M::SPEC`から取得
- sessionのreport mode、IMU mode、vibration state管理

扱わないもの:

- USB / HCI / L2CAP
- filesystem
- worker thread
- pairing key
- tracing subscriber

### 13.2 `ReportSender<M>`

すべてのHID interrupt outputの唯一の送信点。

- bytes構築とtransport sendを同じserialized operationで行う
- timerとIMU next stateはacceptance後だけ進める
- reply prefixはsend直前のcommitted `InputState<M>`から取る
- `0x40` ACKより新modeの`0x30`を先行させない

## 14. transport port

Bumble固有APIを隠すmodel非依存interface:

```rust
trait TransportPort {
    fn open(&mut self, config: &TransportConfig) -> Result<TransportCapabilities>;
    fn start_pairing(&mut self, policy: PairingPolicy) -> Result<()>;
    fn start_reconnect(&mut self, peer: PeerIdentity) -> Result<()>;
    fn poll(&mut self, timeout: Duration) -> Result<Vec<TransportEvent>>;
    fn send_hid_interrupt(&mut self, payload: &[u8]) -> Result<SendAcceptance>;
    fn send_hid_control(&mut self, payload: &[u8]) -> Result<SendAcceptance>;
    fn disconnect(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
```

意味:

- send successはlocal queue / L2CAP path acceptance
- air delivery / completed packetは成功条件外
- `poll`はtimeout以内に戻る
- `close`は冪等
- transport eventは`Button<M>` / `InputState<M>`を含まない

## 15. Bumble統合

### 15.1 open

1. `AdapterSelector`をBumble transport specへ変換
2. `open_split_transport`
3. `ExternalHost::new`
4. `Device::from_config`
5. `ExternalHost::initialize_device`
6. capabilitiesとlocal addressを取得
7. discoverability / connectabilityをoffに保持
8. worker loopへ移行

### 15.2 `DeviceConfiguration`

- Classic enabled
- accept-any false
- pairing windowまたはstored peerに一致するrequestだけaccept
- local name、class of device、inquiry responseは`M::SPEC`から`TransportConfig`へ値として渡す
- Bumble adapter自体はgeneric `M`を知らない

### 15.3 SDP

Classic SDP PSMは`0x0001`。

model固有HID service recordとdescriptorはmodel/protocol layerで構築し、Bumble serviceは配信、continuation、channel lifecycleを担う。

SDP request処理がreport schedulerを長時間blockしないよう、1 worker iterationあたりの処理数に上限を設ける。

### 15.4 HIDP bridge

```text
bumble_host::Device Classic SDU API
  ↕ Vec<u8>
SwbtHidChannelBridge
  ↕ bumble_hid::Message
bumble_hid::DeviceRuntime
  ↕ payload / control event
TransportEvent
```

bridgeはPSM/CID、MTU、HIDP framing、routingを扱う。NX意味は`SwitchHidProtocol<M>`が扱う。

### 15.5 pairing / key store

`SwbtProfileKeyStore<M>`はtyped profileとBumble key-store traitのadapter。

- namespaceはlocal controller address
- current peer最大1件
- `PairingProfile<M>`全体を原子的に更新
- controller kind mismatchはadapter open前に拒否
- adapter-defaultはpower-on後のlocal addressをnamespaceにする
- key materialをlogしない
- failed pairing後もvalid envelopeを維持

Python `PairingKeys.to_dict()`とBumble Rustのstored型のfield / hex表現をcompatibility fixtureで固定する。

## 16. profile persistence

raw JSON DTOは`ControllerKind`を持つ。validation後は`PairingProfile<M>`。

```text
JSON bytes
  ↓ ProfileDocument { controller_kind: ControllerKind, ... }
schema / shape validation
  ↓ compare with M::KIND
PairingProfile<M>
  ↓ Bumble key conversion
```

create-new / update:

1. parent directory作成
2. same-directory temporary file
3. complete JSON write
4. flush
5. `sync_all`
6. create-newはno-replace、updateはatomic replace
7. supported OSではparent sync
8. temp cleanup

自動backup、世代管理、復元機能は持たない。並行更新はlockで拒否する。

## 17. dynamic boundary

CLIの`--controller pro`のようにmodelが実行時に決まる場合は入口で一度だけ分岐する。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後はtyped pathを維持する。`AnyController`、model非依存`Button`、model非依存`InputState`は初期APIに追加しない。

## 18. lifecycle state machine

```text
Configured
   │ open
   ▼
Open
   │ pair / reconnect / connect
   ▼
Connecting
   ├─ recoverable failure ─────► Open
   ├─ terminal failure ────────► Failed
   └─ protocol readiness
          ▼
        Ready
          ├─ disconnect ───────► Open
          ├─ connect command ──► Busy
          └─ close
                ▼
             Closing
                ▼
              Closed
                │ open
                └──────────────► Open
```

### 18.1 connection session

各Classic ACLにmonotonically increasing session IDを割り当てる。

sessionごとにreset:

- handshake
- report mode
- player lights readiness
- IMU mode / encoding state
- timer byte
- holdoff deadline
- HID channel IDs
- committed `InputState<M>`をneutralへ戻す

`M`と`R`はcontroller lifetime中に変化しない。旧sessionのeventはID mismatchで破棄し、diagnosticsへ記録する。

## 19. shutdown ordering

1. new command acceptance停止
2. tap / scheduler停止
3. connectedならtyped neutral encode/send
4. 未送信の interrupt ACL がホスト側の待ち行列を離れてコントローラのフロー制御枠へ入るまで期限付きで排出
5. HID interrupt close
6. HID control close
7. Classic ACL disconnect
8. SDP / pairing state破棄
9. `Device` final poll
10. HCI sink flush / close
11. reader termination確認
12. worker response / join

コントローラ内で送信中の ACL に対する完了クレジットが残っていても、ホスト側の待ち行列が空なら排出完了とする。neutral send または drain 失敗でも後続 cleanup を続ける。`Drop` では完全順序を保証しない。

## 20. timing

| 項目 | 値 |
|---|---:|
| periodic report | 8 ms |
| bootstrap retry | 1 s |
| public connect timeout | caller supplied |
| HCI command timeout | initial candidate 5 s |
| channel drain timeout | initial candidate 1 s |
| worker idle poll upper bound | implementation spikeで評価 |

clockはmonotonic。wall clockはdiagnosticsと運用記録だけに使う。

## 21. diagnosticsとsecurity

- `tracing` eventはkey materialを含めない
- raw packet traceはCLIの明示opt-in
- statusはworker I/Oを待たないsnapshot
- `controller_kind`と`reporting_kind`は`M` / `R`から導出
- `report_tx_accepted`はtransport受理を意味し、Switch UI反映を意味しない
- panic/errorにprofile全文を含めない

## 22. dependency / upstream policy

Bumbleは[source-baseline.md](source-baseline.md)のexact revisionに固定する。

Bumble gap:

1. swbt transport contract test
2. minimal Bumble reproduction
3. upstream issue / PR
4. 必要期間だけtemporary fork
5. upstream merge後official revisionへ戻す

Bumble APIの都合で型モデルを崩さない。

## 23. 未検証事項

- `ExternalHost` activityとtyped command channelの低jitter同時待機
- `Device` APIだけでSDP / HID accepted channelを完全駆動できるか
- `bumble_hid::DeviceRuntime`のSwitch HIDP適合
- Classic pairing key JSONのPython/Rust互換
- Windows 11 / CSR8510 A10 / WinUSB実機
- explicit local address recovery
- Linux permission / driver detach
- generic workerのbinary size / compile time
- type alias経由のrustdocの見え方

未検証事項を理由にpublic type invariantを弱めない。
