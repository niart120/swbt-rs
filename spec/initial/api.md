# 公開 API 仕様

- 状態: **決定**
- 対象: library target `swbt`
- 基準断面: [source-baseline.md](source-baseline.md)

この文書は `swbt-rs` の初期公開 API と、その成功・失敗・状態確定の意味を定義する。構文は実装時の Rust API を拘束する。型名や method 名を変更する場合は、先にこの文書を更新する。

## 1. API 方針

### 1.1 決定事項

- 公開 API は同期・blocking API とする。利用者に特定の async runtime を要求しない
- I/O、pairing、接続待ち、`tap()` の待ち時間には `std::time::Duration` を使う
- Bumble の型、CID、HCI packet、L2CAP manager を公開 API に出さない
- controller shape と送信所有者を concrete type で固定する
- 共通操作は sealed trait で表し、第三者 transport / controller 実装の拡張点にはしない
- controller object は `Clone` しない。初期 API では `Send`、非 `Sync` を目標にする
- 状態を変更する操作は `&mut self` を要求し、利用者側の同時呼び出しを型で抑制する
- `Drop` は best-effort shutdown だけを行う。neutral 送信とエラー確認には明示的な `close()` が必要
- protocol bytes を直接送る API は公開しない
- Rust の `Result` と typed error を使う。Python の例外 class を一対一で再現しない
- Python 版との互換対象は wire bytes、入力意味、profile schema、状態確定条件であり、class inheritance と coroutine 形式ではない

### 1.2 crate root から公開する項目

初期公開対象は次の通り。

```rust
pub use adapter::{list_adapters, AdapterInfo, AdapterSelector};
pub use connection::{
    ConnectOptions, ConnectionPath, ConnectionResult, ConnectionStatus,
    CreateProfileOptions,
};
pub use controller::{
    DirectJoyConL, DirectJoyConR, DirectProController,
    DirectSwitchGamepad, JoyConL, JoyConR,
    PeriodicSwitchGamepad, ProController, SwitchGamepad,
};
pub use diagnostics::{GamepadStatus, LifecycleState};
pub use error::{Error, ErrorKind, Result};
pub use input::{Button, ImuFrame, InputState, Stick};
pub use profile::{
    ControllerColors, ControllerKind, LocalAddress, ProfileIdentity, Rgb24,
};
```

`swbt::transport`、`swbt::protocol`、`swbt::runtime` は公開 module にしない。

## 2. 基本利用例

### 2.1 保存済み profile で接続する

```rust
use std::time::Duration;
use swbt::{Button, ConnectOptions, ProController};

fn main() -> swbt::Result<()> {
    let mut pad = ProController::builder("usb:0")
        .profile_path("profiles/switch-pro.json")
        .build()?;

    pad.open()?;
    pad.connect(ConnectOptions {
        timeout: Duration::from_secs(30),
        allow_pairing: false,
    })?;

    pad.tap(&[Button::A], Duration::from_millis(80))?;
    pad.neutral()?;
    pad.close()?;
    Ok(())
}
```

`build()` は設定と既存 profile を検証するが、adapter を開かない。controller shape が異なる profile は `build()` で拒否する。

`open()` は HCI transport と runtime worker を準備するが、discoverable / connectable 化、pairing、reconnect を始めない。

`connect()` は通常入力 readiness まで待つ。Classic ACL と HID control / interrupt channel が open しただけでは成功にしない。

### 2.2 新しい profile を作成して pairing する

```rust
use std::time::Duration;
use swbt::{CreateProfileOptions, ProfileIdentity, ProController};

fn main() -> swbt::Result<()> {
    let mut pad = ProController::builder("usb:0")
        .profile_path("profiles/switch-pro.json")
        .build()?;

    pad.open()?;
    pad.create_profile(CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout: Duration::from_secs(60),
    })?;

    pad.close()?;
    Ok(())
}
```

`create_profile()` は profile path が存在する場合に上書きしない。profile envelope を原子的に作成した後、pairing を開始する。pairing に失敗しても envelope は残し、同じ path で再試行できる。

