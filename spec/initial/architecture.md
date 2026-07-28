# アーキテクチャ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係の正本: [type-modeling.md](type-modeling.md)
- 公開契約: [api.md](api.md)

この文書は `swbt-rs` の module 境界、型所有権、runtime 駆動、Bumble 統合、profile persistence、接続状態機械を定義する。

## 1. 設計目標

優先順位は次の通り。

1. model 固有の入力能力と reporting mode の差をコンパイル時に保証する
2. NX HID wire behavior と送信順序を Python 基準断面と一致させる
3. cleanup、neutral、state commit の意味を予測可能にする
4. Bumble 依存を transport 境界へ閉じ込める
5. 実機なしで決定的に検証できる構造にする
6. 利用者に async runtime を要求しない
7. 性能と依存量の削減は correctness 確定後に行う

モデル差を共通 enum と実行時 validation だけへ畳み込まず、公開 controller、入力状態、worker、protocol session が `M` と `R` を保持する。

## 2. 全体構成

```text
利用者 thread
  │
  │ Controller<M, R>
  │ Button<M> / InputState<M>
  ▼
ControllerBuilder<M, R>
  ├─ typed profile validation: PairingProfile<M>
  ├─ immutable configuration
  └─ worker creation
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
          ├─ Classic pairing / key store bridge
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

- controller model: `model::Pro` / `model::JoyConL` / `model::JoyConR`
- reporting mode: `reporting::Periodic` / `reporting::Direct`
- model-valid button: `Button<M>`
- model-valid complete state: `InputState<M>`
- model 固有 profile: `PairingProfile<M>`
- model 固有 worker / protocol state: `ControllerWorker<M, R>` / `SwitchHidProtocol<M>`

### 3.2 共通値として保持するもの

- `Stick`
- `ImuFrame`
- `ImuSamples`
- `ControllerColors`
- `Rgb24`
- Bluetooth address / peer identity
- HCI / L2CAP / HIDP bytes

共通物理量を model ごとに複製しない。能力差は method の trait bound、wire 差は `M::SPEC` と encoder で表す。

### 3.3 runtime projection

`ControllerKind` と `ReportingKind` は profile DTO、diagnostics、CLI などの動的境界で使う値である。core controller に重複 field として保持せず、常に `M::KIND` と `R::KIND` から導出する。

## 4. model 宣言の単一正本

model の次の情報は 1 箇所の宣言から生成または検査する。

- marker type
- `ControllerKind` variant
- profile 文字列
- 使用可能 `ButtonKind`
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

`ModelSpec` は runtime 読み取り専用であり、public customization point にしない。model 宣言と別の手書き対応表を protocol、profile、docs に複製しない。

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
- profile DTO から public controller concrete alias へ依存すること
- controller から `transport::bumble` concrete type へ依存すること
- public signature に Bumble type を出すこと
- test fake から production Bumble adapter へ依存すること
- Bumble adapter から `Button<M>` や `InputState<M>` を扱うこと
- core runtime で `ControllerKind` を毎操作 `match`して model 型を再現すること

## 7. public controller と builder

### 7.1 `Controller<M, R>`

public controller が所有するもの:

- validated immutable `ControllerConfig<M, R>`
- typed worker command sender
- worker join handle
- read-only status snapshot
- lifecycle client-side guard
- `PhantomData<fn() -> (M, R)>` または同等の型保持

所有しないもの:

- open USB handle
- `ExternalHost`
- `bumble_host::Device`
- L2CAP CID
- pairing key の runtime copy
- timer byte
- report scheduler
- Switch session state

### 7.2 `ControllerBuilder<M, R>`

builder は共通型 1 個とし、model / reporting の値 field を持たない。

```rust
struct ControllerConfig<M: ControllerModel, R: ReportingMode> {
    adapter: AdapterSelector,
    profile_path: Option<PathBuf>,
    colors: ControllerColors,
    report_period: R::PeriodConfig,
    _marker: PhantomData<fn() -> M>,
}
```

`R::PeriodConfig` は Periodic では validated `Duration`、Direct では unit 型とするか、同等に不正状態を表現不能にする。Direct config に `Option<Duration>` を残して build 時に reject する設計は採らない。

path が存在する場合は `PairingProfile<M>::load()` で model を確定する。path が存在しない場合は create-new target として保持し、通常 reconnect は `NoBond`、`create_profile()` は新規作成へ進む。

## 8. model-valid input

### 8.1 `ButtonKind` と `Button<M>`

`ButtonKind` は closed な論理 ID であり、explicit discriminant を持つ。`Button<M>` は対象 model で使用可能であることの型証明である。

公開経路:

```text
static Rust code
  → ProButton::A / JoyConRButton::A
  → Button<M>
  → ButtonKind
  → explicit wire mapping table
