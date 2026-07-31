# M7 Joy-Con L/R Periodic、Direct、実機

- 状態: **着手中**
- milestone: M7
- branch: `feat/unit-008-m7-joy-con`
- 正本:
  - `spec/initial/roadmap.md` 10
  - `spec/initial/api.md` 3、4、7、9、10
  - `spec/initial/type-modeling.md` 2–10、14、15
  - `spec/initial/architecture.md` 7、9、12、15、16、18
  - `spec/initial/testing.md` 5、7、8、10、12、13
- Python 基準断面:
  - repository: `niart120/swbt-python`
  - version: `0.6.0`
  - revision: `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`
- Bumble fork:
  - repository: `https://github.com/niart120/bumble-rs`
  - branch: `fix/external-host-reader-lifecycle`
  - revision: `b8c7cd625bc2ac2f58a4beb4ade1264426969819`
  - public fork と branch push だけを許可範囲とし、upstream PR / issue は作成しない

## 1. 目的

既存の model generic protocol/runtime/profile を Joy-Con L と Joy-Con R の公開利用経路で検査し、
左右固有の button、stick、device identity、profile model、Periodic/Direct reporting が混線しない
ことを確定する。Windows 11、CSR8510 A10、WinUSB、Switch 2 system version `22.5.0`
（ユーザ報告）の実機で左右を別々に pairing し、同じ model の profile を Periodic から Direct へ
再利用して入力、neutral、期限付き close、adapter reopen を確認する。

M7 は新しい `JoyConPair` や左右をまとめた動的 controller を追加しない。左右の machine evidence と
Switch UI の人手観測を分離し、片側の成功を他方の成功として扱わない。

## 2. 現状と Intent Delta

M7 着手時点で次は実装・検査済みである。

- `JoyConL` / `JoyConR`、Periodic/Direct alias、model-valid button/state、片側 stick trait
- Joy-Con L の D-pad/L/ZL/Minus/Capture/LeftStick/SL/SR と Joy-Con R の
  ABXY/R/ZR/Plus/Home/RightStick/SL/SR
- L/R で異なる SL/SR wire position
- Python fixture による input report、device info、SPI device type/colors/calibration
- model 固有 local name、SDP policy、advertising/EIR projection
- fake runtime の3 model×2 reporting smoke と、virtual Classic packet path の6組 Ready
- schema v2 typed profile の3 model一致判定

着手時の再確認では、Rust 1.87 の model test 3件、pinned Python protocol fixture test 6件、
all-feature virtual 6組 matrix 1件が成功した。この既存 Green を M7 の新規実装として数えず、
以下の未達差分だけを TDD item にする。

未達の差分は次である。

| 境界 | M7 着手時 | M7 完了条件 |
|---|---|---|
| profile fixture | Python/Rust round-trip の外部 fixture は Pro 1件 | Joy-Con L/R fixture を追加し、各 typed reader と cross-model reject を検査 |
| fake runtime | L/R 各 reporting の単一 button smoke | 左右固有 button/stick、snapshot、Direct idle/transaction、neutral close を明示検査 |
| virtual packet path | 6組の Ready/close は検査済み | L/R 固有 input bytes、device info、SPI colors、reporting readiness を同じ session で検査 |
| profile reuse | file-backed Periodic→Direct は Pro のみ | Joy-Con L/R を別 profile で Periodic→Direct reconnect し、bytes 不変と cross-model reject を検査 |
| hardware | Pro のみ | L/R を別 run、別 profile、別 UI record で Periodic pairing→Direct reconnect まで記録 |

## 3. 対象範囲

- pinned Python profile fixture の Joy-Con L/R case
- Joy-Con L/R typed profile read/write と model mismatch
- Joy-Con L/R の左右固有 button/stick を使う fake runtime test
- Joy-Con L/R の Periodic/Direct virtual Classic session
- model 固有 device info、SPI colors、input report の packet-level assertion
- Joy-Con L/R 別 profile の Periodic→Direct reuse
- Joy-Con L/R hardware runner
- fresh Periodic pairing、stored-key Direct reconnect、neutral close、adapter reopen
- secret-free machine NDJSON と別の UI observation record
- beta.1 criteria note

