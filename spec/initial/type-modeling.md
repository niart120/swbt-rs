# 型モデリング

- 状態: **決定**
- 公開 API: [api.md](api.md)
- 内部構造: [architecture.md](architecture.md)

この文書は、controller model、reporting mode、入力能力を Rust の型で表す規則を定義する。公開 API と内部 runtime は、この型関係を正本として実装する。

## 1. 基本方針

`swbt-python` の6具象クラスは次の2軸の直積である。

```text
Controller model: Pro / JoyConL / JoyConR
Reporting mode:   Periodic / Direct
```

Rust版の正本:

```rust
pub struct Controller<M, R> {
    // private fields
}
```

- `M`: 使用可能button、stick能力、固定profileを決める
- `R`: `apply()` / `send()`、state commit、scheduler所有を決める

型引数はinstance生成後に変化しない性質だけに使う。接続状態、report mode、player lights、IMU mode、pairing stateはruntime state machineに置く。

## 2. model型

```rust
pub mod model {
    pub enum Pro {}
    pub enum JoyConL {}
    pub enum JoyConR {}
}

pub trait ControllerModel: private::Sealed + Send + 'static {
    const KIND: ControllerKind;
    const PROFILE_NAME: &'static str;
    const SPEC: &'static ModelSpec;
}
```

marker typeは値を持たない。`ControllerModel`はsealedにする。

## 3. reporting型

```rust
pub mod reporting {
    pub enum Periodic {}
    pub enum Direct {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReportingKind {
    Periodic,
    Direct,
}

pub trait ReportingMode: private::Sealed + Send + 'static {
    const KIND: ReportingKind;
}
```

PeriodicとDirectは公開method集合と状態確定条件を変えるため、実行時enumだけでなく型引数として保持する。

## 4. 公開alias

```rust
pub type ProController = Controller<model::Pro, reporting::Periodic>;
pub type DirectProController = Controller<model::Pro, reporting::Direct>;

pub type JoyConL = Controller<model::JoyConL, reporting::Periodic>;
pub type DirectJoyConL = Controller<model::JoyConL, reporting::Direct>;

pub type JoyConR = Controller<model::JoyConR, reporting::Periodic>;
pub type DirectJoyConR = Controller<model::JoyConR, reporting::Direct>;
```

6名は利用者向けaliasであり、6個のpublic newtype、builder型、forwarding実装は作らない。

## 5. `ControllerKind`はruntime projection

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerKind {
    Pro,
    JoyConL,
    JoyConR,
}
```

`ControllerKind`はprofile JSON、diagnostics、CLIで使う実行時表現。model型と独立した正本にしない。

model宣言1箇所から次を生成または検査する。

- model marker
- `ControllerKind` variant
- `ControllerModel::KIND`
- profile文字列
- supported button集合
- stick capability trait
- runtime `ModelSpec`

概念宣言:

```rust
controller_models! {
    Pro {
        kind: Pro,
        profile_name: "pro",
        buttons: [
            A, B, X, Y, L, R, ZL, ZR,
            Plus, Minus, Home, Capture,
            LeftStick, RightStick,
            DpadUp, DpadDown, DpadLeft, DpadRight,
        ],
        sticks: [left, right],
    }
    JoyConL {
        kind: JoyConL,
        profile_name: "joycon_l",
        buttons: [
            L, ZL, Minus, Capture, LeftStick, SL, SR,
            DpadUp, DpadDown, DpadLeft, DpadRight,
        ],
        sticks: [left],
    }
    JoyConR {
        kind: JoyConR,
        profile_name: "joycon_r",
        buttons: [
            A, B, X, Y, R, ZR, Plus, Home,
            RightStick, SL, SR,
        ],
        sticks: [right],
    }
}
```

macro採否は実装詳細だが、同情報を複数の手書き表へ重複させないことは仕様とする。

## 6. ボタンの全体集合

```rust
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

explicit discriminantは論理IDとtable indexに使う。NX reportのbyte/bit位置とはみなさない。wire mappingは`(ControllerKind, ButtonKind)`から明示的に決める。

## 7. モデル付きボタン

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Button<M: ControllerModel> {
    kind: ButtonKind,
    _model: PhantomData<fn() -> M>,
}