明示 local address は API 型として保持するが、実機 gate を通るまで `ProfileIdentity::LocalAddress` を production 対応とは扱わない。対応前に指定された場合は `ErrorKind::UnsupportedCapability` を返し、adapter identity を変更しない。

### 2.3 Direct controller を使う

```rust
use swbt::{Button, DirectProController, InputState, Stick};

fn send_one_state(pad: &mut DirectProController) -> swbt::Result<()> {
    let state = InputState::neutral()
        .with_buttons([Button::L, Button::R])
        .with_left_stick(Stick::up(1.0)?);

    pad.send(state)?;
    Ok(())
}
```

Direct controller は利用者入力用の周期 report loop を持たない。`send()` は input report 1 件が transport の L2CAP 送信経路に受理されるまで待ち、受理された後だけ local state を確定する。

## 3. controller 型

### 3.1 concrete type

| controller shape | 周期送信 | 直接送信 |
|---|---|---|
| Pro Controller | `ProController` | `DirectProController` |
| Joy-Con L | `JoyConL` | `DirectJoyConL` |
| Joy-Con R | `JoyConR` | `DirectJoyConR` |

各 concrete type は内部 generic の type alias ではなく、public newtype とする。これにより rustdoc、将来の method 追加、error context、semver を concrete controller ごとに管理できる。

### 3.2 sealed trait

```rust
pub trait SwitchGamepad: private::Sealed {
    fn open(&mut self) -> Result<()>;
    fn pair(&mut self, timeout: Duration) -> Result<()>;
    fn reconnect(&mut self, timeout: Duration) -> Result<()>;
    fn connect(&mut self, options: ConnectOptions) -> Result<ConnectionPath>;
    fn try_reconnect(&mut self, timeout: Duration) -> Result<ConnectionResult>;
    fn try_connect(&mut self, options: ConnectOptions) -> Result<ConnectionResult>;

    fn press(&mut self, buttons: &[Button]) -> Result<()>;
    fn release(&mut self, buttons: &[Button]) -> Result<()>;
    fn tap(&mut self, buttons: &[Button], duration: Duration) -> Result<()>;
    fn left_stick(&mut self, stick: Stick) -> Result<()>;
    fn right_stick(&mut self, stick: Stick) -> Result<()>;
    fn sticks(&mut self, left: Option<Stick>, right: Option<Stick>) -> Result<()>;
    fn imu(&mut self, frames: &[ImuFrame]) -> Result<()>;
    fn neutral(&mut self) -> Result<()>;

    fn snapshot(&self) -> InputState;
    fn status(&self) -> GamepadStatus;

    fn close(&mut self) -> Result<()>;
    fn close_without_neutral(&mut self) -> Result<()>;
}

pub trait PeriodicSwitchGamepad: SwitchGamepad {
    fn apply(&mut self, state: InputState) -> Result<()>;
    fn report_period(&self) -> Duration;
}

pub trait DirectSwitchGamepad: SwitchGamepad {
    fn send(&mut self, state: InputState) -> Result<()>;
}
```

traits は object safe を維持する。constructor、builder、profile 固有情報は concrete type の associated function とする。

各 concrete type には trait method と同名の inherent forwarding method も実装する。通常利用では trait の `use` を要求せず、generic code だけが sealed trait bound を使う。

### 3.3 builder

各 concrete type は同じ形の builder を返す。

```rust
impl ProController {
    pub fn builder(adapter: impl AsRef<str>) -> ProControllerBuilder;
}
```

共通設定:

```rust
pub struct ControllerBuilder {
    pub adapter: AdapterSelector,
    pub profile_path: Option<PathBuf>,
    pub controller_colors: ControllerColors,
    pub report_period: Option<Duration>, // Periodic だけ
}
```

実 API では field を private にし、chainable method で設定する。

```rust
let pad = ProController::builder("usb:0")
    .profile_path("profiles/pro.json")
    .report_period(Duration::from_millis(8))
    .controller_colors(ControllerColors::default())
    .build()?;
```

