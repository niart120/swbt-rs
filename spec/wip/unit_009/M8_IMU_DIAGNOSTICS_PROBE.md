# M8 IMU、diagnostics、probe

- 状態: **着手中**
- milestone: M8
- branch: `feat/unit-009-m8-imu-diagnostics-probe`
- 正本:
  - `spec/initial/roadmap.md` 2、11
  - `spec/initial/api.md` 6、14、15
  - `spec/initial/architecture.md` 5、9、13、18、21
  - `spec/initial/testing.md` 4、5、7、8、10、12、13
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

既存の共通 IMU 型と protocol encoder を Pro Controller、Joy-Con L、Joy-Con R の固定 fixture で
検査し、mode 変更と送信受理の順序を確定する。runtime の内部状態を version 付きの安定した
diagnostics event へ投影し、接続調査に必要な lifecycle、session、subcommand、受理数、切断理由を、
秘密情報や機器識別子を含めずに保存できるようにする。

`swbt-probe` は adapter 列挙、open、pair、reconnect、profile inspect/verify を提供する。CLI 境界で
`ControllerKind` を parse し、Pro/Joy-Con L/Joy-Con R の `run::<M>()` へ明示分岐する。probe の成功は
transport と runtime の観測結果であり、Switch UI 反映の証拠として扱わない。

## 2. 現状と Intent Delta

M8 着手時点で次は実装・検査済みである。

- 公開 `ImuFrame` / `ImuSamples`、raw/物理量変換、丸めと範囲検査
- standard mode と quaternion mode `0x02`–`0x05`、same-mode reset、session reset
- report sender の timer/IMU state acceptance commit と、`0x40` ACK 後の mode 反映
- Pro の non-neutral IMU Python fixture と IMU unit test
- nonblocking `GamepadStatus`、lifecycle、report mode、受理数、最終 subcommand、切断理由、
  sanitized worker failure
- runtime 内部の単調増加 session ID、session ごとの observed subcommand 集合、stale event 破棄
- 公開 `AdapterInfo` と `list_adapters()`、typed profile の JSON read/write

この既存 Green を新規実装として数えず、次の差分だけを TDD item にする。

| 境界 | M8 着手時 | M8 完了条件 |
|---|---|---|
| IMU fixture | non-neutral は Pro のみ | 3 model の standard/quaternion golden と mode/ACK ordering を固定 fixture で検査 |
| diagnostics | snapshot と内部 counter はあるが安定 event schema はない | version 付き event 名・field・意味を固定し、runtime session から発行 |
| redaction | 個別 error/status は sanitized | trace 全体で path、address、key、serial、raw packet、source chain が出ないことを検査 |
| dynamic boundary | typed controller と typed profile だけ | controller kind、profile metadata、unsupported button を model ごとに安全に判定 |
| probe | binary target なし | 6 command、終了コード、safe NDJSON、typed dispatch、利用手順を実装 |
| hardware | M5/M7 の個別 runner 証跡 | probe 経由の IMU/diagnostics trace、期限付き long-run、cleanup を記録 |

### 2.1 release target と milestone 順序

初期 roadmap の主依存列は M7 の後に M8 を置く一方、release target は diagnostics/probe を
`0.1.0-alpha.2`、Joy-Con を `0.1.0-beta.1` に置いている。実装順は主依存列どおり M7→M8 とし、
過去の alpha.2 相当機能を後から release したとは扱わない。M8 完了時には機能集合ごとの達成を記録し、
実際に採用する version と publish 可否は M9 の release gate で決める。

## 3. 対象範囲

- 3 model の standard/quaternion IMU encoder fixture と ordering test
- version 1 の安定 diagnostics event contract
- lifecycle、session、report mode、subcommand、counter、disconnect、failure の event 投影
- controller/reporting/package version/OS/architecture からなる安全な実行環境情報
- dynamic controller kind と model-specific button の検証
- 秘密値を返さない profile summary/verify 境界
- `swbt-probe` の6 command、usage、終了コード、NDJSON trace
- Pro Controller を使う Windows 実機 IMU/diagnostics/long-run 証跡
- README、crate rustdoc、probe help、hardware matrix、beta.1 criteria note

