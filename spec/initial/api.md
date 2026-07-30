# 公開 API 仕様

- 状態: **決定**
- 対象: library target `swbt`
- 基準断面: [source-baseline.md](source-baseline.md)
- 型関係の正本: [type-modeling.md](type-modeling.md)

この文書は `swbt-rs` の初期公開 API と、その成功・失敗・状態確定の意味を定義する。

## 1. API 方針

- 公開 API は同期・blocking API とする
- controller model と reporting mode は `Controller<M, R>` の型引数で表す
- model 固有のボタンと入力状態は `Button<M>`、`InputState<M>` で表す
- `Stick` と `ImuFrame` は model 非依存の共通値型とする
- 使用できないボタン、存在しないスティック、異なる model の入力状態は可能な限りコンパイル時に拒否する
- profile JSON、status、CLI のような動的境界だけが `ControllerKind`、`ReportingKind`、`ButtonKind` を扱う
- Bumble の型、CID、HCI packet、L2CAP manager を公開 API に出さない
- controller object は `Clone` しない。初期 API では `Send`、非 `Sync` を目標にする
- 状態変更操作は `&mut self` を要求する
- `Drop` は短い best-effort shutdown だけを行い、neutral と終了エラーの確認には明示的な `close()` を使う
- raw HID / HCI bytes を送る公開 API は作らない
- Python 版との互換対象は wire bytes、入力意味、profile schema、状態確定条件であり、class inheritance と coroutine 形式ではない

## 2. crate root の公開要素

```rust
pub use adapter::{list_adapters, AdapterInfo, AdapterSelector};
pub use connection::{
    ConnectOptions, ConnectionPath, ConnectionResult, ConnectionStatus,
    CreateProfileOptions,
};
pub use controller::{
    Controller, ControllerBuilder,
    DirectJoyConL, DirectJoyConR, DirectProController,
    JoyConL, JoyConR, ProController,
};
pub use diagnostics::{GamepadStatus, LifecycleState};
pub use error::{Error, ErrorKind, Result};
pub use input::{
    Button, ButtonKind,
    ImuFrame, ImuSamples, InputState, Stick,
    JoyConLButton, JoyConLInputState,
    JoyConRButton, JoyConRInputState,
    ProButton, ProInputState,
};
pub use model::{
    ControllerModel, HasDualSticks, HasLeftStick, HasRightStick,
};
pub use profile::{
    ControllerColors, ControllerKind, LocalAddress, ProfileIdentity, Rgb24,
};
pub use reporting::{ReportingKind, ReportingMode};
```

marker type は module 経由で参照する。

```rust
pub mod model {
    pub enum Pro {}
    pub enum JoyConL {}
    pub enum JoyConR {}
}

pub mod reporting {
    pub enum Periodic {}
    pub enum Direct {}
}
```

`transport`、`protocol`、`runtime` は公開 module にしない。

## 3. controller 型

### 3.1 generic 正本と alias

```rust
pub struct Controller<M: ControllerModel, R: ReportingMode> {
    // private fields
}
```

```rust
pub type ProController =
    Controller<model::Pro, reporting::Periodic>;
pub type DirectProController =
    Controller<model::Pro, reporting::Direct>;

pub type JoyConL =
    Controller<model::JoyConL, reporting::Periodic>;
pub type DirectJoyConL =
    Controller<model::JoyConL, reporting::Direct>;

pub type JoyConR =
    Controller<model::JoyConR, reporting::Periodic>;
pub type DirectJoyConR =
    Controller<model::JoyConR, reporting::Direct>;
```

6 個の public newtype、6 個の builder 型、6 組の forwarding method は作らない。

### 3.2 共通 builder

```rust
pub struct ControllerBuilder<M: ControllerModel, R: ReportingMode> {
    // private fields
}

impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn builder(
        adapter: impl Into<AdapterSelector>,
    ) -> ControllerBuilder<M, R>;
}
```