制約:

- `adapter` は必須
- `profile_path = None` は永続 bond を持たない一時 controller
- `report_period` は Periodic type だけが公開する
- `report_period` は `1 ms..=1 s`。既定値は `8 ms`
- controller colors は build 時に固定し、接続後の setter を提供しない
- public builder に transport injection を入れない
- fake transport は `cfg(test)` または repository 内 test support だけから注入する

## 4. lifecycle

### 4.1 state

```rust
#[non_exhaustive]
pub enum LifecycleState {
    Configured,
    Open,
    Connecting,
    Ready,
    Closing,
    Closed,
    Failed,
}
```

遷移の正本は [architecture.md](architecture.md) とする。

### 4.2 `open()`

`open()` は次を行う。

- adapter selector を解決する
- HCI transport を open / split する
- runtime worker を起動する
- Bumble device を初期化する
- diagnostics と command channel を利用可能にする

次は行わない。

- discoverable / connectable 化
- pairing
- stored bond での reconnect
- HID channel 待機
- periodic input report の開始

`Open` で再度呼ばれた `open()` は成功する。`Ready` での再呼び出しも no-op とする。`Closing` 中は `Busy`、recover 不能な `Failed` では原因を保持した error を返す。

`close()` 完了後の再 `open()` は許可する。新 session は neutral state、timer byte 0、host request state 未設定から開始する。

### 4.3 `close()`

`close()` は接続中なら trailing neutral report を 1 件送る。その後、保留中の interrupt channel 送信を規定範囲で drain し、HID channel、Classic ACL、HCI transport、worker thread を順に停止する。

`close_without_neutral()` は trailing input report を追加しない。それ以外の cleanup 順序は同じである。

両 method は冪等にする。cleanup の一部が失敗した場合も残りの cleanup を試し、最初の error と追加 error の要約を返す。

`Drop` は期限なしの wait、pairing、neutral report を行わない。command channel の終了と worker join の短い best-effort だけを行う。neutral fail-safe を必要とする利用者は必ず `close()` を呼ぶ。

## 5. 接続 API

### 5.1 option と結果

```rust
pub struct ConnectOptions {
    pub timeout: Duration,
    pub allow_pairing: bool,
}

#[non_exhaustive]
pub enum ConnectionPath {
    Reconnected,
    Paired,
}

#[non_exhaustive]
pub enum ConnectionStatus {
    Connected,
    NoBond,
    TimedOut,
    Failed,
}

pub struct ConnectionResult {
    pub status: ConnectionStatus,
    pub path: Option<ConnectionPath>,
    pub message: Option<String>,
}
```

`message` は診断用であり、program logic は `status` と typed `ErrorKind` を使う。key material や raw packet を含めない。

### 5.2 `pair()`

`pair(timeout)` は初回 pairing の明示入口である。

- Classic discoverable / connectable を有効にする
- Switch 側からの incoming connection を待つ
- SSP / link key 処理を行う
- HID control / interrupt channel を受け入れる
- NX protocol readiness まで進める
- successful bond が得られた場合は profile に保存する

profile path がない場合は session 内だけで key を保持する。timeout は `ErrorKind::ConnectionTimeout`、pairing reject や protocol failure はそれぞれの typed error とする。

### 5.3 `reconnect()`

`reconnect(timeout)` は保存済み bond だけを使う。pairing fallback を行わない。

- current peer が 0 件なら `ErrorKind::NoBond`
- current peer が 2 件以上なら `ErrorKind::InvalidKeyStore`
- active reconnect と incoming bonded reconnect のどちらを先に使うかは transport policy に閉じ込める
- retry loop と advertising recovery を暗黙に開始しない

### 5.4 `connect()`

`connect(options)` は次の順序を固定する。

1. usable bond があれば `reconnect()`
2. bond がなく `allow_pairing = true` なら `pair()`
3. bond がなく `allow_pairing = false` なら `NoBond`
4. reconnect が通信失敗した場合、`allow_pairing = true` でも自動的に bond を削除して fresh pairing へ移らない

