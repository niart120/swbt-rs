# アーキテクチャ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係の正本: [type-modeling.md](type-modeling.md)
- 公開契約: [api.md](api.md)

この文書は `swbt-rs` の module 境界、型所有権、runtime 駆動、Bumble 統合、profile persistence、接続状態機械を定義する。

## 1. 設計目標

優先順位:

1. model 固有入力能力と reporting mode の差をコンパイル時に保証する
2. NX HID wire behavior と送信順序を Python 基準断面と一致させる
3. cleanup、neutral、state commit の意味を予測可能にする
4. Bumble 依存を transport 境界へ閉じ込める
5. 実機なしで決定的に検証できる構造にする
6. 利用者に async runtime を要求しない
7. 性能と依存量の削減は correctness 確定後に行う

モデル差を共通 enum と実行時 validation だけへ畳み込まず、public controller、入力状態、worker、protocol session が `M` と `R` を保持する。

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

Bumble、protocol state、report timer、connection state は controller ごとの単一 worker thread が所有する。複数 thread から同じ `Device`、channel manager、`InputState<M>` を mutate しない。

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

共通物理量を model ごとに複製しない。能力差は method の trait bound、wire 差は `M::SPEC` と encoder で表す。

### 3.3 runtime projection

`ControllerKind` と `ReportingKind` は profile DTO、diagnostics、CLI の動的境界で使う。core controller に重複 field として保持せず、`M::KIND` と `R::KIND` から導出する。

## 4. model 宣言の単一正本

次を 1 箇所の宣言から生成または検査する。

- marker type
- `ControllerKind` variant
- profile 文字列
- supported `ButtonKind`
- `Button<M>` associated constants
- `TryFrom<ButtonKind> for Button<M>`
- `HasLeftStick` / `HasRightStick` / `HasDualSticks`
- local name、class of device、device info、SPI seed、SDP policy を持つ `ModelSpec`

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

`ModelSpec` は読み取り専用で、public customization point にしない。

## 5. crate / module 構成

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
  ui/
  fixtures/
  protocol/
  runtime/
  transport_contract/
  bumble_virtual/
  profile_compat/
  hardware/
```

公開 module は `adapter`、`connection`、`controller`、`diagnostics`、`input`、`model`、`profile`、`reporting`、`error` に限定する。`protocol`、`runtime`、`transport` は crate-private とする。

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

- `input` から Bumble、transport、runtime
- `ButtonKind` discriminant から暗黙に wire byte / bit を導くこと
- profile DTO から public controller alias へ依存すること
- controller から `transport::bumble` concrete type へ依存すること
- public signature に Bumble type を出すこと
- Bumble adapter から `Button<M>` や `InputState<M>` を扱うこと
- core runtime で `ControllerKind` を毎操作 `match`して model 型を再現すること

## 7. builder と profile lifecycle

### 7.1 `build()`

`ControllerBuilder<M, R>::build()` は side-effect-free な構築境界とする。

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

存在しない `profile_path` を `build()` へ渡した場合は `ProfileNotFound`。adapter、worker、Bumble device は作らない。

### 7.2 `create_profile()`

新規 profile 作成は builder frontend が所有する複合操作である。worker command ではない。

```text
ControllerBuilder<M, R>::create_profile(options)
  1. profile_path 必須を検査
  2. path が存在しないことを検査
  3. M::KIND と identity を持つ valid empty envelope を create-new
  4. PairingProfile<M> として再読込・検証
  5. Controller<M, R> を構築
  6. worker / adapter を open
  7. pair to normal-input readiness
  8. Ready Controller<M, R> を返す
```

重要な順序:

- envelope persistence は adapter open より先
- explicit local address を実装する場合も、identity 確定前に controller を power on しない
- pairing failure でも empty envelope は残す
- failure 時は内部 controller を cleanup し、partial object は返さない
- path が既に存在すれば `ProfileAlreadyExists`

`Controller<M, R>` に `create_profile()` method は置かない。existing empty profile からの pairing 再試行は `build()` → `open()` → `pair()` を使う。

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

`R::PeriodConfig` は Periodic では validated `Duration`、Direct では unit 型とする。Direct config に無効な `Option<Duration>` を残さない。

## 8. public controller 所有権

`Controller<M, R>` が所有するもの:

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

これらは worker に閉じ込める。

## 9. model-valid input

### 9.1 `ButtonKind` と `Button<M>`

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

`Button<M>` の private constructor は model declaration module に閉じる。

### 9.2 `InputState<M>`

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

public API が不正な `Option<Stick>` の組み合わせを作って送信時に検査する構造は採らない。

### 9.3 共通センサー値

`ImuFrame` は model 非依存。wire encoder が `M::SPEC.protocol` と connection session の IMU mode から bytes を生成する。

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

profile file の create-new は frontend 完了済みであり、worker command に含めない。

reporting 固有 command:

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

stick command は capability-bound frontend で作られ、worker へ渡す時点でも model-valid な private command にする。

command channel は bounded。report tick は command queue に積まず、worker scheduler が所有する。

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

worker の model/reporting generic は compile-time invariant のために維持する。transport I/O は model 非依存の共通実装を共有する。

### 11.1 loop

```text
while lifecycle != Closed:
    1. reporting policy と handshake の最短 deadline を計算
    2. typed command channel を non-blocking drain
    3. Bumble Device を poll
    4. inbound transport event を HIDP / NX protocol へ渡す
    5. 生成 reply を ReportSender<M> へ渡す
    6. due scheduler event を処理
    7. typed state から status projection を更新
    8. command、HCI activity、deadline のいずれかまで待つ
