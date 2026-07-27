# アーキテクチャ

- 状態: **決定**
- 基準断面: [source-baseline.md](source-baseline.md)
- 公開契約: [api.md](api.md)

この文書は `swbt-rs` の module 境界、所有権、runtime 駆動、Bumble 統合、接続状態機械を定義する。

## 1. 設計目標

`swbt-rs` は daemon ではなく library とする。利用者は process 内の controller object を操作し、library 内部の worker が Bluetooth と周期送信を駆動する。

優先順位は次の通り。

1. NX HID wire behavior と送信順序の再現
2. cleanup、neutral、state commit の予測可能性
3. Bumble 依存の局所化
4. 実機なしで決定的に検証できる構造
5. Rust 利用者が async runtime を選ばずに使える公開 API
6. 性能と依存量の削減

初期段階で最小 build size を優先して Bumble の責務を再実装しない。

## 2. 全体構成

```text
利用者 thread
  │
  │  swbt public API
  ▼
Concrete Controller
  ├─ builder / profile validation
  ├─ immutable configuration
  ├─ status snapshot
  └─ bounded command client
             │ command / response
             ▼
      Controller Worker (1 thread / controller)
        ├─ lifecycle state machine
        ├─ InputStateStore
        ├─ ReportScheduler
        ├─ ReportSender
        ├─ ProtocolHandshake
        ├─ SwitchHidProtocol
        ├─ ProfileStore
        └─ TransportPort
               │
               ▼
        BumbleTransportAdapter
          ├─ bumble_transport::ExternalHost
          ├─ bumble_host::Device
          ├─ Classic pairing / key store bridge
          ├─ SDP channel service
          ├─ HID control/interrupt channel bridge
          └─ USB HCI transport
               │
               ▼
        USB Bluetooth adapter
               │
               ▼
        Nintendo Switch
```

**決定:** Bumble、protocol state、report timer、connection state を同じ worker thread が所有する。複数 thread から同じ `Device` や channel manager を mutate しない。

## 3. crate / module 構成

初期の想定構成:

```text
src/
  lib.rs

  adapter/
    mod.rs
    discovery.rs
    selector.rs

  controller/
    mod.rs
    builder.rs
    common.rs
    periodic.rs
    direct.rs
    pro_controller.rs
    joycon_l.rs
    joycon_r.rs

  input/
    mod.rs
    button.rs
    imu.rs
    state.rs
    stick.rs

  protocol/
    mod.rs
    input_report.rs
    output_report.rs
    session.rs
    handshake.rs
    subcommand.rs
    spi.rs
    rumble.rs
    profiles/
      mod.rs
      common.rs
      pro_controller.rs
      joycon_l.rs
      joycon_r.rs

  profile/
    mod.rs
    document.rs
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
    swbt-probe.rs       # roadmap 後半で追加

tests/
  common/
  fixtures/
  protocol/
  runtime/
  transport_contract/
  bumble_virtual/
  profile_compat/
  hardware/
```

公開 module は `adapter`、`controller`、`diagnostics`、`input`、`profile`、`error` の再 export に限定する。`protocol`、`runtime`、`transport` は crate-private とする。

## 4. 依存方向

```text
controller
  → input
  → profile
  → runtime
  → error

runtime
  → input
  → protocol
  → profile
  → transport::port
  → error

protocol
  → input
  → protocol::profiles
  → error

profile
  → protocol::profiles::ControllerKind
  → error

transport::bumble
  → transport::port
  → profile::key_store interface
  → bumble-* crates
  → error

input
  → std only
```

禁止する依存:

- `input` から `protocol` / `transport` / Bumble
- `protocol` から `runtime` / `transport` / Bumble
- `profile::document` から Bumble の stored key type
- `controller` から `transport::bumble` の concrete type
- public signature から Bumble type
- test fake から production Bumble adapter
- Bumble adapter から concrete public controller type

`profile::key_store` は swbt schema と neutral key representation を所有し、Bumble conversion は `transport::bumble::key_store` に置く。

## 5. controller object と worker 所有権

### 5.1 public controller

public controller が所有するもの:

- validated immutable `ControllerConfig`
- worker command sender
- worker join handle
- lock-freeまたは短時間 read lock の status snapshot
- lifecycle client-side guard
- controller kind marker
- reporting mode marker

public controller が所有しないもの:

- open USB handle
- `ExternalHost`
- `bumble_host::Device`
- L2CAP CID
- pairing key の runtime copy
- timer byte
- report scheduler
- Switch session state

