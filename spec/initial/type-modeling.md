# 型モデリング

- 状態: **決定**
- 公開 API: [api.md](api.md)
- 内部構造: [architecture.md](architecture.md)

この文書は、コントローラーモデル、送信方式、入力能力を Rust の型で表す規則を定義する。`swbt-rs` の公開 API と内部 runtime は、この文書の型関係を正本として実装する。

## 1. モデリング方針

`swbt-python` の公開型は、次の 2 軸を 6 個の具象クラスとして表している。

```text
Controller model: Pro / JoyConL / JoyConR
Reporting mode:   Periodic / Direct
```

Rust 版では 6 個の独立実装を作らず、次の 1 型を正本とする。

```rust
pub struct Controller<M, R> {
    // private fields
}
```

- `M`: コントローラーモデル。使用可能なボタン、スティック、固定 profile を決める
- `R`: 送信方式。`apply()` と `send()` のどちらを公開するか、状態確定条件、scheduler 所有を決める

型で表すのは、インスタンス生成後に変化しない性質だけとする。接続状態、report mode、player lights、IMU mode、pairing 状態は runtime state machine に置き、型引数へ追加しない。

## 2. モデル型と送信方式型

### 2.1 モデル型

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

モデル型は値を持たない marker type である。利用者が独自モデルを追加できないよう `ControllerModel` は sealed にする。

### 2.2 送信方式型

```rust
pub mod reporting {
    pub enum Periodic {}
    pub enum Direct {}
}

pub trait ReportingMode: private::Sealed + Send + 'static {
    const KIND: ReportingKind;
}
```

`Periodic` と `Direct` は公開 method の集合と状態確定条件を変える。したがって実行時 enum だけではなく、型引数として保持する。

### 2.3 公開 alias

```rust
pub type ProController = Controller<model::Pro, reporting::Periodic>;
pub type DirectProController = Controller<model::Pro, reporting::Direct>;

pub type JoyConL = Controller<model::JoyConL, reporting::Periodic>;
pub type DirectJoyConL = Controller<model::JoyConL, reporting::Direct>;

pub type JoyConR = Controller<model::JoyConR, reporting::Periodic>;
pub type DirectJoyConR = Controller<model::JoyConR, reporting::Direct>;
```

6 個の名前は利用者向けの別名として残す。実装、builder、runtime、入力状態の正本は generic 型であり、6 個の public newtype と forwarding method は作らない。

## 3. `ControllerKind` は runtime projection

profile JSON、diagnostics、CLI 引数では、コンパイル時に `M` が決まっていないため実行時表現が必要になる。

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerKind {
    Pro,
    JoyConL,
    JoyConR,
}
```

`ControllerKind` とモデル型を独立した 2 つの正本にしない。モデル宣言を 1 箇所に集約し、次を同じ宣言から生成する。

- `model::Pro` / `model::JoyConL` / `model::JoyConR`
- `ControllerKind` variant
- `ControllerModel::KIND`
- profile 文字列表現
- 使用可能ボタン集合
- スティック能力 trait の実装
- runtime 用 `ModelSpec`

概念上の宣言は次の形とする。

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

macro の採否は実装詳細だが、同じ情報を手作業の複数表へ重複させないことは仕様とする。`ControllerBuilder<M, R>` は `ControllerKind` を引数や field として受け取らず、必要な値を常に `M::KIND` から導出する。

## 4. ボタン型

### 4.1 全体集合

全モデルで使われる論理ボタン名は `ButtonKind` に集約する。

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

明示 discriminant は論理 ID と table index に使う。NX input report の byte / bit 位置そのものとはみなさない。wire mapping は `(ControllerKind, ButtonKind)` から明示的に決め、golden test で固定する。モデルによって同名ボタンの物理配置や意味が異なっても、`ButtonKind` の数値へ暗黙に埋め込まない。

### 4.2 モデル付きボタン

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Button<M: ControllerModel> {
    kind: ButtonKind,
    _model: PhantomData<fn() -> M>,
}
```

`Button<M>` の自由な constructor は公開しない。各モデルで使用可能な associated constant だけを生成する。

```rust
pub type ProButton = Button<model::Pro>;
pub type JoyConLButton = Button<model::JoyConL>;
pub type JoyConRButton = Button<model::JoyConR>;
```