## 4. 対象外

- IMU fusion、姿勢推定、補間、calibration 推定、sensor polling API
- `GamepadStatus` field の追加や既存公開型の破壊的変更
- raw HID/HCI packet trace と raw profile dump
- profile の編集、移行、秘密値表示
- 自動再接続、daemon、対話 UI、複数 controller 同時操作
- Joy-Con 左右同時接続、`JoyConPair`
- Linux/macOS 実機、cross compile、package、version 更新、tag、publish: M9
- Python repository の変更
- Bumble upstream PR / issue

## 5. 振る舞い仕様

### 5.1 IMU と mode ordering

- `ImuFrame` / `ImuSamples` は model 非依存の公開物理量を維持する
- encoder は `M::SPEC.protocol` と session の IMU mode から 36-byte block と次状態を計算する
- Pro/Joy-Con L/Joy-Con R の standard と quaternion mode は pinned Python fixture と一致する
- 同じ quaternion mode の再指定は encoding state を初期状態へ戻す
- `0x40` reply は旧 committed mode の input prefix を使い、ACK の transport 受理後だけ新 mode を commit する
- input report の transport reject では timer と IMU encoding state を進めない
- new session は IMU mode と encoding state を既定値へ戻す

### 5.2 安定 diagnostics event

安定 event は `tracing` target `swbt::diagnostics` だけに出す。event record は次を契約とする。

- `schema`: `swbt.diagnostics`
- `schema_version`: `1`
- `event`: `environment`、`session_started`、`lifecycle_changed`、`subcommand_observed`、
  `report_tx_accepted`、`reply_tx_accepted`、`session_ended`、`worker_failed`、
  `unsupported_button` のいずれか
- 共通 field: `controller_kind`、`reporting_kind`。runtime event は `session_id` も持つ
- event 固有 field: lifecycle、report mode、subcommand ID、累積受理数、disconnect reason、
  failure category、button kind

`environment` は session 開始前に発行し、package version、target OS、target architecture を持つ。
event field は machine-readable な固定名と値を使い、人向け error message を契約にしない。`session_id` は
process 内の単調増加値で、Bluetooth address や profile identity から生成しない。counter event は受理ごとに
記録できるが、入力内容や raw bytes は記録しない。`GamepadStatus` は既存 field を維持し、安定 event と同じ
internal status projection から値を得る。

### 5.3 environment と redaction

probe 開始 record に package version、target OS、target architecture、controller kind、reporting kind を含める。
次は stdout、stderr、trace、error の安定 field に含めない。

- adapter selector、USB bus/address/port、USB serial
- profile path、profile JSON、peer/local Bluetooth address、link key
- raw HID/HCI packet、入力した stick/IMU の値
- Bumble error の source chain と debug 表現

trace writer は `swbt::diagnostics` target だけを収集する。既存 transport debug event を混在させない。
v1 は raw packet opt-in を実装しない。NDJSON は各行を独立した JSON object とし、中断後も完了行を読める。

### 5.4 dynamic controller/profile 境界

CLI の controller 名は `pro`、`joycon-l`、`joycon-r` だけを受け付ける。未知値を Pro へ fallback しない。
parse 後は入口の `match` で `run::<ProController>()`、`run::<JoyConL>()`、`run::<JoyConR>()` へ分岐し、
core runtime に untyped controller state を持ち込まない。

動的な `ButtonKind` は対象 model の typed button へ変換できない場合、`UnsupportedInput` と
`unsupported_button` event を返す。profile inspect/verify は schema と controller kind を検査するが、
秘密値を保持する document 自体を公開しない。summary が返してよい値は schema version、controller kind、
identity kind、namespace 数、bond 数に限定する。明示 Bluetooth address は表示しない。