```rust
impl<M: ControllerModel, R: ReportingMode> ControllerBuilder<M, R> {
    pub fn profile_path(self, path: impl Into<PathBuf>) -> Self;
    pub fn controller_colors(self, colors: ControllerColors) -> Self;

    pub fn build(self) -> Result<Controller<M, R>>;

    pub fn create_profile(
        self,
        options: CreateProfileOptions,
    ) -> Result<Controller<M, R>>;
}
```

Periodic だけが周期を設定できる。

```rust
impl<M: ControllerModel> ControllerBuilder<M, reporting::Periodic> {
    pub fn report_period(self, period: Duration) -> Self;
}
```

### 3.3 `build()`

`build()` は I/O を開始しない。

- `profile_path = None` は永続 bond を持たない一時 controller を構築する
- `profile_path = Some(path)` では path が存在することを要求する
- existing profile を `PairingProfile<M>` として検証する
- controller kind mismatch は adapter open 前に拒否する
- path が存在しない場合は `ProfileNotFound`
- adapter は `open()` まで開かない

新規 profile 用の存在しない path を `build()` へ渡して、後から controller method で作成する流れは提供しない。

### 3.4 builder の `create_profile()`

新規 profile 作成は builder を消費する複合操作とする。

```text
validate builder and target path
  → create valid empty PairingProfile<M> envelope
  → construct Controller<M, R>
  → open adapter / worker
  → pair to normal-input readiness
  → return Ready Controller<M, R>
```

規則:

- `profile_path` は必須
- path が既に存在する場合は `ProfileAlreadyExists`
- envelope を adapter open より先に原子的に作成する
- pairing に失敗しても envelope は残す
- 失敗時は内部 controller を `close_without_neutral()` 相当で cleanup し、controller object は返さない
- 成功時に返す controller は `Ready`
- `ProfileIdentity::LocalAddress` は対応 gate 完了まで `UnsupportedCapability`

`Controller<M, R>` 自体には `create_profile()` method を生やさない。

### 3.5 builder 制約

- adapter は必須
- `report_period` は `1 ms..=1 s`、既定値は `8 ms`
- Direct builder に `report_period()` は存在しない
- `ControllerKind` と reporting mode を値として指定する method は作らない
- controller colors は build 時に固定し、接続後の setter を提供しない
- public builder に transport injection を入れない

## 4. 基本利用例

### 4.1 existing profile で接続する

```rust
use std::time::Duration;
use swbt::{ConnectOptions, ProButton, ProController};

fn main() -> swbt::Result<()> {
    let mut pad = ProController::builder("usb:0")
        .profile_path("profiles/switch-pro.json")
        .build()?;

    pad.open()?;
    pad.connect(ConnectOptions {
        timeout: Duration::from_secs(30),
        allow_pairing: false,
    })?;

    pad.tap([ProButton::A], Duration::from_millis(80))?;
    pad.neutral()?;
    pad.close()?;
    Ok(())
}
```

### 4.2 新規 profile を作成して pairing する

```rust
use std::time::Duration;
use swbt::{CreateProfileOptions, ProfileIdentity, ProButton, ProController};

fn main() -> swbt::Result<()> {
    let mut pad = ProController::builder("usb:0")
        .profile_path("profiles/switch-pro.json")
        .create_profile(CreateProfileOptions {
            identity: ProfileIdentity::AdapterDefault,
            pair_timeout: Duration::from_secs(60),
        })?;

    pad.tap([ProButton::A], Duration::from_millis(80))?;
    pad.close()?;
    Ok(())
}
```

### 4.3 Joy-Con R の A ボタン

```rust
use std::time::Duration;
use swbt::{JoyConR, JoyConRButton};

fn tap_a(right: &mut JoyConR) -> swbt::Result<()> {
    right.tap([JoyConRButton::A], Duration::from_millis(80))
}
```

`ProButton::A` と `JoyConRButton::A` は同じ論理 `ButtonKind::A` を指すが、型は別である。`JoyConLButton::A` は存在しない。

### 4.4 Direct controller へ完全状態を送る