```

busy loop は禁止する。

## 12. reporting policy

### 12.1 Periodic

- state command は validation 完了後に `InputState<M>` を commit
- wire send failure で local state を rollback しない
- readiness 成立後に scheduler 開始
- absolute monotonic deadline
- overrun 時に missed tick を burst 送信しない
- each tick で latest state を 1 件送る

### 12.2 Direct

- connected 必須
- candidate `InputState<M>` を encode
- transport acceptance 後だけ commit
- acceptance 前 failure では previous state を維持
- user-input scheduler なし
- `tap()` 押下から解放まで同じ transaction

## 13. protocol component

### 13.1 `SwitchHidProtocol<M>`

責務:

- `InputState<M>` から `0x30` report 生成
- `0x01` / `0x10` parse
- subcommand effect と `0x21` reply 生成
- model 固有 device info / SPI / colors を `M::SPEC` から取得
- session の report mode、IMU mode、vibration state 管理

扱わないもの:

- USB / HCI / L2CAP
- filesystem
- worker thread
- pairing key
- tracing subscriber

### 13.2 button wire mapping

```rust
fn button_wire_position(
    kind: ControllerKind,
    button: ButtonKind,
) -> Option<ButtonWirePosition>;
```

core generic pathでは`M::KIND`を渡す。`ButtonKind as u8`をreport bit offsetとして使わない。

### 13.3 `ReportSender<M>`

すべての HID interrupt output の唯一の送信点。

- bytes構築とtransport sendを同じserialized operationで行う
- timerとIMU next stateはacceptance後だけ進める
- reply prefixはsend直前のcommitted `InputState<M>`から取る
- `0x40` ACKより新modeの`0x30`を先行させない

## 14. transport port

Bumble 固有 API を隠す model 非依存 interface:

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

- send success は local queue / L2CAP path acceptance
- air delivery / completed packet は成功条件外
- `poll` は timeout 以内に戻る
- `close` は冪等
- transport event は `Button<M>` / `InputState<M>` を含まない

## 15. Bumble 統合

### 15.1 open

1. `AdapterSelector` を Bumble transport spec へ変換
2. `open_split_transport`
3. `ExternalHost::new`
4. `Device::from_config`
5. `ExternalHost::initialize_device`
6. capabilities と local address を取得
7. discoverability / connectability を off に保持
8. worker loop へ移行

### 15.2 `DeviceConfiguration`

- Classic enabled
- accept-any false
- pairing window または stored peer に一致する request だけ accept
- local name、class of device、inquiry response は `M::SPEC` から `TransportConfig` へ値として渡す
- Bumble adapter 自体は generic `M` を知らない

### 15.3 SDP

PSM `0x0001`。model 固有 HID service record と descriptor は model/protocol layerで構築し、Bumble serviceは配信だけを担う。

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

bridge は PSM/CID、MTU、HIDP framing、routing を扱う。NX 意味は `SwitchHidProtocol<M>` が扱う。

### 15.5 pairing / key store

`SwbtProfileKeyStore<M>` は typed profile と Bumble key-store trait の adapter。

- namespace は local controller address
- current peer 最大1件
- `PairingProfile<M>` 全体を原子的に更新
- controller kind mismatch は adapter open 前に拒否
- adapter-default は power-on 後の local address を namespace にする
- key material を log しない
- failed pairing 後も valid envelope を維持

## 16. profile persistence

raw JSON DTO は `ControllerKind` を持つ。validation後は `PairingProfile<M>`。

```text
JSON bytes
  ↓ ProfileDocument { controller_kind: ControllerKind, ... }
schema / shape validation
  ↓ compare with M::KIND
PairingProfile<M>
  ↓ Bumble key conversion
```

create-new / update:

1. parent directory 作成
2. same-directory temporary file
3. complete JSON write
4. flush
5. `sync_all`
6. create-newはno-replace、updateはatomic replace
7. supported OSではparent sync
8. temp cleanup

自動backup、世代管理、復元機能は持たない。並行更新はlockで拒否する。

## 17. dynamic boundary

CLI の `--controller pro` のようにmodelが実行時に決まる場合は入口で一度だけ分岐する。

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

connection sessionごとにreset:

- handshake
- report mode
- player lights readiness
- IMU mode / encoding state
- timer byte
- holdoff deadline
- HID channel IDs
- committed `InputState<M>` を neutralへ戻す

`M` と `R` はcontroller lifetime中に変化しない。

## 19. shutdown ordering

1. new command acceptance停止
2. tap / scheduler停止
3. connectedならtyped neutral encode/send
4. pending interrupt ACLをbounded drain
5. HID interrupt close
6. HID control close
7. Classic ACL disconnect
8. SDP / pairing state破棄
9. `Device` final poll
10. HCI sink flush / close
11. reader termination確認
12. worker response / join

neutral sendまたはdrain失敗でも後続cleanupを続ける。`Drop`では完全順序を保証しない。

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

## 21. dependency / upstream policy

Bumbleは[source-baseline.md](source-baseline.md)のexact revisionに固定する。

Bumble gap:

1. swbt transport contract test
2. minimal Bumble reproduction
3. upstream issue / PR
4. 必要期間だけtemporary fork
5. upstream merge後official revisionへ戻す

Bumble APIの都合で型モデルを崩さない。

## 22. 未検証事項

- `ExternalHost` activityとtyped command channelの低jitter同時待機
- `Device` APIだけでSDP / HID accepted channelを完全駆動できるか
- `bumble_hid::DeviceRuntime`のSwitch HIDP適合
- Classic pairing key JSONのPython/Rust互換
- Windows 11 / CSR8510 A10 / WinUSB実機
- explicit local address recovery
- Linux permission / driver detach
- generic workerのbinary size / compile time

未検証事項を理由にpublic type invariantを弱めない。