### 5.5 probe command と終了コード

```text
swbt-probe adapters
swbt-probe open --adapter usb:0
swbt-probe pair --controller pro --profile path --trace trace.jsonl
swbt-probe reconnect --controller pro --profile path --trace trace.jsonl
swbt-probe profile inspect path
swbt-probe profile verify path
```

- 成功は終了コード `0`
- adapter/controller/profile/runtime/trace の操作失敗は終了コード `1`
- 未知 command、欠落/重複 option、未知 controller/mode は終了コード `2`
- stdout は成功時の safe NDJSON、stderr は短い分類済み error とし、path/source chain を出さない
- `adapters` は claim/open せず候補を列挙し、selector や serial を出力しない
- `open` は指定 adapter の open/close と resource cleanup を検査する
- `pair` は Periodic、`reconnect` は `--reporting periodic|direct` を受け、既定は Periodic とする
- `pair` は既存 profile を上書きせず、`reconnect` は profile 不一致時に adapter を開かない
- `--trace` は create-new とし、既存 trace を上書きしない
- `bumble`/probe feature なしの通常 library build と test を維持する

### 5.6 hardware evidence

Windows 11、CSR8510 A10、WinUSB、Switch 2 system version `22.5.0`（ユーザ報告）で Pro Controller の
stored-key reconnect を行う。runner/probe は timeout と run index を明示し、次を別 record で残す。

- machine: Ready、IMU mode、non-neutral IMU report 受理、session/counter/subcommand/reason、neutral close、
  adapter reopen、profile bytes 不変、trace parse/redaction
- timing: 8 ms 周期の受理 timestamp、p50/p95/p99/max jitter、overrun、command/reply latency、
  idle CPU、shutdown latency
- UI: button/stick/IMU の観測と残留入力なし。machine evidence から推測しない

long-run は明示した固定時間で実行し、その一回を成功率や production reliability の証拠にしない。
hardware 実行前にユーザへ準備を依頼し、各 UI 観測を受け取るまで `ui_observed` を未確認とする。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-done | T01 3 model の standard/quaternion IMU golden と mode/ACK ordering を固定 fixture で検査する | new/regression | protocol/integration | 既存 common IMU を再利用 |
| refactor-done | T02 version 1 diagnostics event の名前、field、値、redaction を検査する | new | unit | raw packetなし |
| refactor-done | T03 runtime session から lifecycle/subcommand/counter/disconnect/failure event を発行する | new/regression | runtime | statusと同じ投影 |
| refactor-done | T04 profile JSON を動的に検査し、秘密値を含まない summary を返す | new | public boundary | cross-model情報を保持 |
| refactor-done | T05 probe の profile inspect/verify、usage、終了コード、safe output を検査する | new | CLI integration | hardwareを開かない |
| todo | T06 probe の adapters/open と feature-disabled build/error を検査する | new | CLI/package | fake境界を使う |
| todo | T07 pair/reconnect の3 model dispatch、reporting選択、unsupported button、fallback禁止を検査する | new | CLI/runtime | fake/virtual transport |
| todo | T08 trace writer が diagnostics target だけを有効な NDJSON へ保存し、秘密値を除外する | new | integration | create-new |
| todo | T09 Pro実機で IMU/diagnostics/long-run trace と期限付き cleanup を記録する | new | hardware | machine/UI分離 |
| todo | T10 completion gate、公開文書、criteria note、self-review を確定する | new | docs/package | releaseはしない |

### 6.1 TDD cycle evidence

各 item の red、green、refactor、targeted command、commit をここへ追記する。既存実装が追加 test を
最初から満たした場合は regression-green と記録し、失敗を捏造しない。hardware item は自動 test と
machine evidence を先に確定し、人手 UI 観測を別に追記する。