これらは worker に閉じ込める。

### 5.2 worker command

概念上の command:

```rust
enum WorkerCommand {
    Open,
    CreateProfile(CreateProfileRequest),
    Pair { timeout: Duration },
    Reconnect { timeout: Duration },
    Connect(ConnectOptions),

    Apply(InputState),        // Periodic
    Send(InputState),         // Direct
    Press(Vec<Button>),
    Release(Vec<Button>),
    Tap { buttons: Vec<Button>, duration: Duration },
    SetSticks { left: Option<Stick>, right: Option<Stick> },
    SetImu([ImuFrame; 3]),
    Neutral,

    Close { send_neutral: bool },
}
```

各 command は one-shot response sender を持つ。response を失った場合も worker は command の副作用を途中で巻き戻さない。呼び出し thread が timeout した後の扱いを曖昧にしないため、入力 command 自体には library-defined operation timeout を設け、public call の待機 timeout と一致させる。

channel は bounded とする。初期値は 64 command。queue full は無期限 block ではなく `Busy` とする。report tick は command queue に積まず、worker 内 scheduler が持つ。

### 5.3 単一 worker を採用する理由

- Bumble core が同期・明示 poll model である
- `Device`、Classic channel、pairing session の mutable ownership を一箇所にできる
- `0x21` reply と `0x30` input の実送信順を一つの sender で固定できる
- Direct transaction の「受理後 commit」を lock ではなく event loop 順序で実現できる
- periodic timer と HCI activity の待機を同じ deadline calculation で扱える
- close / disconnect / timeout と入力 command の競合を再現可能にできる

async executor を内部に導入して同じ mutable state を `Arc<Mutex<_>>` へ分散させない。

## 6. worker loop

概念的な loop:

```text
while lifecycle != Closed:
    1. due report / tap release / handshake retry の最短 deadline を計算
    2. command channel を非 blocking drain
    3. Bumble Device を poll
    4. inbound transport event を protocol へ渡す
    5. 生成された reply を ReportSender へ渡す
    6. due scheduler event を処理
    7. status snapshot を更新
    8. command、HCI activity、deadline のいずれかまで待つ
```

`ExternalHost` の reader thread が HCI source を block read し、worker は receiver と timer を待つ。command channel と HCI activity を同時 select できない場合は、次のどちらかを実装 spike で比較する。

- `ExternalHost` の activity receiver を worker select に統合する小さな upstream / local adapter
- 最大 1 ms の bounded poll interval と deadline-aware wait

busy loop は禁止する。8 ms periodic mode で idle CPU 使用率と jitter を測定し、選択を roadmap M2 で確定する。

## 7. runtime component

### 7.1 `InputStateStore`

責務:

- current committed state の保持
- neutral baseline の生成
- helper operation から候補 state を生成
- controller profile validation の適用
- `snapshot()` 用 copy の公開
- disconnect / new session 時の neutral reset

Periodic:

- `Apply` / helper command の validation 完了後に commit
- wire send 失敗で local state を rollback しない

Direct:

- candidate report が transport に受理された後だけ commit
- 受理前 failure では current state を維持

state store は worker 専有であり、内部 mutex を持たない。public snapshot は worker が更新する read-only mirror を使う。

### 7.2 `ReportSender`

すべての HID interrupt output の唯一の送信点とする。

所有する状態:

- 8-bit timer byte
- connection session id
- IMU encoding state
- reply 後 automatic input holdoff deadline
- accepted report counters
- trace metadata

入力:

- periodic / direct / handshake の `0x30`
- subcommand reply の `0x21`
- close 時 neutral

規則:

- report bytes の構築と transport send を同じ worker iteration で行う
- timer byte は transport acceptance 後に進める
- IMU next state も acceptance 後に commit する
- reply の state prefix は send 直前の committed state から取る
- reply が要求する session state transition は ACK と同じ serialized operation 内で行う
- `0x40` IMU mode ACK より新形式 `0x30` が先に出ない
- raw send failure は lifecycle / connection policy へ error event を返す

### 7.3 `ReportScheduler`

Periodic controller だけが通常入力用 scheduler を持つ。

- 既定 period 8 ms
- monotonic clock
- deadline = previous deadline + period
- send が遅延した場合、過去 deadline 分を burst 送信しない
- current time 以上になるまで deadline を period 単位で進める
- each tick で latest state snapshot を 1 件だけ送る
- reply holdoff 中も deadline progression は維持し、解除後に過去 tick を追送しない
- disconnect 中は通常 tick を送らない
- reconnect で新 session が ready になった時点から新しい epoch を開始する