```rust
use swbt::{
    DirectProController, ProButton, ProInputState, Stick,
};

fn send_one_state(pad: &mut DirectProController) -> swbt::Result<()> {
    let state = ProInputState::neutral()
        .with_buttons([ProButton::L, ProButton::R])
        .with_left_stick(Stick::up(1.0)?);

    pad.send(state)
}
```

## 5. lifecycle

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

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn open(&mut self) -> Result<()>;
    pub fn pair(&mut self, timeout: Duration) -> Result<()>;
    pub fn reconnect(&mut self, timeout: Duration) -> Result<()>;
    pub fn connect(
        &mut self,
        options: ConnectOptions,
    ) -> Result<ConnectionPath>;
    pub fn try_reconnect(
        &mut self,
        timeout: Duration,
    ) -> Result<ConnectionResult>;
    pub fn try_connect(
        &mut self,
        options: ConnectOptions,
    ) -> Result<ConnectionResult>;
    pub fn status(&self) -> GamepadStatus;
    pub fn close(&mut self) -> Result<()>;
    pub fn close_without_neutral(&mut self) -> Result<()>;
}
```

`open()` は冪等とする。`close()` と `close_without_neutral()` も冪等にし、一部 cleanup が失敗しても残りを続行する。

`close()` は接続中なら trailing neutral report を 1 件送信し、未送信の interrupt report がホスト側の待ち行列を離れて HCI コントローラのフロー制御枠へ入るまで期限付きで排出した後、HID channel、Classic ACL、HCI transport、worker を停止する。コントローラ内で送信中のパケットに対する完了クレジットがすべて返るまでは待たない。

`close()` と `close_without_neutral()` は cleanup の完了を待って worker を join し、cleanup または join の失敗を `Result` で返す。

`Drop` は neutral report と pending send の drain を省き、priority shutdown 後の完了を bounded wait する。期限内に完了した worker だけを join し、timeout または completion channel 切断時は worker handle を detach する。`Drop` は終了失敗を返せず、bounded wait の内部時間値は公開 API の保証に含めない。

## 6. 接続 API

```rust
pub struct ConnectOptions {
    pub timeout: Duration,
    pub allow_pairing: bool,
}

pub struct CreateProfileOptions {
    pub identity: ProfileIdentity,
    pub pair_timeout: Duration,
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

`connect()` の順序:

1. usable bond があれば reconnect
2. bond がなく `allow_pairing = true` なら pairing
3. bond がなく `allow_pairing = false` なら `NoBond`
4. reconnect が通信失敗した場合、bond を暗黙削除して fresh pairing へ移らない

`pair()` は一時 controller、または既存の empty profile から pairing を再試行する入口である。新規 file 作成は行わない。

新しい connection session の開始時に input snapshot を neutral へ戻す。接続前に変更した入力状態、前 session の入力状態、前 session の stale event は新しい session へ持ち越さない。

接続 API は次を満たした後だけ成功する。

- Classic ACL が有効
- HID control / interrupt channel が両方 open
- bootstrap neutral report を送信済み
- supported `0x03` set report mode の reply が受理済み
- 0 以外の `0x30` player lights の reply が受理済み
- 同じ connection session で上記を満たす
- handshake state を停止・回収済み

Periodic は最後の automatic input holdoff 終了後に scheduler を開始できた時点で `Ready`。Direct は protocol ready で `Ready` とし、確認用 periodic report を送らない。

## 7. ボタン

### 7.1 論理集合

```rust
#[repr(u8)]
pub enum ButtonKind {
    A = 0x00,
    B = 0x01,
    X = 0x02,
    Y = 0x03,
    L = 0x04,
    R = 0x05,
    ZL = 0x06,
    ZR = 0x07,
    Plus = 0x08,
    Minus = 0x09,
    Home = 0x0A,
    Capture = 0x0B,
    LeftStick = 0x0C,
    RightStick = 0x0D,
    SL = 0x0E,
    SR = 0x0F,
    DpadUp = 0x10,
    DpadDown = 0x11,
    DpadLeft = 0x12,
    DpadRight = 0x13,
}
```

explicit discriminant は論理 ID と table index に使い、NX wire bit とは別契約とする。

### 7.2 モデル付きボタン

```rust
pub struct Button<M: ControllerModel> {
    // private
}

pub type ProButton = Button<model::Pro>;
pub type JoyConLButton = Button<model::JoyConL>;
pub type JoyConRButton = Button<model::JoyConR>;
```

| model | buttons |
|---|---|
| Pro | A/B/X/Y, L/R/ZL/ZR, Plus/Minus/Home/Capture, LeftStick/RightStick, D-pad |
| Joy-Con L | L/ZL, Minus/Capture, LeftStick, SL/SR, D-pad |
| Joy-Con R | A/B/X/Y, R/ZR, Plus/Home, RightStick, SL/SR |

自由な `Button<M>::new(ButtonKind)` は公開しない。dynamic boundary 用に `TryFrom<ButtonKind> for Button<M>` を提供する。

## 8. `Stick`

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
}
```

`Stick` は共通値型とし、method の存在を capability trait で制限する。

```rust
impl<M: HasLeftStick, R: ReportingMode> Controller<M, R> {
    pub fn left_stick(&mut self, stick: Stick) -> Result<()>;
}