pub type ProButton = Button<model::Pro>;
pub type JoyConLButton = Button<model::JoyConL>;
pub type JoyConRButton = Button<model::JoyConR>;
```

自由なconstructorは公開せず、modelで使用可能なassociated constantだけを生成する。

```rust
let pro_a = ProButton::A;
let right_a = JoyConRButton::A;
let left_up = JoyConLButton::DPAD_UP;
```

`ProButton::A`と`JoyConRButton::A`は同じ`ButtonKind::A`を指すが型は異なる。

compile-fail:

```compile_fail
left.press([ProButton::A])?;
```

```compile_fail
let button = JoyConLButton::A;
```

動的境界:

```rust
impl<M: ControllerModel> TryFrom<ButtonKind> for Button<M> {
    type Error = Error;
    fn try_from(kind: ButtonKind) -> Result<Self>;
}
```

静的Rustコードはtyped constantを使い、通常経路を動的検査へ戻さない。

## 8. 共通値型

model非依存:

- `Stick`: 12-bit x/y座標
- `ImuFrame`: accel 3軸 + gyro 3軸
- `ImuSamples`: 1frame反復または3frame
- `ControllerColors`
- `Rgb24`
- `LocalAddress`

ProとJoy-Conで同じ六軸値を表すために`ImuFrame<Pro>`等を作らない。model差は`ModelSpec`とprotocol encoderの責務。

## 9. stick能力

```rust
pub trait HasLeftStick: ControllerModel {}
pub trait HasRightStick: ControllerModel {}
pub trait HasDualSticks: HasLeftStick + HasRightStick {}
```

```text
Pro      : HasLeftStick + HasRightStick + HasDualSticks
JoyConL  : HasLeftStick
JoyConR  : HasRightStick
```

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

Joy-Con Lにright、Joy-Con Rにleft、片側Joy-Conにdual methodを公開しない。

## 10. モデル付き入力状態

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputState<M: ControllerModel> {
    // private model-valid representation
}
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
}

impl<M: HasRightStick> InputState<M> {
    pub fn with_right_stick(self, stick: Stick) -> Self;
}

impl<M: HasDualSticks> InputState<M> {
    pub fn with_sticks(self, left: Stick, right: Stick) -> Self;
}
```

Pro用stateをJoy-Conへ渡せない。公開constructorで不正な共通stateを作り、send時にkind検査する設計へ戻さない。

## 11. reportingによるmethod制約

共通:

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn press(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> Result<()>;
    pub fn release(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> Result<()>;
    pub fn tap(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
        duration: Duration,
    ) -> Result<()>;
    pub fn imu(&mut self, samples: impl Into<ImuSamples>) -> Result<()>;
    pub fn neutral(&mut self) -> Result<()>;
    pub fn snapshot(&self) -> InputState<M>;
}
```

Periodic:

```rust
impl<M: ControllerModel> Controller<M, reporting::Periodic> {
    pub fn apply(&mut self, state: InputState<M>) -> Result<()>;
    pub fn report_period(&self) -> Duration;
}
```

Direct:

```rust
impl<M: ControllerModel> Controller<M, reporting::Direct> {
    pub fn send(&mut self, state: InputState<M>) -> Result<()>;
}
```

Directに`apply()`、Periodicに`send()`は存在しない。

## 12. 共通builder

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

impl<M: ControllerModel> ControllerBuilder<M, reporting::Periodic> {
    pub fn report_period(self, period: Duration) -> Self;
}
```

builderは`ControllerKind`、button capability、reporting modeを値として受け取らない。

`build()`:

- no profile path → ephemeral controller
- existing path → `PairingProfile<M>` validation
- nonexistent path → `ProfileNotFound`
- I/Oなし

`create_profile()`:

- path必須、target不存在を要求
- valid empty envelopeをadapter open前にcreate-new
- typed controllerをopenしてpairing
- success時Ready controllerを返す
- failure時envelopeを残し内部resourceをcleanup

`Controller<M,R>`に`create_profile()` methodは置かない。

## 13. typed profile

```rust
pub struct PairingProfile<M: ControllerModel> {
    // validated document
}
```

JSON parserは`ControllerKind`を読み、`M::KIND`と照合してtyped profileへ変換する。

`PairingProfile<model::Pro>`をJoy-Conへ渡すAPIは作らない。raw inspectionとCLIだけが`ControllerKind`を直接扱う。

## 14. dynamic boundary

modelが実行時に決まるCLIは入口で一度だけ分岐する。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後はtyped pathを使う。core runtimeで`ControllerKind`を繰り返しmatchしない。

`AnyController`やtrait objectは初期APIに含めない。

## 15. runtime境界

`ControllerWorker<M,R>`、command、state store、protocol sessionは可能な範囲で`M`と`R`を保持する。

```rust
struct ControllerWorker<M: ControllerModel, R: ReportingMode> {
    state: InputState<M>,
    // transport and session
}
```

Bumble transport、HCI、L2CAP、HIDP framingはmodel非依存なのでgenericにしない。model固有値は`M::SPEC`を参照する。

型消去を許可する境界:

- profile JSON DTO
- diagnosticsのkind値
- CLI文字列
- Bumbleへ渡すbytes / events

core input stateを早期に`ControllerKind + untyped buttons`へ変換しない。

## 16. 検証規則

compile-pass/failで固定する。

- ProとJoy-Con RでそれぞれAを使用可能
- Joy-Con LにA constantなし
- Joy-Con LへProButtonを渡せない
- Joy-Con Lにright stickなし
- Joy-Con Rにleft stickなし
- Directにapplyなし
- Periodicにsendなし
- Pro stateをJoy-Conへ渡せない
- ImuFrameは全model共通
- Direct builderにreport periodなし
- controllerにcreate_profile methodなし
- builder create_profileがtyped controllerを返す

値audit:

- ButtonKind discriminant重複なし
- 全model buttonにwire mappingあり
- model宣言、ControllerKind、profile名、capability一致

## 17. 対象外

- lifecycle typestate
- 利用者定義model / reporting
- model間暗黙変換
- `InputState<M>` stable serde
- model非依存Button alias
- unsupported inputのsilent ignore
- `AnyController`
- controller methodのprofile create-new

書き味をそろえる目的だけで異なる能力を同一型へ戻さない。共通物理量は共通値型にし、能力差がある操作は型で分ける。