clock と wait は trait で注入可能にするが crate-private とする。

### 7.4 `ProtocolHandshake`

connection session ごとに一つ作る。

state:

```text
WaitingForFirstSubcommand
  ├─ 1 秒ごとに bootstrap neutral
  └─ first valid output report
       ↓
WaitingForReportModeAndLights
  ├─ supported 0x03 30 reply accepted
  ├─ non-zero 0x30 player lights reply accepted
  └─ 条件成立
       ↓
ProtocolReady
```

規則:

- first valid subcommand 受信後は 1 秒 bootstrap retry を停止
- supported report mode reply 後は requested mode の neutral `0x30` を handshake owner として送る
- unsupported report mode は ready 条件にしない
- readiness flag は session id と結び、旧 connection から再利用しない
- reply acceptance failure、disconnect、timeout で `Failed`
- ready 条件成立後は handshake を明示停止してから normal scheduler を開始する

### 7.5 `SwitchHidProtocol`

純粋または明示 state 入出力の protocol core とする。

責務:

- `0x30` input report の生成
- `0x01` / `0x10` output report の parse
- rumble raw state の保持に必要な event 生成
- subcommand dispatch
- `0x21` reply payload と ACK の生成
- virtual SPI read
- controller kind 固有 device info / colors
- report mode、IMU mode、vibration enabled 等の connection session state
- IMU 36-byte block encode

扱わないもの:

- HCI / ACL / L2CAP
- USB
- wall clock sleep
- filesystem
- public controller lifecycle
- pairing key
- tracing subscriber

protocol API は byte slice と明示 state を受け、同じ入力から同じ output を返す部分を最大化する。

## 8. transport port

### 8.1 internal interface

Bumble 固有 API を隠す crate-private interface:

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

実装時は borrow と event batching に合わせて signature を調整してよいが、次の意味は変えない。

- `send_*` success = local transport queue / L2CAP send path acceptance
- air delivery / controller completed packets は success 条件外
- `poll` は timeout 以内に戻る
- `close` は冪等
- peer key material は event の Debug に出さない
- protocol layer は channel CID / PSM を知らない

### 8.2 event

```rust
enum TransportEvent {
    ClassicConnectionRequested { peer: PeerIdentity },
    ClassicConnected { peer: PeerIdentity, handle: u16 },
    PairingComplete { peer: PeerIdentity, bond: Option<BondRecord> },
    ChannelOpened { kind: HidChannelKind },
    HidControlReceived(Vec<u8>),
    HidInterruptReceived(Vec<u8>),
    ChannelClosed { kind: HidChannelKind, reason: Option<u8> },
    Disconnected { reason: Option<u8> },
    TransportEnded,
}
```

handle、CID、PSM は Bumble adapter 内部で必要だが、runtime 上位へ出す event は意味型へ正規化する。

## 9. Bumble 統合

### 9.1 transport open

想定手順:

1. `AdapterSelector` を Bumble transport spec へ変換
2. `bumble_transport::open_split_transport` で source / sink を分離
3. `bumble_transport::ExternalHost::new`
4. `bumble_host::Device::from_config`
5. `ExternalHost::initialize_device`
6. controller capabilities と local address を取得
7. Classic discoverability / connectability の初期状態を off に保持
8. worker event loop へ移行

adapter open failure は context を加えて `TransportOpen` へ変換する。

`bumble-transport` 内の reader thread に加えて swbt worker thread を持つ。thread 数は docs と diagnostics に記録し、unbounded thread spawn をしない。

### 9.2 `DeviceConfiguration`

初期候補:

- `classic_enabled = true`
- `classic_accept_any = false`
- `classic_smp_enabled` は Bumble pairing path に合わせて設定
- local name、class of device、inquiry response は controller profile から構築
- LE は NX HID 初期経路では無効または未使用

worker は pending Classic connection request を取り出し、pairing window 中の peer、または保存済み bond と一致する peer だけに `accept_classic` を発行する。それ以外は reject / timeout させる。これにより immutable な `DeviceConfiguration` に動的 policy を埋め込まず、pairing window 終了後の新規 peer を受け入れない。

### 9.3 SDP

Classic SDP PSM は `0x0001`。

`BumbleSdpService` は次を所有する。

- controller kind 固有 HID service record
- HID descriptor
- service handle
- incoming SDP channel ごとの server state
- continuation state
- accepted channel cleanup