正常終了は `ConnectionPath` を返す。`try_connect()` は接続不成立を `ConnectionResult` に畳み込み、profile schema 不正、複数 peer、adapter open failure、内部不変条件違反は `Err` のまま返す。

### 5.5 readiness

接続 API は次をすべて満たした後だけ成功する。

- Classic ACL が有効
- HID control / interrupt channel が両方 open
- 最初の bootstrap neutral report を送信済み
- supported `0x03` set report mode を受信し、reply が transport に受理済み
- 0 以外の `0x30` player lights を受信し、reply が transport に受理済み
- 同じ connection session で上記を満たす
- readiness を成立させた handshake task / state を停止・回収済み

Periodic controller は最後の automatic input holdoff 終了後に report scheduler を開始し、最初の通常 tick を予約できた時点で `Ready` にする。Direct controller は protocol ready で `Ready` とし、確認用 periodic report を送らない。

## 6. 入力操作

### 6.1 共通規則

- `apply()` / `send()` は完全な `InputState` で現在状態を置換する。差分適用ではない
- `press()` / `release()` / stick / IMU helper は現在状態から候補状態を作る
- profile が対応しない button / stick は report 送信と state commit の前に拒否する
- new connection session は neutral baseline から始める
- empty button slice の `press()` / `release()` / `tap()` は `InvalidInput`
- `tap()` duration は `0..=24 h`。0 は押下 report の直後に release report を送る
- raw protocol duration field は作らない

### 6.2 Periodic semantics

Periodic controller の `apply()` と semantic input helper は、runtime worker が local state を確定した時点で成功する。接続していなくても state 更新を許可する。未接続中の state は wire へ送らない。

接続中は次の periodic deadline で latest state を送る。過去 tick を burst 送信せず、overrun 後は現在時刻以降の最初の deadline へ進める。

`tap()` は例外であり、action semantics を持つ。接続済みを要求し、押下状態と解放状態が最低 1 回ずつ transport に受理されるよう、通常 scheduler と同じ送信所有者へ明示 command を入れる。解放対象は引数の button だけである。

### 6.3 Direct semantics

Direct controller の `send()` と semantic input helper は接続済みを要求する。

1. 最後に受理された state から候補 state を作る
2. profile validation を行う
3. input report 1 件を構築する
4. transport が L2CAP 送信経路へ受理するまで待つ
5. 受理後だけ候補 state を commit する

step 4 より前に失敗した場合、`snapshot()` は以前の state を返す。controller flow-control completion、air delivery、Switch UI への反映は成功条件に含めない。

`tap()` は押下から解放まで同じ worker transaction とする。解放送信が失敗した場合、押下 state を最後に受理された state として維持する。

### 6.4 stick helper

```rust
pub struct Stick {
    x: u16,
    y: u16,
}

impl Stick {
    pub const MIN: u16 = 0;
    pub const CENTER: u16 = 2048;
    pub const MAX: u16 = 4095;

    pub const fn center() -> Self;
    pub fn raw(x: u16, y: u16) -> Result<Self>;
    pub fn normalized(x: f32, y: f32) -> Result<Self>;
    pub fn tilt(x: f32, y: f32) -> Result<Self>;
    pub fn up(amount: f32) -> Result<Self>;
    pub fn down(amount: f32) -> Result<Self>;
    pub fn left(amount: f32) -> Result<Self>;
    pub fn right(amount: f32) -> Result<Self>;

    pub const fn x(&self) -> u16;
    pub const fn y(&self) -> u16;
}
```

`normalized` / `tilt` は各軸 `-1.0..=1.0`、direction helper は `0.0..=1.0` を受ける。NaN と infinity を拒否する。変換は Python 基準断面と同じ、負側は center-to-min、正側は center-to-max の個別丸めを使う。

### 6.5 IMU

```rust
pub struct ImuFrame {
    accel: [i16; 3],
    gyro: [i16; 3],
}
```