```rust
let pro_a = ProButton::A;
let right_a = JoyConRButton::A;
let left_up = JoyConLButton::DPAD_UP;
```

`ProButton::A` と `JoyConRButton::A` は同じ `ButtonKind::A` を指すが、型は異なる。これにより、別モデルのボタンを誤って渡せない。

次はコンパイルエラーにする。

```compile_fail
let mut left: JoyConL = make_left();
left.press([ProButton::A])?;
```

次も、associated constant 自体が存在しないためコンパイルエラーにする。

```compile_fail
let button = JoyConLButton::A;
```

動的入力境界では明示変換を使う。

```rust
impl<M: ControllerModel> TryFrom<ButtonKind> for Button<M> {
    type Error = Error;

    fn try_from(kind: ButtonKind) -> Result<Self>;
}
```

CLI や設定ファイルから得た `ButtonKind` が対象モデルで使えない場合は `UnsupportedInput` を返す。静的な Rust 呼び出しではモデル付き定数を使い、通常経路を動的検査へ戻さない。

## 5. 共通値型とモデル能力

### 5.1 共通値型

次はモデルに依存しない値として、非 generic のまま保持する。

- `Stick`: 12-bit の x / y 座標
- `ImuFrame`: 加速度 3 軸と角速度 3 軸
- `ControllerColors`: 24-bit RGB の集合
- `Rgb24`
- `LocalAddress`

Pro Controller と Joy-Con で同じ六軸センサー値を表すために、`ImuFrame<Pro>` と `ImuFrame<JoyConR>` のような別型を作らない。モデル差が wire encoding や校正値にある場合は、値型ではなく model spec と protocol encoder の責務とする。

### 5.2 スティック能力

```rust
pub trait HasLeftStick: ControllerModel {}
pub trait HasRightStick: ControllerModel {}
pub trait HasDualSticks: HasLeftStick + HasRightStick {}
```

実装関係:

```text
Pro      : HasLeftStick + HasRightStick + HasDualSticks
JoyConL  : HasLeftStick
JoyConR  : HasRightStick
```

`Controller<M, R>` と `InputState<M>` のスティック method は能力 trait の境界で公開する。

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

Joy-Con L には `right_stick()`、Joy-Con R には `left_stick()`、片側 Joy-Con には `sticks()` を公開しない。