service record bytes / attributes は Python 基準断面から fixture 化し、Rust 側の semantic builder と byte-level snapshot の両方を test する。

SDP request handling は HID report scheduler を block し続けない。1 worker iteration あたりの request 処理数に上限を設け、再度 loop へ戻る。

### 9.4 HID control / interrupt

PSM:

- HID control: `0x0011`
- HID interrupt: `0x0013`

Bumble 基準断面の `bumble-hid` には `Message` codec、`DeviceRuntime`、`L2capTransport` がある。ただし external `Device` 経路との ownership が一致しないため、初期 adapter は次の形にする。

```text
bumble_host::Device Classic SDU API
  ↕ Vec<u8>
SwbtHidChannelBridge
  ↕ bumble_hid::Message
bumble_hid::DeviceRuntime
  ↕ data payload / control event
TransportEvent
```

`SwbtHidChannelBridge` の責務:

- accepted PSM と CID の対応付け
- MTU 検査
- complete SDU と HIDP message の encode / decode
- control / interrupt の routing
- `DeviceRuntime` の protocol request 応答
- protocol data payload を上位 event にする
- channel close cleanup

HID report payload に NX 意味を付けるのは `SwitchHidProtocol` であり、bridge は HIDP framing までとする。

上流 `L2capTransport` を直接再利用できる API が追加された場合も、transport contract test を維持したまま bridge 内だけを置換する。

### 9.5 pairing と key store

Classic pairing は `bumble_transport::ClassicPairingSession` と `bumble::keys::KeyStore` contract を使う。

`SwbtProfileKeyStore` は Bumble trait adapter として実装し、次を守る。

- namespace は local controller address
- current namespace の peer は最大 1 件
- update は profile schema v2 envelope 全体を原子的に置換
- unknown fields / unsupported schema を黙って消さない
- link key material を log しない
- controller kind mismatch は adapter open 前に拒否
- adapter-default profile では power on 後に得た local address を namespace とする
- failed pairing 後も valid empty envelope を維持する

Python `PairingKeys.to_dict()` と Bumble Rust `StoredPairingKeys` の field / hex 表現を compatibility test で固定する。変換不能な key field を drop しない。

### 9.6 local address

`adapter-default` は initial production path とする。controller が報告する public address を変更しない。

`LocalAddress` path は次の gate を通るまで無効。

- target adapter の vendor-specific command を識別
- current address read-back
- temporary / persistent write の意味を識別
- failure 後の recovery 手順
- expected-address guard
- power cycle 後の状態確認
- Python profile と namespace の互換確認
- CSR8510 A10 / WinUSB 実機 test

generic HCI API に address write が見つからないことを理由に、vendor command を protocol core へ埋め込まない。adapter identity backend は Bumble adapter 内の別 module とする。

## 10. profile persistence

profile document は `serde` 用 private DTO と、validated domain type を分ける。

```text
JSON bytes
  ↓ serde DTO
schema / shape validation
  ↓
PairingProfile domain
  ↓
Bumble key conversion
```

write 手順:

1. parent directory を作成
2. same directory に permission-restricted temporary file を作る
3. complete JSON を書く
4. file flush
5. `sync_all`
6. create-new は no-replace operation、update は atomic replace
7. 対応 OS では parent directory を sync
8. temporary file を cleanup

同じ profile を複数 process が更新する競合を検出するため、initial implementation は lock file または platform file lock を使う。lock を取得できない場合は `Busy` とし、last-writer-wins にしない。

詳細は [migration-strategy.md](migration-strategy.md) を参照する。

## 11. lifecycle state machine

```text
Configured
   │ open
   ▼
Open
   │ pair / reconnect / connect
   ▼
Connecting
   ├─ failure ───────────────► Open または Failed
   │
   └─ protocol readiness
          ▼
        Ready
          ├─ disconnect ─────► Open
          ├─ connect command ─► Busy
          └─ close
                ▼
             Closing
                ▼
              Closed
                │ open
                └────────────► Open
```

### 11.1 `Failed`

`Failed` は worker panic、transport terminal failure、internal invariant violation 等、同じ worker を安全に再利用できない状態である。

connection timeout、peer reject、no bond は必ずしも `Failed` にしない。cleanup が完了すれば `Open` へ戻す。

`Failed` でも `close()` は実行できる。再 `open()` は旧 worker を完全に join し、新 worker を作れる場合だけ許可する。

### 11.2 connection session