提供する constructor / converter:

- `ImuFrame::neutral()`
- `ImuFrame::raw(accel, gyro)`
- `ImuFrame::gyro([i16; 3])`
- `ImuFrame::accel([i16; 3])`
- `with_gyro` / `with_accel`
- `from_gyro_rate_rad_s([f32; 3])`
- `to_gyro_rate_rad_s()`
- `from_accel_g([f32; 3])`
- `to_accel_g()`

尺度は Python 基準断面と同じく、gyro `0.070 dps/raw`、accelerometer `1/4096 G/raw` とする。非有限値と i16 範囲外を clamp せず拒否する。

`imu(&[ImuFrame])` は長さ 1 または 3 だけを受理する。1 frame は 3 slot へ複製し、3 frame は順に使う。

### 6.6 button

```rust
#[non_exhaustive]
pub enum Button {
    A, B, X, Y,
    L, R, ZL, ZR,
    Plus, Minus, Home, Capture,
    LeftStick, RightStick,
    SL, SR,
    DpadUp, DpadDown, DpadLeft, DpadRight,
}
```

enum discriminant を wire bit として使わない。wire mapping は protocol module と golden test で固定する。

Joy-Con L は ABXY、right stick 等を拒否する。Joy-Con R は D-pad、left stick 等を拒否する。`SL` / `SR` は単体 Joy-Con 用であり、Pro Controller では拒否する。

## 7. `InputState`

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputState {
    // private fields
}

impl InputState {
    pub fn neutral() -> Self;
    pub fn with_buttons(self, buttons: impl IntoIterator<Item = Button>) -> Self;
    pub fn with_left_stick(self, stick: Stick) -> Self;
    pub fn with_right_stick(self, stick: Stick) -> Self;
    pub fn with_sticks(self, left: Option<Stick>, right: Option<Stick>) -> Self;
    pub fn with_imu(self, frames: [ImuFrame; 3]) -> Self;

    pub fn buttons(&self) -> impl Iterator<Item = Button> + '_;
    pub fn left_stick(&self) -> Stick;
    pub fn right_stick(&self) -> Stick;
    pub fn imu_frames(&self) -> &[ImuFrame; 3];
}
```

`InputState` は controller shape 非依存の値である。shape validation は concrete controller へ適用するときに行う。

button 集合の内部表現は private bitset とし、iteration order は enum の論理順に固定する。serialization を初期 API に含めない。

## 8. controller identity と profile

### 8.1 controller kind

```rust
pub enum ControllerKind {
    Pro,
    JoyConL,
    JoyConR,
}
```

Periodic / Direct は送信方式であり、profile の controller kind には含めない。同じ shape の profile は両方式で共有できることを目標とする。

### 8.2 color

`ControllerColors` は body、buttons、left grip、right grip の 24-bit RGB を持つ。既定値は次の通り。

| field | value |
|---|---:|
| body | `0x323232` |
| buttons | `0xFFFFFF` |
| left grip | `0x00B2FF` |
| right grip | `0xFF3B30` |

各 field は `Rgb24` newtype で範囲を保証する。SPI `0x6050` から body / buttons / left grip / right grip の順で big-endian 3 bytes を返す。接続後の色変更 API は提供しない。

### 8.3 profile identity

```rust
pub enum ProfileIdentity {
    AdapterDefault,
    LocalAddress(LocalAddress),
}
```

`LocalAddress` は `XX:XX:XX:XX:XX:XX` を parse し、次を検査する。

- 6 octet
- individual address
- locally administered
- reserved inquiry LAP `0x9E8B00..=0x9E8B3F` ではない

profile schema と永続化は [migration-strategy.md](migration-strategy.md) で定義する。

## 9. adapter discovery

```rust
pub fn list_adapters() -> Result<Vec<AdapterInfo>>;
```

adapter を claim / open せず、USB descriptor と interface class から Bluetooth HCI candidate を列挙する。

```rust
pub struct AdapterInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub bus_number: Option<u8>,
    pub device_address: Option<u8>,
    pub port_numbers: Vec<u8>,
    pub is_bluetooth_hci: bool,
}
```

`name` は controller builder に渡せる primary selector とする。初期 USB naming は `usb:0`、`usb:VVVV:PPPP`、必要に応じて serial / duplicate suffix を使う。Bumble の内部 USB selector 型は返さない。

descriptor 読み取りだけが OS 権限で失敗した場合、その optional field を `None` にして列挙を継続する。enumeration 自体を開始できない場合は `AdapterDiscovery` error とする。

## 10. status と diagnostics

```rust
pub struct GamepadStatus {
    pub lifecycle: LifecycleState,
    pub connected: bool,
    pub controller_kind: ControllerKind,
    pub report_mode: Option<u8>,
    pub input_reports_accepted: u64,
    pub replies_accepted: u64,
    pub last_subcommand: Option<u8>,
    pub last_disconnect_reason: Option<u8>,
    pub worker_failure: Option<String>,
}
```

`status()` は安価な snapshot を返す。worker thread の I/O を待たない。key、raw pairing payload、full packet bytes を含めない。

構造化 trace は `tracing` facade を使い、target を `swbt` とする。Python の `DiagnosticsConfig` に相当する library-owned writer object は初期 stable API にしない。JSON Lines が必要な利用者は subscriber を設定する。hardware probe CLI は既定 subscriber を提供する。

最低限の event 名:

- `transport_open_start` / `transport_open_complete`
- `controller_initialized`
- `pairing_start` / `pairing_complete`
- `connected`
- `l2cap_channel_open`
- `output_report_rx`
- `subcommand_rx`
- `report_tx_accepted`
- `neutral_tx_accepted`
- `disconnected`
- `transport_close_complete`
- `error`

`report_tx_accepted` は transport 受理を意味し、controller completion や air delivery を意味しない。

## 11. error

```rust
pub type Result<T> = std::result::Result<T, Error>;

pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

#[non_exhaustive]
pub enum ErrorKind {
    AdapterDiscovery,
    TransportOpen,
    TransportClosed,
    InvalidProfile,
    ProfileControllerMismatch,
    InvalidKeyStore,
    NoBond,
    ConnectionTimeout,
    ConnectionFailed,
    Protocol,
    InvalidInput,
    UnsupportedInput,
    UnsupportedCapability,
    Busy,
    WorkerFailed,
    Shutdown,
    Internal,
}
```

`Error::kind()` を stable classification とする。message 文言は semver 契約にしない。Bumble error は source chain に保持するが public enum variant として露出しない。

秘密情報保護:

- link key、LTK、IRK、CSRK を `Debug` / `Display` / trace に出さない
- pairing profile 全文を error に含めない
- raw packet logging は CLI の明示 opt-in とし、初期 library API では無効

## 12. thread と cancellation

- concrete controller は `Send` を目標とするが、同時操作は `&mut self` で禁止する
- worker command channel は bounded とし、無制限 queue を作らない
- public blocking method は timeout または worker termination を観測できる
- `tap()` の sleep は public thread ではなく worker scheduler が管理し、close command で中断できる
- close が始まった後、新しい input command は `TransportClosed` または `Busy`
- worker panic は join 時に `WorkerFailed` へ変換し、process abort を前提にしない

## 13. 初期公開 API に含めないもの

- raw HID control / interrupt packet API
- raw HCI / L2CAP API
- custom transport 実装用 public trait
- custom controller profile 実装用 public trait
- `JoyConPair`
- controller manager / 複数 controller orchestration
- 高水準 rumble
- amiibo / NFC
- IR camera
- macro scheduler
- daemon IPC
- C ABI / FFI
- stable serde format for `InputState`
- automatic infinite reconnect
- implicit bond deletionとfresh pairing fallback

これらを実装する際は、既存型へ未使用 field を先に追加せず、別の仕様作業単位で failure semantics と ownership を定義する。