impl<M: HasRightStick, R: ReportingMode> Controller<M, R> {
    pub fn right_stick(&mut self, stick: Stick) -> Result<()>;
}

impl<M: HasDualSticks, R: ReportingMode> Controller<M, R> {
    pub fn sticks(&mut self, left: Stick, right: Stick) -> Result<()>;
}
```

## 9. 六軸センサー

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImuFrame {
    accel: [i16; 3],
    gyro: [i16; 3],
}

pub enum ImuSamples {
    Repeat(ImuFrame),
    Frames([ImuFrame; 3]),
}
```

- gyro scale: `0.070 dps/raw`
- accelerometer scale: `1/4096 G/raw`
- non-finite と i16 overflow は reject
- `ImuFrame` と `ImuSamples` は全 model で共通
- model 固有の校正値と wire packing は protocol encoder が `M::SPEC` から選ぶ

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn imu(&mut self, samples: impl Into<ImuSamples>) -> Result<()>;
}
```

## 10. モデル付き `InputState`

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputState<M: ControllerModel> {
    // private model-valid representation
}

pub type ProInputState = InputState<model::Pro>;
pub type JoyConLInputState = InputState<model::JoyConL>;
pub type JoyConRInputState = InputState<model::JoyConR>;
```

```rust
impl<M: ControllerModel> InputState<M> {
    pub fn neutral() -> Self;
    pub fn with_buttons(
        self,
        buttons: impl IntoIterator<Item = Button<M>>,
    ) -> Self;
    pub fn with_imu(self, samples: impl Into<ImuSamples>) -> Self;
    pub fn buttons(&self) -> impl Iterator<Item = Button<M>> + '_;
    pub fn imu_frames(&self) -> &[ImuFrame; 3];
}

impl<M: HasLeftStick> InputState<M> {
    pub fn with_left_stick(self, stick: Stick) -> Self;
    pub fn left_stick(&self) -> Stick;
}

impl<M: HasRightStick> InputState<M> {
    pub fn with_right_stick(self, stick: Stick) -> Self;
    pub fn right_stick(&self) -> Stick;
}

impl<M: HasDualSticks> InputState<M> {
    pub fn with_sticks(self, left: Stick, right: Stick) -> Self;
}
```

異なる model 間の変換、model 非依存 `InputState` alias、stable serde format は提供しない。

