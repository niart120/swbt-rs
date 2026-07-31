# M7 Joy-Con L/R Periodic、Direct、実機

- 状態: **完了**
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
| refactor-done | T02 fake runtime で Joy-Con L/R×Periodic/Direct の左右固有 input、snapshot、neutral close を検査する | new/regression | integration | model/stick sideを混線させない |
| refactor-done | T03 virtual Classic で L/R の device info、SPI colors、typed input、Periodic/Direct readiness を packet-level 検査する | new/regression | integration | M4の6組 Readyを強化 |
| refactor-done | T04 L/R 別の同一 profile を Periodic→Direct で再利用し、Direct idle、profile不変、cross-model rejectを検査する | new/regression | integration | Pro-only M6 testを一般化 |
| refactor-done | T05 hardware runner で Joy-Con L の Periodic pairing、Direct reconnect、入力、close を記録する | new | hardware | machine/UIを別 recordにする |
| refactor-done | T06 hardware runner で Joy-Con R の Periodic pairing、Direct reconnect、入力、close を記録する | new | hardware | Lと別 profile/evidence |
| refactor-skipped | T07 completion gate、public docs、beta.1 criteria note、self-review を確定する | new | docs/package | 未検証事項を明記 |

### 6.1 TDD cycle evidence

各 item の red、green、refactor、targeted gate、commit をここへ追記する。既存実装が最初の test で
満たした regression item は red を捏造せず、その事実と追加 test の価値を記録する。