| phase | item | evidence |
|---|---|---|
| refactor-done | T01 | red: Rust fixture consumer に Joy-Con L/R の standard と quaternion `0x02`–`0x05` の10 caseを要求すると、固定fixtureがProの5 caseだけだったためcase総数45対55で失敗した。green: cleanなpinned Python 0.6.0 / revision `84d2723...` のgeneratorを3 modelへ一般化し、55 caseを再生成した。Rustは3 model×standard/quaternionをbyte-for-byte検査して6 passed、IMU encoder 8、全model mode/same-mode reset 11、reply reject後のsession未commit/retry 1、fixture provenance 2が成功した。既存testにより`0x40` replyは旧prefixで作られ、transport受理後だけmodeをcommitすることも維持した。refactor: generatorのprofile反復とRust consumerのIMU suffix判定へ重複を集約。production code変更なし |
| refactor-done | T02 | red: version、target、9 event、field/value、禁止fieldを要求するunit testはevent module/型/定数が未定義のため`E0432`で失敗した。green: `swbt.diagnostics` schema version 1、`swbt::diagnostics` target、`environment`と8 runtime eventをclosed enumで定義した。controller/reporting/lifecycle/button/failureは固定文字列、session/counter/reasonは数値だけを受け、path/address/key/serial/raw packet/error source/messageをpayloadへ渡す入口を持たない。exact record/redaction/failure categoryの3 tests、default all-target clippy `-D warnings`、rustfmt、diff checkが成功。stable eventはdefault runtimeでも発行するため`tracing`を非optionalにした。refactor: 共通runtime contextとfield builderへschema/controller/reporting/sessionの重複を集約。T03 wiring前の未使用lintだけ理由付き`expect`とし、T03で除去する |
| refactor-done | T03 | red: fake Direct worker の session開始からReady、input、disconnectまでの安定event順とfieldを要求すると、inject可能なevent emitterが未定義のため`E0432`で失敗した。green: status projectionへ非0 session IDを渡し、session start、lifecycle、parse済みsubcommand、transport受理後のinput/reply累積数、disconnect reasonを同じstate更新点から発行した。実workerの10 event順、session/controller/reporting、counter、reasonを1 testで検査し、sourceに`T26_SECRET`を持つterminal transport failureは`transport` category、failed lifecycle/session end、既存sanitized statusだけを返す別testも成功した。callbackはstatus lock解放後に呼ぶ。`GamepadStatus` fieldは変更していない。refactor: module全体の一時dead-code expectationを除去し、status readerの`R` markerとpublisherの単相化context関数によりcontroller/reporting kindを`M`/`R`から導出した。全feature library 299 passed / 2 ignored、default/all-feature clippy `-D warnings`、rustfmt、diff checkが成功 |
| refactor-done | T04 | red: 外部crate視点で`ProfileSummary`、`ProfileIdentityKind`、`inspect_profile`を要求すると3 public itemが未定義のため`E0432`で失敗した。green: schema v2の完全validation後にschema version、controller kind、address-free identity kind、namespace/bond件数だけをコピーするnon-exhaustive summaryを追加した。raw JSON、unknown field、namespace/peer address、key、pathはsummaryに保持しない。valid local-address profile、malformed file、missing fileを2 integration testsで検査し、ErrorKindは`InvalidProfile`/`ProfileNotFound`、Display/Debugは秘密pathなしとなった。refactor: dynamic inspectionを`profile::summary`へ分離し、秘密を保持する`ProfileDocument`はcrate-privateのままにした。公開fieldなし、getterは値返し、filesystem error sourceは既存chainへ保持する。all-feature test 2、default/no-default check、all-feature clippy `-D warnings`、all-feature rustdoc、rustfmt、diff checkが成功 |
| refactor-done | T05 | red: `swbt-probe` の profile inspect/verify、usage、終了コード、秘密値非表示を要求するCLI integration testを追加すると、Cargo packageに`probe` featureがなくcommandを実行できなかった。green: `probe` featureでだけ構築するbinaryを追加し、profile inspectはschema version、controller kind、identity kind、namespace/bond件数、verifyはcontroller kindとvalidだけをversion 1 NDJSONへ出力する。malformed/missing profileは分類済みerrorで終了1、欠落・過剰・未知引数はusage errorで終了2、helpは終了0とした。3 integration testsが成功し、profile path、Bluetooth address、link keyはstdout/stderrに出ない。featureなしでは同test targetが0件で成功し、binaryを通常buildへ含めない。refactor: parse、実行、record生成、writerを分離した。gateで検出した既存adapter selectorの不要なclosure参照は別commitで除去し、selector正常/異常2 testsとall-feature clippy `-D warnings`、rustfmt、diff checkが成功した。hardwareは開いていない |