## 11. 入力操作と状態確定

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn press(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
    ) -> Result<()>;
    pub fn release(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
    ) -> Result<()>;
    pub fn tap(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
        duration: Duration,
    ) -> Result<()>;
    pub fn neutral(&mut self) -> Result<()>;
    pub fn snapshot(&self) -> InputState<M>;
}
```

空 button iterator は `InvalidInput`。`tap()` duration は `0..=24 h`。

Periodic 専用:

```rust
impl<M: ControllerModel> Controller<M, reporting::Periodic> {
    pub fn apply(&mut self, state: InputState<M>) -> Result<()>;
    pub fn report_period(&self) -> Duration;
}
```

- local state commit 後に成功
- 未接続でも state 更新可能
- wire failure で rollback しない
- next deadline で latest state を送る

Direct 専用:

```rust
impl<M: ControllerModel> Controller<M, reporting::Direct> {
    pub fn send(&mut self, state: InputState<M>) -> Result<()>;
}
```

- 接続済みを要求
- L2CAP path acceptance 後だけ commit
- acceptance 前 failure では previous state を維持
- user-input scheduler を持たない

`tap()` は両方式で押下と解放を最低 1 回ずつ受理させる。解放失敗時は最後に受理された押下 state を維持する。

## 12. profile と controller identity

```rust
pub enum ControllerKind {
    Pro,
    JoyConL,
    JoyConR,
}

pub enum ProfileIdentity {
    AdapterDefault,
    LocalAddress(LocalAddress),
}
```

`ControllerKind` は runtime projection であり、typed builder の選択値ではない。model 宣言、profile 文字列、button capability、stick capability は単一宣言から生成または検査する。

Periodic / Direct は profile kind に含めない。同じ model の profile は両方式で共有できる。

## 13. adapter discovery

```rust
pub fn list_adapters() -> Result<Vec<AdapterInfo>>;
```

adapter を claim / open せず、USB descriptor と interface class から Bluetooth HCI candidate を列挙する。Bumble の selector 型は返さない。

## 14. status と diagnostics

```rust
pub struct GamepadStatus {
    pub lifecycle: LifecycleState,
    pub connected: bool,
    pub controller_kind: ControllerKind,
    pub reporting_kind: ReportingKind,
    pub report_mode: Option<u8>,
    pub input_reports_accepted: u64,
    pub replies_accepted: u64,
    pub last_subcommand: Option<u8>,
    pub last_disconnect_reason: Option<u8>,
    pub worker_failure: Option<String>,
}
```

kind は `M` と `R` から導出し、controller 内部に重複 field として保持しない。`status()` は worker I/O を待たない snapshot とする。

構造化 trace は `tracing` facade を使う。`report_tx_accepted` は transport 受理を意味し、air delivery や Switch UI 反映を意味しない。

## 15. error

```rust
#[non_exhaustive]
pub enum ErrorKind {
    AdapterDiscovery,
    TransportOpen,
    TransportClosed,
    ProfilePathRequired,
    ProfileNotFound,
    ProfileAlreadyExists,
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

通常の静的 API で model 不一致 input を受け付けないため、`UnsupportedInput` は主に CLI、config、trace replay、`ButtonKind -> Button<M>` の動的境界で使う。

message 文言は semver 契約にしない。Bumble error は source chain に保持し、public variant として露出しない。

## 16. thread と cancellation

- `Controller<M, R>` は `Send` を目標とする
- 同時操作は `&mut self` で禁止する
- worker command channel は bounded
- public blocking method は timeout または worker termination を観測できる
- `tap()` delay は worker scheduler が管理し、close で中断できる
- worker panic は `WorkerFailed`

## 17. 初期公開 API に含めないもの

- model 非依存の `Button` / `InputState` alias
- 利用者定義 model / reporting mode
- `AnyController` と型消去された controller collection
- dynamic `ControllerKind` factory
- controller method の `create_profile()`
- raw HID / HCI / L2CAP API
- custom transport public trait
- `JoyConPair`
- controller manager
- 高水準 rumble
- amiibo / NFC
- IR camera
- daemon IPC
- C ABI / FFI
- automatic infinite reconnect
- implicit bond deletion と fresh pairing fallback

動的な統一操作面が必要になった場合は、型消去で失う能力と failure semantics を別仕様で定義する。