| phase | item | evidence |
|---|---|---|
| refactor-done | T01 | red: `cargo +1.87.0 test --test profile_compat --locked` は期待する Pro/L/R 3 ID に対して既存 fixture が Pro 1件だけのため、round-trip と cross-model の2 testが失敗。green: synthetic Classic link key を持つ adapter-default Joy-Con L/R caseを固定 fixtureへ追加し、各 `PairingProfile<M>` の未知 field 保持、決定的 JSON、opposite Joy-Con の `ProfileControllerMismatch` を検査して2 passed / 1 manual ignored。固定 repository HEAD `84d2723...` の Python 3.13.5 reader は3 caseを `pro_controller` / `joycon_left` / `joycon_right` として読み、key値は出力していない。profile document 7 test、target clippy `-D warnings`、rustfmt、diff checkが成功。refactor: case ID順序を固定し、3 modelのround-tripをgeneric helperへ集約 |
| refactor-done | T02 | red: 6組の fake-runtime smoke に `close()` 後の snapshot neutral を要求すると全件失敗し、最後に受理された利用者入力が保持されていた。`spec/initial/api.md` と公開 rustdoc を確認すると、`close()` の契約は trailing neutral report の送信であり、snapshot の neutral reset は次の connection session 開始時であるため、この期待値は誤りと判定した。green: Joy-Con L は L/ZL/SL/SR/D-pad/left stick、Joy-Con R は A/R/ZR/SL/SR/right stick を Periodic/Direct で適用し、Ready 中と close 後の snapshot、最終 `0x30` の neutral button/stick bytes、disconnect/transport close/worker join を6件で検査して成功。common input bridge 1件、all-feature clippy `-D warnings`、rustfmt、diff checkも成功。refactor: smoke helperを単一buttonからmodel-valid `InputState<M>` 入力へ一般化し、共通 close assertionへwire neutral検査を集約 |
| refactor-done | T03 | red: 6組の virtual packet test に device-info `0x02` と SPI colors `0x10` reply を要求すると、既存 virtual peer が readiness 用の report mode と player lights しか送らないため最初の `0x02` 不在で失敗。green: peer request 列を identity、colors、report mode、player lights の順に拡張し、固定 Python fixture 由来の Pro/L/R device-info 12 bytes、colors 12 bytes、全対応button 3 bytesを各 Periodic/Direct session の実パケットで検査して成功。report mode `0x30` と非0 player lights 後の Ready、typed snapshot、neutral closeを維持した。virtual transport 5 test、all-feature clippy `-D warnings`、rustfmt、diff checkが成功。refactor: model別期待値を1構造体にまとめ、全対応button stateとsubcommand reply検索をgeneric helperへ集約 |
| refactor-done | T04 | regression-green: 初回の `cargo +1.87.0 test same_joycon_profiles_reconnect_periodic_then_direct_without_model_leakage --all-features --locked` から成功し、既存の model-generic profile key-store/runtime に製品コード不足はなかった。Joy-Con L/R を別 schema v2 fileで検査し、反対側 model の public builder が `ProfileControllerMismatch` を adapter open 前に返すこと、Periodic stored-key reconnect と全対応button、同じfileのDirect reconnect、5 idle stepで追加 `0x30` なし、Direct input、明示neutral、final neutral、各run前後の完全bytes一致を確認。virtual transport 6 test、all-feature clippy `-D warnings`、rustfmt、diff checkが成功。refactor: file-backed profile fixtureをmodel generic化し、L/R共通のreuse helperへ集約 |
| refactor-done | T05 | red: `cargo test --all-features --example joycon_profile_hardware --locked` は runner target がなく失敗。green: model、reporting、pair/reconnect、timeout、run indexを明示し、adapter selector、profile path、raw profile、key materialを出力しない NDJSON runnerを追加した。Joy-Con L run 1はfresh Periodic Pairが5.808秒でReadyとなり、D-pad 4方向、L+ZL、SL+SR、left stick 4方向、中立close、adapter reopen、新規profileのmodel検査を完了した。run 2は同じprofileのDirect reconnectが3.336秒でReadyとなり、Ready後idleのuser input 0件、各操作のpress/release 2件、明示neutral、close、adapter reopen、profile完全一致を確認した。ユーザは両runで全入力のUI反映と残留入力なしを報告した。runner unit 3 test、rustfmt、secret-free evidence検査が成功。refactor: L/RとPeriodic/Directのdispatch、machine/UI record、profile pre/postflight、操作ごとの受理数と接続維持検査を共通化 |
| refactor-done | T06 | red: Joy-Con R fresh Pairのrun 3/6は120秒でtimeoutしたがschema v2 bondは保存され、run 7のPeriodic reconnectは通常入力を1,732件受理しながら30秒でReadyにならなかった。statusはreply 14件、最後のsubcommand `0x22`、worker failureなしを示した。pinned Python `0.6.0` / commit `84d2723...` の実装と過去のJoy-Con R実機記録を照合し、RustだけがNFC/IR MCU state `0x22`を未対応と特定した。`nfc_ir_mcu_state_reply_accepts_the_python_supported_modes` は最初、stateless replyが`None`のため失敗した。green: mode `0x00`–`0x02`をACK `0x80`、追加dataなしで受け、payload欠落と未知modeをtyped `ProtocolError`にした。targeted subcommand 14 test、facade routing、all-feature lib clippy `-D warnings`が成功した。修正後run 13は既存bondのPeriodic reconnectが2.303秒でReadyとなり、3秒idleと全入力の間も接続を維持し、中立close、adapter reopen、profile完全一致を完了した。run 14は同じprofileのDirect reconnectが2.027秒でReady、2回の500 ms idleでuser input 0件、各操作2 report、全入力のUI反映、残留入力なしを確認した。Switch登録解除後のrun 15はfresh Periodic Pairが10.588秒でReadyとなり、全入力、中立close、adapter reopen、新規profileと反対側model拒否を完了し、ユーザはJoy-Con R登録と残留入力なしを確認した。500 msのX押下でremote reason `0x13`の切断を再現し、200 msではrun 13–15が成功したためface buttonだけ200 msへ短縮したが、Switch UIの長押し終了条件だったかは未検証。refactor: stateless replyのerror検査helper、接続喪失/入力未受理の即時failure、Periodicのpress/releaseとDirectのtapをrunner内で分離 |
| refactor-skipped | T07 | red: READMEはM6まで、crate rustdocはPro実機だけ、hardware matrixはJoy-Con L/Rを未検証としており、M7の達成範囲とbeta.1の未達項目を判別できなかった。completion test初回は、`0x22`を未対応caseとして固定した既存facade testが`InvalidNfcIrMcuState`を受けて失敗した。green: unsupported caseを`0x23`へ移し、README、crate rustdoc、hardware matrixをJoy-Con L/R Periodic/Direct、runner手順、UI/machine境界へ更新した。beta.1はJoy-Con対象だけ達成し、Linux、dependency/license inventory、公開API review、semver freeze候補は未達と明記した。Rust 1.87 all-feature libraryは294 passed / 2 ignored、default/no-defaultは各250 passed / 1 ignoredで、integrationとdoc testも成功した。all-target/all-feature check、stable all/default clippy `-D warnings`、Rust 1.87 all/default/no-default build、all-feature rustdoc `-D warnings`、rustfmt、diff checkが成功した。仮テキストとevidence秘密情報の`rg`は該当なし。文書・test期待値の更新でproduction behaviorを追加変更していないためrefactorは省略 |

## 7. 対象ファイル

- `tests/fixtures/python-v0.6.0/profile/`
- `tests/profile_compat.rs`
- `src/controller/runtime_tests.rs`
- `src/protocol/`
- `src/runtime/transport/virtual_tests.rs`
- `examples/`
- `README.md`、crate rustdoc
- `spec/initial/testing.md`
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

## 10. beta.1 criteria note

M7は`0.1.0-beta.1`追加対象のうち、Joy-Con L/R Periodic/Direct、model-specific button/state、
model固有wire/identity、profile再利用、Windows実機経路を満たした。common IMU conversionと
wire parityは既存fixtureで維持しているが、M8のIMU実機・診断・probeは本作業では実行していない。