```

動的経路:

```text
CLI / config string
  → ButtonKind
  → TryFrom<ButtonKind> for Button<M>
  → supported / UnsupportedInput
```

`Button<M>` の private constructor を crate 内でも無秩序に使わない。生成 code と dynamic conversion を model declaration module に閉じる。

### 8.2 `InputState<M>`

`InputState<M>` は public API から model-valid な状態だけを構築できる。

内部候補:

```rust
struct InputState<M: ControllerModel> {
    buttons: ButtonSet<M>,
    sticks: M::StickState,
    imu: [ImuFrame; 3],
}
```

`M::StickState` は private associated type とし、次のような model 固有 layout を許可する。

```text
Pro      → DualStickState { left, right }
JoyConL  → LeftStickState { left }
JoyConR  → RightStickState { right }
```

公開 API が不正な `Option<Stick>` の組み合わせを作ってから送信時に検査する構造は採らない。

### 8.3 共通センサー値

`ImuFrame` は model 非依存である。`InputState<M>` に同じ `[ImuFrame; 3]` を保持し、wire encoder が `M::SPEC.protocol` と connection session の IMU mode から bytes を生成する。

## 9. typed worker command

概念上の command は model 型を保持する。

```rust
pub(crate) enum CommonCommand<M: ControllerModel> {
    Open,
    CreateProfile(CreateProfileRequest),
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

stick command は capability-bound frontend で作られ、worker へ渡す時点でも model-valid な private command にする。

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

`R` に応じた associated command type または同等の private abstraction で worker を構築する。内部 enum に `Apply` と `Send` の両方を残し、到達不能 branch を runtime error にする構造は避ける。

command channel は bounded とする。report tick は command queue に積まず、worker 内 scheduler が所有する。

## 10. `ControllerWorker<M, R>`

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

worker の model/reporting generic は compile-time invariant のために維持する。Bumble transport を 6 通りに複製するためではない。transport field と I/O loop は model 非依存の共通実装を共有する。

### 10.1 loop

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

busy loop は禁止する。command channel と `ExternalHost` activity を同時 select できない場合は、activity receiver の統合か bounded deadline-aware poll を計測して選ぶ。

## 11. reporting policy

### 11.1 Periodic

- state command は validation 完了後に `InputState<M>` を commit
- wire send 失敗で local state を rollback しない
- 通常入力 readiness 成立後に scheduler を開始
- absolute monotonic deadline を使う
- overrun 時に missed tick を burst 送信しない
- 各 tick で latest typed state を 1 件だけ送る

### 11.2 Direct

- 接続済みを要求
- candidate `InputState<M>` を report に encode
- transport acceptance 後だけ state を commit
- acceptance 前 failure では previous state を維持
- user-input scheduler を持たない
- `tap()` の押下から解放まで同じ transaction を保持

## 12. protocol component

### 12.1 `SwitchHidProtocol<M>`

責務:

- `InputState<M>` から `0x30` report を生成
- `0x01` / `0x10` output report を parse
- subcommand effect と `0x21` reply を生成
- model 固有 device info / SPI / controller colors を `M::SPEC` から取得
- connection session の report mode、IMU mode、vibration state を管理

扱わないもの:

- USB / HCI / L2CAP
- filesystem
- worker thread
- pairing key
- tracing subscriber

### 12.2 button wire mapping

wire mapping は明示表とする。

```rust
fn button_wire_position(
    kind: ControllerKind,
    button: ButtonKind,
) -> Option<ButtonWirePosition>;
```

core generic path では `M::KIND` を渡す。`ButtonKind as u8` をそのまま report bit offset として使わない。model 宣言で使用可能とされた全 button に mapping があることを table audit test で保証する。

### 12.3 `ReportSender<M>`

すべての HID interrupt output の唯一の送信点。

所有する状態:

- timer byte
- connection session id
- IMU encoding state
- automatic input holdoff deadline
- accepted counters

規則:

- bytes 構築と transport send を同じ serialized operation で行う
- timer と IMU next state は acceptance 後だけ進める
- reply prefix は send 直前の committed `InputState<M>` から取る
- `0x40` ACK より新 mode の `0x30` を先行させない

## 13. transport port

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

- send success は local transport queue / L2CAP path acceptance
- air delivery / controller completed packet は成功条件外
- `poll` は timeout 以内に戻る
- `close` は冪等
- protocol layer は CID / PSM を知らない
- transport event は `Button<M>` や `InputState<M>` を含まない

## 14. Bumble 統合

### 14.1 open

1. `AdapterSelector` を Bumble transport spec へ変換
2. `open_split_transport`
3. `ExternalHost::new`
4. `Device::from_config`
5. `ExternalHost::initialize_device`
6. controller capabilities と local address を取得
7. Classic discoverability / connectability を off に保持
8. worker loop へ移行

### 14.2 `DeviceConfiguration`

- Classic enabled
- accept-any は false
- pairing window または stored peer に一致した incoming request だけ accept
- local name、class of device、inquiry response は `M::SPEC` から builder 時に `TransportConfig` へ値として渡す
- Bumble adapter 自体は generic `M` を知らない

### 14.3 SDP

PSM `0x0001`。model 固有 HID service record と descriptor は protocol/model layerで bytes または neutral descriptor に構築し、Bumble service はそれを配信する。

### 14.4 HIDP bridge

```text
bumble_host::Device Classic SDU API
  ↕ Vec<u8>
SwbtHidChannelBridge
  ↕ bumble_hid::Message
bumble_hid::DeviceRuntime
  ↕ payload / control event
TransportEvent
```

bridge は PSM/CID、MTU、HIDP framing、control/interrupt routing を扱う。NX 意味は `SwitchHidProtocol<M>` が扱う。

### 14.5 pairing / key store

`SwbtProfileKeyStore<M>` は typed profile と Bumble key-store trait の adapter とする。

- namespace は local controller address
- current peer は最大 1 件
- `PairingProfile<M>` 全体を原子的に更新
- controller kind mismatch は adapter open 前に拒否
- adapter-default は power-on 後の local address を namespace にする
- key material を log しない
- failed pairing 後も valid empty envelope を維持

## 15. profile persistence

raw JSON DTO は `ControllerKind` を持つ。validation 後は `PairingProfile<M>` へ変換する。

```text
JSON bytes
  ↓ ProfileDocument { controller_kind: ControllerKind, ... }
schema / shape validation
  ↓ M::KIND comparison
PairingProfile<M>
  ↓ Bumble key conversion
```

write 手順:

1. parent directory を作成
2. same-directory temporary file を作る
3. complete JSON を書く
4. flush
5. `sync_all`
6. create-new は no-replace、update は atomic replace
7. 対応 OS では parent directory を sync
8. temporary file を cleanup

自動 backup、世代管理、復元機能は持たない。同一 profile の並行更新は lock で拒否する。

## 16. dynamic boundary

CLI の `--controller pro` のように model が実行時に決まる場合は、入口で 1 度だけ分岐する。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後は typed path を維持する。`AnyController`、model 非依存 `Button`、model 非依存 `InputState` は初期 core API に追加しない。

## 17. lifecycle state machine

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

connection session ごとに reset するもの:

- handshake
- report mode
- player lights readiness
- IMU mode / encoding state
- timer byte
- holdoff deadline
- HID channel IDs
- committed `InputState<M>` を neutral baseline へ戻す

model type `M` と reporting type `R` は controller lifetime 中に変化しない。

## 18. shutdown ordering

1. new command acceptance を停止
2. tap / scheduler を停止
3. connected なら typed neutral state を encode して送る
4. pending interrupt ACL を bounded timeout で drain
5. HID interrupt channel を閉じる
6. HID control channel を閉じる
7. Classic ACL を切断
8. SDP / pairing state を破棄
9. `Device` を final poll
10. HCI transport sink を flush / close
11. reader termination を確認
12. worker response を返して join

neutral send または drain が失敗しても後続 cleanup を続ける。`Drop` ではこの完全順序を保証しない。

## 19. timing

| 項目 | 値 |
|---|---:|
| periodic report | 8 ms |
| bootstrap retry | 1 s |
| public connect timeout | caller supplied |
| HCI command timeout | initial candidate 5 s |
| channel drain timeout | initial candidate 1 s |
| worker idle poll upper bound | implementation spike で評価 |

clock は monotonic。wall clock は diagnostics と運用記録だけに使う。

## 20. dependency / upstream policy

Bumble は [source-baseline.md](source-baseline.md) の exact revision に固定する。同一 repository の Bumble crate を複数 revision で混在させない。

Bumble gap が見つかった場合:

1. swbt transport contract test を作る
2. minimal reproduction を Bumble test として作る
3. upstream issue / PR
4. 必要な期間だけ temporary fork revision
5. upstream merge 後に official revision へ戻す

型モデルを Bumble API の都合で崩さない。Bumble は bytes と transport event の境界に留める。

## 21. 未検証事項

- `ExternalHost` activity と typed command channel の低 jitter 同時待機
- `Device` API だけで SDP / HID accepted channel を完全に駆動できるか
- `bumble_hid::DeviceRuntime` の Switch HIDP control sequence 適合
- Classic pairing key JSON の Python/Rust互換
- Windows 11 / CSR8510 A10 / WinUSB 実機
- explicit local address recovery
- Linux permission / driver detach
- model generic worker の binary size と compile time

未検証事項を理由に public type invariant を弱めない。必要なら transport adapter または build 構成を修正する。