## 4. 対象外

- `JoyConPair`、左右を束ねる API、grip mode abstraction
- 左右同時接続、controller order の自動操作
- unsupported button/stick の compile-fail fixture
- IMU mode の追加、long-run jitter、stable diagnostics schema、`swbt-probe`: M8
- Linux、macOS、cross compile、release/publish: M9
- explicit local Bluetooth address
- automatic infinite reconnect と invalid bond の暗黙削除
- Python repository の変更
- Bumble upstream PR / issue

## 5. 振る舞い仕様

### 5.1 model と typed input

- Joy-Con L の supported button 集合に `A` を含めない
- Joy-Con R の supported button 集合に D-pad を含めない
- Joy-Con L は left stick だけ、Joy-Con R は right stick だけを公開する
- L/R の `SL` / `SR` は同じ論理名でも model 固有 wire position を使う
- 利用不能な反対側 stick は input report 上で neutral を保つ
- `JoyConPair`、untyped button vector、実行時 model switch を core runtime に追加しない

型や method が存在しないことだけを検査する compile-fail suite は作らない。公開 library、example、
rustdoc の通常 compile と、実在する method の正の型検査を使う。

### 5.2 protocol identity

Joy-Con L/R は pinned Python fixture と一致する local name、SDP policy、device info、SPI device type、
default colors、button bytes を同じ typed `M` から投影する。Periodic/Direct は protocol identity を
変えない。Ready は各 session の report-mode `0x30` と非0 player lights の reply 後に成立する。

### 5.3 profile と reconnect

Joy-Con L と Joy-Con R は別々の schema v2 profile を使う。profile の `controller_kind` が `M::KIND`
と異なる場合は adapter open 前に `ProfileControllerMismatch` とする。各 model では Periodic pairing で
保存した profile を Direct reconnect に再利用し、reporting mode 用 field を追加しない。

reconnect failure は profile を暗黙削除せず fresh pairing へ fallback しない。成功した Direct run
では実行前後の profile bytes が完全一致する。

### 5.4 Periodic / Direct input

Periodic は Ready 後に最新 snapshot を期限どおり送る。Direct は Ready 後の idle で user input
report を周期送信せず、`send` の transport acceptance 後だけ snapshot を確定する。左右とも
明示 neutral と `close()` の final neutral を送る。

実機の最低入力集合:

- Joy-Con L: D-pad 4方向、L+ZL、SL+SR、left stick 4方向
- Joy-Con R: A/B/X/Y、R+ZR、SL+SR、right stick 4方向

### 5.5 hardware evidence

runner は adapter、model、profile、connection path、reporting mode、timeout、run index を明示入力と
する。pairing は存在しない target を create-new し、reconnect は既存の model-matching profile を
read-only preflight する。終了時に profile parse/model、neutral close、adapter reopen を検査する。

標準出力は秘密情報を含まない NDJSON とし、adapter selector、profile path、raw profile、peer
address、link key、USB serial、error source を出力しない。command acceptance は UI 反映の証拠に
せず、`ui_observed` は `null` のままにする。A/D-pad、shoulder、SL+SR、stick、残留入力なしは
ユーザ観測を左右別の record に保存する。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-done | T01 pinned Python profile fixture を Pro/L/R の3 caseにし、typed round-trip と cross-model reject を検査する | new/regression | integration | synthetic key、secret-free failure |
| todo | T02 fake runtime で Joy-Con L/R×Periodic/Direct の左右固有 input、snapshot、neutral close を検査する | new/regression | integration | model/stick sideを混線させない |
| todo | T03 virtual Classic で L/R の device info、SPI colors、typed input、Periodic/Direct readiness を packet-level 検査する | new/regression | integration | M4の6組 Readyを強化 |
| todo | T04 L/R 別の同一 profile を Periodic→Direct で再利用し、Direct idle、profile不変、cross-model rejectを検査する | new/regression | integration | Pro-only M6 testを一般化 |
| todo | T05 hardware runner で Joy-Con L の Periodic pairing、Direct reconnect、入力、close を記録する | new | hardware | machine/UIを別 recordにする |
| todo | T06 hardware runner で Joy-Con R の Periodic pairing、Direct reconnect、入力、close を記録する | new | hardware | Lと別 profile/evidence |
| todo | T07 completion gate、public docs、beta.1 criteria note、self-review を確定する | new | docs/package | 未検証事項を明記 |