## 6. モデル付き入力状態

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputState<M: ControllerModel> {
    // private model-valid representation
}
```

`InputState<M>` は、同じモデルの `Button<M>` と、そのモデルが持つスティックだけから構築できる。

```rust
impl<M: ControllerModel> InputState<M> {
    pub fn neutral() -> Self;
    pub fn with_buttons(
        self,
        buttons: impl IntoIterator<Item = Button<M>>,
    ) -> Self;
    pub fn with_imu(self, frames: [ImuFrame; 3]) -> Self;
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

Pro 用状態を Joy-Con へ適用することはできない。

```compile_fail
let state: InputState<model::Pro> = InputState::neutral()
    .with_buttons([ProButton::A]);
let mut right: JoyConR = make_right();
right.apply(state)?;
```

内部表現はモデルごとに有効な状態しか構築できない形にする。公開 constructor で一旦不正な共通状態を作り、送信時に controller kind で検査する設計へ戻さない。

## 7. 送信方式による API 制約

共通 method は `Controller<M, R>` に実装する。

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn press(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> Result<()>;
    pub fn release(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> Result<()>;
    pub fn tap(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
        duration: Duration,
    ) -> Result<()>;
    pub fn imu(&mut self, frames: &[ImuFrame]) -> Result<()>;
    pub fn neutral(&mut self) -> Result<()>;
    pub fn snapshot(&self) -> InputState<M>;
}
```

Periodic 専用:

```rust
impl<M: ControllerModel> Controller<M, reporting::Periodic> {
    pub fn apply(&mut self, state: InputState<M>) -> Result<()>;
    pub fn report_period(&self) -> Duration;
}
```

Direct 専用:

```rust
impl<M: ControllerModel> Controller<M, reporting::Direct> {
    pub fn send(&mut self, state: InputState<M>) -> Result<()>;
}
```

Direct に `apply()`、Periodic に `send()` は存在しない。実行時に `UnsupportedOperation` を返す共通 method は作らない。

## 8. 共通 builder

```rust
pub struct ControllerBuilder<M: ControllerModel, R: ReportingMode> {
    // private fields
}
```

```rust
impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    pub fn builder(
        adapter: impl Into<AdapterSelector>,
    ) -> ControllerBuilder<M, R>;
}
```

共通設定は generic builder に 1 度だけ実装する。`report_period()` は Periodic builder にだけ実装する。

```rust
impl<M: ControllerModel, R: ReportingMode> ControllerBuilder<M, R> {
    pub fn profile_path(self, path: impl Into<PathBuf>) -> Self;
    pub fn controller_colors(self, colors: ControllerColors) -> Self;
    pub fn build(self) -> Result<Controller<M, R>>;
}

impl<M: ControllerModel> ControllerBuilder<M, reporting::Periodic> {
    pub fn report_period(self, period: Duration) -> Self;
}
```

builder は `ControllerKind`、button capability、reporting mode を値として受け取らない。これらは `M` と `R` から決まる。

## 9. profile と型の対応

JSON parser はまず実行時値として `ControllerKind` を読む。typed controller が使用する前に `M::KIND` と照合し、成功した文書を `PairingProfile<M>` として保持する。

```rust
pub struct PairingProfile<M: ControllerModel> {
    // validated document
}

impl<M: ControllerModel> PairingProfile<M> {
    pub fn load(path: &Path) -> Result<Self>;
}
```

`PairingProfile<model::Pro>` を `Controller<model::JoyConR, _>` へ渡す API は作らない。raw profile inspection と CLI の動的選択だけが `ControllerKind` を直接扱う。

## 10. dynamic boundary

型が実行時にしか決まらない CLI や設定ファイルでは、境界で 1 度だけ `ControllerKind` を分岐させる。

```rust
match kind {
    ControllerKind::Pro => run::<model::Pro>(),
    ControllerKind::JoyConL => run::<model::JoyConL>(),
    ControllerKind::JoyConR => run::<model::JoyConR>(),
}
```

分岐後は generic な typed path を使う。core runtime の各操作で `ControllerKind` を繰り返し `match`し、公開型の保証を失わせない。

異なるモデルを 1 collection に格納する `AnyController` や trait object は初期公開 API に含めない。必要性が明確になった時点で、型消去により失う能力を別仕様で定義する。

## 11. 内部 runtime の境界

`ControllerWorker<M, R>`、command、state store、protocol session は可能な範囲で `M` と `R` を保持する。

```rust
struct ControllerWorker<M: ControllerModel, R: ReportingMode> {
    state: InputState<M>,
    // transport and session fields
}
```

Bumble transport、HCI、L2CAP、HIDP framing はモデル非依存なので generic にしない。model 固有値が必要な箇所では `M::SPEC` を参照する。

型消去は次の境界だけで許可する。

- profile JSON DTO
- diagnostics の `ControllerKind`
- CLI の文字列引数
- Bumble へ渡す bytes と transport event

core input state を `ControllerKind + untyped buttons` へ早期変換しない。

## 12. 検証規則

最低限、次を compile-pass / compile-fail test で固定する。

- Pro と Joy-Con R のそれぞれで `A` を使用できる
- Joy-Con L に `A` constant が存在しない
- Joy-Con L controller に `ProButton` を渡せない
- Joy-Con L に `right_stick()` が存在しない
- Joy-Con R に `left_stick()` が存在しない
- Direct に `apply()` が存在しない
- Periodic に `send()` が存在しない
- `InputState<Pro>` を Joy-Con L/R へ渡せない
- `ImuFrame` は全モデルで同じ型として使える
- `ButtonKind` discriminant が重複しない
- 全 model button が wire mapping table を持つ
- model 宣言、`ControllerKind`、profile 名、capability table が一致する

compile-fail test には `trybuild` または同等の UI test harness を使う。rustdoc の `compile_fail` だけを唯一の保証にしない。

## 13. 対象外

初期仕様では次を行わない。

- lifecycle typestate
- 利用者定義 model
- 利用者定義 reporting mode
- model 間の暗黙変換
- `InputState<M>` の stable serde format
- model 非依存の `Button` alias
- unsupported input を受け付けて実行時に無視する API
- `AnyController` による型消去された統一操作面

書き味をそろえる目的だけで異なる入力能力を同一型へ戻さない。共通の物理量は共通値型にし、能力差がある操作は型で分ける。