`0.1.0-beta.1`全体のrelease-ready判定ではない。limited Linux bring-up、dependency/license inventory、
公開API reviewとsemver freeze候補はM9まで未達である。Windows実機も1台のSwitch 2、ユーザ報告
system version 22.5.0、CSR8510 A10、WinUSBに限定され、長時間・反復成功率は測定していない。

## 11. Self-review

### 11.1 Work

- spec: `unit_008`のJoy-Con L/R Periodic/Direct、profile再利用、実機経路
- intent delta: Joy-Con L/R profile fixture、左右固有fake/virtual input、同一profileのreporting再利用、
  Joy-Con実機runner、NFC/IR MCU state `0x22`互換reply、公開文書とhardware matrix
- non-goals: `JoyConPair`、左右同時接続、M8 IMU/diagnostics/probe、M9 portability/release、upstream PR

### 11.2 Findings and residual risk

| severity | finding | evidence | disposition |
|---|---|---|---|
| medium | `0.1.0-beta.1`全体のLinux、dependency/license inventory、公開API review、semver freeze候補は未達 | roadmap 2.3、M9 | M7完了とrelease-ready判定を分離し、M9まで公開しない |
| medium | `bumble` featureはpublic forkの固定SHAに依存する | `Cargo.lock`、固定revision | fork branch push以外へ権限を広げず、upstream PR / issueを作成しない |
| low | Switch system version 22.5.0はユーザ報告で、runnerは機械検出していない | machine evidenceの`switch_system_version_machine_verified: false` | README、crate rustdoc、matrixでユーザ報告と明記 |
| low | Joy-Con Rの500 ms X押下時にremote reason `0x13`で切断したが、Switch UIの終了条件だったかは未検証 | run 12診断、200 msのrun 13–15成功 | face buttonを200 msに限定し、原因を断定しない |
| low | 実機はWindows 11、CSR8510 A10、WinUSB、Switch 2各1環境で、長時間・反復成功率を測っていない | T05/T06 evidence | M8/M9へ先送りし、成功runを信頼性指標にしない |

critical/high findingはない。private protocol handlerの追加で公開API型は変更していない。model宣言、
左右stick capability、profile model、Periodic/Directの型境界を維持し、`unsafe`、feature、filesystem schema、
key出力を追加していない。

### 11.3 Gates

| gate | result | evidence |
|---|---|---|
| Requirements | pass | roadmap M7、initial API/type/architecture/testing、pinned Python 0.6.0、固定Bumble revisionを照合 |
| Design | pass | `Controller<M,R>`、model固有mapping、別profile、reporting policyを維持。`JoyConPair`なし |
| Test | pass | Rust 1.87 all-feature 294 passed / 2 ignored、default/no-default各250 passed / 1 ignored、integration/doc test成功 |
| Static | pass | stable all/default clippy `-D warnings`、rustfmt、rustdoc `-D warnings`、仮テキスト/秘密情報該当なし |
| Package | build pass / package not applicable | Rust 1.87 all/default/no-default build成功。release対象外のため`cargo package`未実行 |
| Integration Review | pass | README、crate rustdoc、initial hardware matrix、work-unit、evidence、固定fork SHAを照合 |
| Hardware | pass within M7 scope | L run 1/2、R run 13–15。UI observationはmachine NDJSONと別record |

T07ではT05/T06の保存済み実機証跡を参照し、hardwareを再実行していない。Python writer、network、
Linux、macOS、cross compile、long-run、package、publishは対象外。Bumble forkへの追加変更とbranch pushは
不要で、upstream PR / issueは作成していない。

## 12. 完了チェックリスト

- [x] T01–T07 が個別 commit で完了している
- [x] Joy-Con L supported集合にAを含めない
- [x] Joy-Con R supported集合にD-padを含めない
- [x] stick capabilityとneutral opposite sideがmodelに一致する
- [x] SL/SR mappingがL/RそれぞれのPython fixtureと一致する
- [x] device info、SPI colors、SDP identityがmodelに一致する
- [x] L/R×Periodic/Directがfake/virtual Readyとtyped inputを通る
- [x] cross-model profileをadapter open前に拒否する
- [x] L/Rそれぞれで同一profileをPeriodic/Direct利用する
- [x] Direct idleで周期user input reportを送らない
- [x] L/R別の実機machine evidenceとUI observationを記録する
- [x] key material、raw profile、secretがerror、trace、evidenceに残らない
- [x] `JoyConPair`を追加していない
- [x] beta.1 criteria、未実行条件、residual riskを記録する
- [x] upstream Bumble PR / issueを作成していない
- [x] self-reviewとcompletion gateを通す
- [x] `spec/complete/unit_008/`へ移動する