### 6.1 TDD cycle evidence

各 item の red、green、refactor、targeted gate、commit をここへ追記する。既存実装が最初の test で
満たした regression item は red を捏造せず、その事実と追加 test の価値を記録する。

| phase | item | evidence |
|---|---|---|
| refactor-done | T01 | red: `cargo +1.87.0 test --test profile_compat --locked` は期待する Pro/L/R 3 ID に対して既存 fixture が Pro 1件だけのため、round-trip と cross-model の2 testが失敗。green: synthetic Classic link key を持つ adapter-default Joy-Con L/R caseを固定 fixtureへ追加し、各 `PairingProfile<M>` の未知 field 保持、決定的 JSON、opposite Joy-Con の `ProfileControllerMismatch` を検査して2 passed / 1 manual ignored。固定 repository HEAD `84d2723...` の Python 3.13.5 reader は3 caseを `pro_controller` / `joycon_left` / `joycon_right` として読み、key値は出力していない。profile document 7 test、target clippy `-D warnings`、rustfmt、diff checkが成功。refactor: case ID順序を固定し、3 modelのround-tripをgeneric helperへ集約 |

## 7. 対象ファイル

- `tests/fixtures/python-v0.6.0/profile/`
- `tests/profile_compat.rs`
- `src/controller/runtime_tests.rs`
- `src/runtime/transport/virtual_tests.rs`
- `examples/`
- `README.md`、crate rustdoc
- `evidence/` の Joy-Con L/R secret-free 実機証跡
- 本作業仕様

production code は red が既存の model generic 実装の不足を示した場合だけ変更する。

## 8. 検証

TDD item ごとに同じ targeted command で red/green を確認する。完了 gate:

```powershell
cargo +1.87.0 check --all-targets --all-features --locked
cargo +1.87.0 test --all-features --locked
cargo +1.87.0 test --locked
cargo +1.87.0 test --no-default-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --locked -- -D warnings
cargo +1.87.0 build --all-features --locked
cargo +1.87.0 build --locked
cargo +1.87.0 build --no-default-features --locked
cargo +1.87.0 doc --all-features --no-deps --locked
cargo fmt --check
git diff --check
```

Python reader を使う場合は version、revision、実行 command を記録する。T05/T06 以外の hardware、
network、cross compile、publish は実行しない。

## 9. 先送り事項

- `JoyConPair` と左右同時接続: roadmap 対象外
- IMU追加仕様、long-run、stable diagnostics、probe: M8
- Linux、macOS、release: M9
- explicit local address: 独立 milestone
- Bumble upstream contribution: ユーザが明示的に許可するまで実施しない

## 10. 完了チェックリスト

- [ ] T01–T07 が個別 commit で完了している
- [ ] Joy-Con L supported集合にAを含めない
- [ ] Joy-Con R supported集合にD-padを含めない
- [ ] stick capabilityとneutral opposite sideがmodelに一致する
- [ ] SL/SR mappingがL/RそれぞれのPython fixtureと一致する
- [ ] device info、SPI colors、SDP identityがmodelに一致する
- [ ] L/R×Periodic/Directがfake/virtual Readyとtyped inputを通る
- [ ] cross-model profileをadapter open前に拒否する
- [ ] L/Rそれぞれで同一profileをPeriodic/Direct利用する
- [ ] Direct idleで周期user input reportを送らない
- [ ] L/R別の実機machine evidenceとUI observationを記録する
- [ ] key material、raw profile、secretがerror、trace、evidenceに残らない
- [ ] `JoyConPair`を追加していない
- [ ] beta.1 criteria、未実行条件、residual riskを記録する
- [ ] upstream Bumble PR / issueを作成していない
- [ ] self-reviewとcompletion gateを通す
- [ ] `spec/complete/unit_008/`へ移動する