各 Classic ACL に monotonically increasing session id を割り当てる。

session-scoped:

- protocol handshake
- report mode
- player lights readiness
- IMU mode / encoding state
- vibration enable state
- timer byte
- holdoff deadline
- HID channel CIDs
- disconnect reason

controller-scoped:

- immutable profile kind / colors
- profile path
- adapter selector
- committed input state。ただし new session 開始時に neutral へ reset
- cumulative diagnostics counters

旧 session の event は id mismatch で破棄し、status counter に stale event として記録する。

## 12. shutdown ordering

正常 close:

1. new command acceptance を停止
2. tap / scheduler を停止
3. connected なら trailing neutral を interrupt channel へ送る
4. pending interrupt ACL を bounded timeout で drain
5. HID interrupt channel を閉じる
6. HID control channel を閉じる
7. Classic ACL を切断
8. SDP / pairing state を破棄
9. `Device` を final poll
10. HCI transport sink を flush / close
11. reader termination を確認
12. worker response を返し join

neutral send または drain が失敗した場合も、resource leak を避けるため後続 cleanup を続ける。ただし「neutral が送れた」とは記録しない。

process termination、panic、`Drop` ではこの完全順序を保証しない。API 文書の明示 close 契約を優先する。

## 13. timing

使用する clock は monotonic。wall clock は trace timestamp と profile metadata にだけ使う。

default:

| 項目 | 値 |
|---|---:|
| periodic report | 8 ms |
| bootstrap retry | 1 s |
| public connect timeout | caller supplied |
| HCI command timeout | transport constant、初期候補 5 s |
| channel drain timeout | 初期候補 1 s |
| worker idle poll upper bound | 実装 spike で 1 ms 以下を評価 |

初期候補値は test と実機計測後に決定値へ更新する。public API の default timeout を magic number として複数箇所に複製しない。

## 14. diagnostics と security

- `tracing` event は worker から emit
- packet payload は既定で記録しない
- key material を含む型は custom `Debug` で redaction
- adapter serial number は通常 status に含めず、explicit environment report だけに含める
- profile file permission は owner read/write を目標にする
- malformed inbound packet は panic せず protocol error event
- size field、SPI address、continuation token、MTU を boundary check
- `unsafe` は USB / upstream dependency 内に閉じ、swbt crate 自身では初期段階で使わない
- `forbid(unsafe_code)` を crate root に設定する。必要になった場合は仕様変更と局所 safety comment を要求する

## 15. dependency policy

Bumble は [source-baseline.md](source-baseline.md) の exact revision に固定する。

`bumble-transport` の transitive dependency が大きいことは既知である。初期 architecture は次の順で対応する。

1. correctness を確認
2. build time / binary size / license inventory を測定
3. upstream の feature gate 可否を確認
4.必要なら USB + ExternalHost を小さい crate へ切り出す upstream PR を作る
5. fork を常態化させない

local patch が必要な場合:

- `patches/bumble-rs/<issue-id>/` のようなコピーは作らない
- fork commit を exact rev で一時固定
- upstream issue / PR を関連付ける
- patch removal condition を roadmap に記載
- protocol / transport contract test を先に追加

## 16. 将来拡張に対する境界

初期に public extension trait を出さないが、内部境界は次を想定する。

- 別 Bluetooth backend: `TransportPort`
- 別 scheduler: `Clock` / `ReportScheduler`
- 別 controller profile: `protocol::profiles`
- daemon / CLI: public controller API の薄い consumer
- async wrapper: blocking controller を `spawn_blocking` で包む別 crate

future async API のために現在の public method を generic future にしない。必要なら `swbt-async` wrapper を別 package として設計する。

## 17. 未検証事項

次は architecture 上の仮説であり、roadmap gate を通るまで保証しない。

- `ExternalHost` activity と swbt command を低 jitter で同時待機できるか
- `Device` API だけで SDP / HID accepted channel を完全に駆動できるか
- `bumble-hid::DeviceRuntime` が Switch の HIDP control sequence を追加実装なしで処理できるか
- Classic pairing link key JSON が Python Bumble と byte-level互換か
- Windows 11 / CSR8510 A10 / WinUSB で bumble-rs USB transport が同じ接続順を通るか
- Switch 2 firmware 22.1.0 以外の実機
- Linux libusb permission / driver detach
- macOS USB backend
- explicit local address recovery

未検証事項は「予定」ではなく test item として [testing.md](testing.md) と [roadmap.md](roadmap.md) に結び付ける。