## 7. 対象ファイル

- `src/input/imu.rs`
- `src/protocol/imu.rs`、`src/protocol/tests/imu.rs`
- `src/diagnostics.rs` または `src/diagnostics/`
- `src/runtime/status.rs`、`src/runtime/session.rs`、`src/runtime/connection.rs`
- `src/profile/`
- `src/bin/swbt-probe.rs` と probe 専用内部 module
- `tests/fixtures/python-v0.6.0/`、`tests/imu_contract.rs`
- probe/diagnostics の unit・integration test
- `Cargo.toml`、`Cargo.lock`
- README、crate rustdoc、`spec/initial/testing.md`
- `evidence/` の secret-free 実機証跡
- 本作業仕様

公開 API の追加は profile の safe inspection に必要な最小境界だけとする。公開型を追加する場合は
`rust-api-boundary-review` と `rustdoc-style` で error、所有権、非網羅性、example を検査する。

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

probe feature の組合せは binary 有無と通常 library build の双方を検査する。Python reader を使う場合は
version、revision、command を記録する。T09 以外の hardware、network、cross compile、package、publish は
実行しない。

## 9. 先送り事項

- Linux/macOS adapter lifecycle と実機: M9
- package、version、tag、GitHub release、crates.io publish: M9
- raw packet trace: v1 diagnostics の対象外。必要性と明示 opt-in を別 unit で判断
- IMU fusion/姿勢推定: core protocol の対象外
- production reliability と反復成功率: 単発 long-run では判定しない
- Bumble upstream contribution: この作業では実施しない

## 10. criteria note

M8 完了時に、旧 `0.1.0-alpha.2` 対象の diagnostics/probe と、`0.1.0-beta.1` 対象の common IMU、
Joy-Con を含む model fixture が揃う。ただし過去版を遡って公開せず、M8 単独を release-ready としない。
limited Linux bring-up、dependency/license inventory、公開 API review、semver freeze 候補、package/publish は
M9 の完了条件に残す。

## 11. 完了チェックリスト

- [ ] T01–T10 が個別 commit で完了している
- [ ] 3 model の standard/quaternion golden が pinned Python fixture と一致する
- [ ] same-mode reset、ACK ordering、acceptance commit、session reset を検査する
- [ ] diagnostics schema/version/event/field の契約を test と docs に固定する
- [ ] status と event が同じ session/counter/lifecycle 状態から投影される
- [ ] dynamic controller/profile/button 境界で fallback しない
- [ ] probe の6 command、終了コード、feature 組合せを検査する
- [ ] trace が有効な NDJSON で、秘密値、path、address、serial、raw packetを含まない
- [ ] Pro実機 IMU/diagnostics/long-run と UI 観測を別々に記録する
- [ ] default/no-default/all-feature の build/test/clippy/doc が成功する
- [ ] README、crate rustdoc、hardware matrix、criteria note が実装と一致する
- [ ] M9 の portability/release 項目に着手していない
- [ ] Bumble upstream PR / issue を作成していない
