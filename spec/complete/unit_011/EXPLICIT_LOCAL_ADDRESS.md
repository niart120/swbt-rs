# Explicit local address

- 状態: **着手中**
- milestone: explicit local address milestone
- branch: `feat/unit-011-explicit-local-address`
- 正本:
  - `spec/initial/roadmap.md` 13
  - `spec/initial/api.md` 3.4、12
  - `spec/initial/architecture.md` 8、15、16、21
  - `spec/initial/testing.md` 6.2、11、12、13
  - `spec/initial/migration-strategy.md` 5、16、17

## 1. 目的

`ProfileIdentity::LocalAddress` を CSR8510 A10 の volatile BD_ADDR 書換えへ接続し、profile に保存した
identity と controller が実際に報告する local address が一致した場合だけ pairing / reconnect を開始する。
書換え途中の失敗では再試行可能な通常エラーと、物理 power cycle が必要な不確定状態を型付きで区別する。

この unit は `swbt-python` の CSR 実機探索と local-address profile の確定済み挙動を Rust へ移植する。
Rust 側の公開 `LocalAddress` は個別かつ locally administered な address だけを受け付けるため、Python 版の
characterization で使った未割当 universal dummy address は製品経路へ移植しない。

## 2. 着手時点

### 2.1 実装済み

- `LocalAddress` は `XX:XX:XX:XX:XX:XX` を parse し、multicast、universal、予約 inquiry LAP を拒否する
- `ProfileIdentity::LocalAddress` と schema v2 の `exp-local-address` 読込みは存在する
- local-address profile の `Debug` と profile summary は address を表示しない
- pairing key store の namespace は controller が報告した local address から決まる
- Bumble fork revision `b8c7cd625bc2ac2f58a4beb4ade1264426969819` は generic HCI command と Vendor Event の
  decode を持つ
- normal Bumble session は connectable / discoverable を無効にしたまま HCI 初期化し、pairing command 後にだけ
  Switch-facing 動作へ進む

### 2.2 未実装

- create-profile は local address を `UnsupportedCapability` で拒否する
- local-address empty envelope を生成する経路がない
- typed profile から runtime identity を取得する crate-private 境界がない
- CSR BCCMD の build / parse、volatile write、warm reset、USB 再列挙、read-back がない
- Bumble `ExternalHost` は Vendor Event を device event として保持するが、command と一致する Vendor Event を期限付きで
  待つ公開 API と、応答を待たず warm reset を送る公開 API を持たない
- write 開始後の不確定状態を表す public error kind がない
- probe、公開文書、実機 matrix に explicit local address がない

## 3. 対象範囲

- CSR BCCMD opcode `0xFC00` の PSKEY_BDADDR GETREQ / volatile SETREQ / warm reset byte contract
- request type、sequence number、VARID、status を使う Vendor Event response matching
- CSR company identifier `10` の capability gate
- already-active fast path、volatile write、warm reset、USB 再列挙、read-back
- write 開始後の失敗を `AdapterIdentityRecoveryRequired` として分類する recovery boundary
- profile 作成前検査、local-address empty envelope、typed reopen、runtime identity projection
- normal Bumble power-on 後かつ pairing / advertising 前の expected-address guard
- actual local address と一致する key-store namespace
- create-profile と existing profile build/open/reconnect の共通 identity preparation
- `swbt-probe pair` から local address を明示する安全な実機入口
- Python schema v2 の `exp-local-address` と namespace 互換 fixture
- Pro / Joy-Con L / Joy-Con R、Periodic / Direct に共通する model-independent unit / virtual coverage
- Windows 11、CSR8510 A10、WinUSB、Switch 2 22.5.0 の期限付き実機検証
- duplicate address、power cycle、失敗時復旧、対応範囲の公開文書
- Bumble public fork branch の最小 API 追加と固定 revision 更新

## 4. 対象外

- CSR persistent store / EEPROM への書込み
- CSR 以外の vendor command と adapter
- 任意の universal address、multicast address、予約 inquiry LAP
- 複数 adapter / 複数 controller の同時 identity 切替
- duplicate address の自動検出
- write 開始後の自動 restore。状態が確定できないため物理 power cycle を要求する
- macOS、Linux、別 CSR 個体での実機保証
- raw address、link key、raw HCI packet を diagnostics trace に出す機能
- upstream Bumble issue / PR。ユーザ指示により作成しない
- M9 の crates.io dependency namespace 解決、crate publish、tag、release

## 5. 振る舞い仕様

### 5.1 profile と事前検査

- `ProfileIdentity::LocalAddress(address)` は `bumble` feature が利用可能な場合だけ create-profile capability gate を通る
- path 必須、identity、target existence の検査順を維持する
- target absent の場合、`exp-local-address` と uppercase address を持つ valid schema v2 empty envelope を adapter open 前に
  create-new する
- concurrent target creation は `ProfileAlreadyExists` とし、既存 file を置換しない
- typed reopen は persisted identity を runtime config へ渡す。caller が渡した一時値を正本にしない
- feature なしでは filesystem / adapter side effect 前に `UnsupportedCapability` を返す

### 5.2 pure CSR contract

- PSKEY_BDADDR は CSR BCCMD channel `0xC2`、VARID `0x7003`、PSKEY `0x0001` を使う
- GETREQ は type `0x0000`、SETREQ は `0x0002`、response は双方とも GETRESP `0x0001` とする
- volatile store selector は `0x0008` だけを生成し、persistent selector を production API に持たせない
- response は channel、response type、sequence number、VARID が request と一致する場合だけ受理する
- status word が非 0、短い response、不一致 response は成功にしない
- address packing と warm reset bytes は Python 基準断面と BlueZ 由来 fixture に一致させる

### 5.3 identity preparation

- adapter open 後、HCI Reset を送らず local version と current BD_ADDR を読む
- company identifier が `10` でなければ write 前に `UnsupportedCapability` を返す
- current address が target と一致すれば write / warm reset / reopen を行わない
- 不一致なら volatile SETREQ の成功 response を待ち、CSR warm reset の同期転送と session close を試行する
- warm reset の同期転送結果と最初の session close 結果は、reset に伴う USB 切断で失敗し得るため保留し、再列挙と
  read-back で決着させる
- selector を使って期限まで再 open を繰り返し、HCI Reset なしで BD_ADDR を read-back する
- read-back が target と一致した場合だけ normal Bumble session の初期化へ進む
- write を送る前の open/read/capability failure は通常の分類済み error とする
- SETREQ の send / response、再列挙、read-back、read-back session close、不一致は
  `ErrorKind::AdapterIdentityRecoveryRequired` とする。利用者は dongle を抜き差しして元 identity を確認するまで再試行しない

### 5.4 expected-address guard と runtime

- normal Bumble session は既存どおり HCI Reset と capability read を行う
- local-address profile では normal power-on が返した local address を persisted target と照合する
- 不一致なら connectable / discoverable、pairing、reconnect、worker spawn へ進まない
- identity preparation で write が発生した後の guard 不一致は physical recovery required とする
- adapter-default profile は現行 open path と error semantics を維持する
- key-store namespace は guard 済みの actual local address と一致する
- local-address existing profile の Periodic / Direct reconnect は同じ preparation と guard を使う

### 5.5 error と秘密情報

- `AdapterIdentityRecoveryRequired` は caller が分岐できる public `ErrorKind` とする
- public message は USB dongle の物理 power cycle と再確認が必要なことを示す
- `Display`、`Debug`、source chain、related error、diagnostics、probe error record に target/current address、selector、path、
  raw packet、key を含めない
- internal stage は closed enum とし、人向け文字列を分岐契約にしない

### 5.6 probe と実機

- `swbt-probe pair` は local address の明示指定を受け、指定なしは adapter-default のままにする
- local address は stdout / stderr / trace に出さず、identity kind だけを記録する
- 実機 run は fresh local-address profile で Pro Periodic pairing、保存鍵 reconnect、Direct reconnect、neutral close、
  adapter reopen、profile bytes / namespace を確認する
- Switch UI 観測と machine evidence を分離する
- 書換えを伴う各実機 run の終了後は dongle を物理 power cycle し、adapter-default address への復帰を read-only で確認する
- failure injection または実機 failure が write 後に起きた場合、その run を中止し、power cycle と元 address 復帰確認まで
  次の write を行わない

### 5.7 duplicate address guidance

- 利用者自身が管理する locally administered address を使い、同じ radio 範囲で重複させない
- 同じ address を複数 dongle で同時に使わない
- profile と local address を一対一で管理し、別 identity の key namespace を混在させない
- profile 削除は dongle の volatile identity を復元しない。復元は物理 power cycle と read-back で確認する

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | T01 CSR GETREQ / volatile SETREQ / warm reset bytes、address decode、response matching、status failure を固定 fixture で検査する | new/characterization | pure unit | adapter I/O なし |
| refactor-done | T02 Bumble `ExternalHost` が generic command に一致する Vendor Event を期限付きで返し、warm reset command を応答待ちなしで送れることを fork 側 test で固定する | new | dependency unit | public fork branch。upstream PR なし |
| refactor-done | T03 identity preparation の already-active、rewrite→re-enumerate→read-back、非 CSR、write 前 failure、write 後 recovery-required を fake port で検査する | new/safety | transport unit | closed stage、bounded retry |
| refactor-done | T04 local-address empty profile を adapter open 前に create-new / typed reopen し、feature-disabled と concurrent target の順序を検査する | new/regression | profile/controller | persisted identity が正本 |
| refactor-done | T05 create/reconnect の normal power-on expected-address guard、actual namespace、adapter-default 無変更を fake / Bumble session で検査する | new/regression | runtime/transport | pairing可視化前に拒否 |
| refactor-skipped | T06 Python schema v2 local-address fixture と、3 model・Periodic/Direct の identity projection を検査する | compatibility | integration/virtual | address/key非表示 |
| refactor-done | T07 probe の local-address 引数、identity-kind output、usage/error/redaction と公開 docs を確定する | new/docs | CLI/public | raw addressを出力しない |
| refactor-done | T08 CSR8510 A10 で Pro Periodic pair/reconnect、Direct reconnect、neutral cleanup、power-cycle recovery を期限付きで記録する | hardware | Windows/Switch | machine/UI分離 |
| refactor-skipped | T09 completion gate、rustdoc/API review、dependency revision/source baseline、self-review を確定する | gate | package/docs | publish/tagなし |

各 item は red、green、必要な refactor を完了してから、その item だけを Conventional Commit で commit する。
Bumble fork 側 T02 は fork repository の専用 branch に独立 commit を作り、その commit SHA を本仕様と `Cargo.toml` に固定する。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| refactor-skipped | T01 | red: CSR codec の6 itemを参照する4 unit testsは全item未定義の `E0432` で失敗した。green: opcode `0xFC00`、channel `0xC2`、PSKEY_BDADDR `0x0001`、volatile selector `0x0008` の GETREQ / SETREQ / warm reset bytes、GETRESP matching、status/address decodeをpure moduleへ実装し、targeted default/all-feature testは各4 passed。malformed/non-CSR errorはpayloadを保持しないclosed enumとした。`cargo fmt --check`、all-target/all-feature clippy `-D warnings`、`git diff --check`も成功した。refactor: codecはI/Oなしの単一moduleに分離済みで、重複や不要な公開境界がないため追加変更を行わなかった |
| refactor-done | T02 | red: Bumble forkのhost testsは`send_vendor_command`と`send_command_without_response`が未定義の`E0599`で失敗した。green: `ExternalHost`へ期限付きVendor Event matchingと応答待ちなしcommand送信を追加し、一致しないpacketをpending順序へ戻した。matching、1 ms timeout、no-response sendの3 testsを追加し、`bumble-transport` host unit 32 passed、package lib clippy `-D warnings`、format、diff checkが成功した。refactor: 既存blocking commandを含むstate/device-flow/write検査をprivate `begin_direct_command`へ集約した。public fork branch `feat/external-host-vendor-command`、commit `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`へpushし、swbtの8 dependencyとlockを同SHAへ更新した。swbt all-feature testはlibrary 304 passed / 2 ignoredと全integration/doc test成功、all-target/all-feature clippy `-D warnings`も成功した。upstream PRは作成していない |
| refactor-done | T03 | red: identity preparationのbackend/session/options/outcome/error/stage/entrypointを参照する6 unit testsは全item未定義の`E0432`/`E0425`で失敗した。green: fake sessionだけでalready-active close、volatile write→warm reset→close→bounded re-enumeration→read-back、非CSR拒否、write前read failure、write call以後のrecovery-required、再列挙timeout/read-back不一致を検査し、default/all-feature targeted testは各6 passed。write前は`FailedBeforeWrite`、company identifier非10は`UnsupportedController`、write call直前以後は`RecoveryRequired`とし、stageをclosed enumで保持した。refactor: 初回greenではbackend errorをsourceに保持していたが、source chainから生messageへ到達できたため除去した。errorはkind/stageだけを保持し、Display/Debug/sourceのいずれにもaddress、packet、backend messageを出さないtestを追加した。all-target/all-feature clippy `-D warnings`、format、diff checkも成功した |
| refactor-done | T04 | red: local-address preflight planとtyped reopen済みprofileのidentityを参照するtestsは、両方の`identity()`が未定義の`E0599`で失敗した。green: create planへ要求identityを保持し、adapter open前のcreate-newでschema v2 `exp-local-address`とuppercase addressを保存し、読戻した`PairingProfile`の構造化identityをruntime configの正本とした。feature無効時はpath検査後かつNULを含むtargetのfilesystem inspection前に`UnsupportedCapability`、feature有効時のcreate-new競合は既存bytesを保持してruntime open前に`ProfileAlreadyExists`となることを検査した。targeted testsはdefault 10、feature無効順序1、all-features 13 passed。default/all-features full test、all-target/all-feature clippy `-D warnings`、format、diff checkも成功した。refactor: profile parse時にidentityを一度だけ型へ変換して保持し、summaryとruntime projectionが生JSON文字列を再解釈しない形へ統合した |
| refactor-done | T05 | red: runtime factory identity、準備付きBumble open、`IdentityMismatch`、`AdapterIdentityRecoveryRequired`を参照するtestsは、wrapperの`E0432`とmethod/error variantの`E0599`で失敗した。green: persisted profile identityをcreate/reconnect共通のruntime factoryからBumble portへ渡し、explicit identityだけがHCI Resetなしのversion/address read、CSR volatile SETREQ、warm reset、100 ms間隔・10 s期限の再列挙、read-backを経て通常初期化へ進む経路を実装した。通常初期化のaddress guardはkey-store設定、identity write、scan enable、worker生成より前に置いた。scripted Bumble testでadapter-defaultが1 openと既存command列を維持すること、rewrite成功が3 openとtarget namespaceへのlink key保存に至ること、write後guard不一致が公開`AdapterIdentityRecoveryRequired`、writeなし不一致が`IdentityMismatch`かつidentity writeなしになることを検査した。Bumble targetedは16 passed / 1 ignored、identity targetedは6 passed、default full testとall-features full test（library 318 passed / 2 ignoredを含む）、default/all-target/all-feature clippy `-D warnings`、format、diff checkが成功した。refactor: response timeoutをversion/address/vendor commandへ一貫して渡し、productionで使われたmodule-level dead-code例外を除去し、初期化traceからraw local address fieldを除去した |
| refactor-skipped | T06 | red: Python fixtureの期待caseを3件から6件へ先に拡張し、既存fixtureがadapter-default 3件だけだったためcase-set検査3件が失敗した。green: source commit `84d2723b127f70fc78e12f4496f5c40af0ccfb0a`のPython v0.6.0実装と一致する`exp-local-address` profileをPro / Joy-Con L / Joy-Con Rへ追加し、全6件のtyped JSON round-trip、未知field保持、local addressとlink keyのDebug非表示を検査した。Periodic / Directを組み合わせた6種のruntime configも同じtyped `ProfileIdentity::LocalAddress`を保持した。`cargo test --all-features --test profile_compat`は3 passed / 1 ignored、identity projection targeted testは1 passed。refactor: 製品コードはT05のmodel-independent projectionで既にgreenとなり、T06は互換fixtureとcharacterization testだけで完結したため追加の構造変更を行わなかった |
| refactor-done | T07 | red: explicit identityを持つprobe request testは`ConnectionRequest.identity`未定義の`E0609`/`E0560`で失敗し、write後recovery categoryも汎用`operation_failed`だった。green: `pair`だけが任意の`--local-address <XX:XX:XX:XX:XX:XX>`を受け、未指定時はadapter-default、reconnectでの指定・不正値・重複指定はusage errorとした。pairは指定identityをcreate-profileへ渡し、reconnectはvalidated profile summaryからidentity kindを取得する。成功NDJSONはaddress-freeな`identity_kind`、write後不確定は`adapter_identity_recovery_required`を出す。integration testは既存profileでhardware open前まで進み、stderr/traceへのraw address非表示を検査した。probe unit 11 passed、probe CLI 8 passed、all-features full test（library 319 passed / 2 ignoredを含む）、all-feature build、all-target/all-feature clippy `-D warnings`、format、diff checkが成功した。README、platform support、troubleshootingにCSR8510 A10限定、profile/address/dongleの一対一管理、再試行禁止とphysical power cycleを記載した。refactor: production successで必ず確定するidentity kindをoptional evidenceから必須fieldへ狭めた |
| refactor-done | T08 | red 1: 最初のfresh Pro pairはSETREQ後約1秒で`adapter_identity_recovery_required`となり、fake testでreset後の初回session close failureを`RecoveryRequired / Close`として再現した。green 1: 初回close結果を再列挙・read-backで決着するよう変更し、identity 7 testsが成功したが、2回目の実機pairも同じ公開errorで停止した。ignored `adapter-tests`診断で内部stageを秘密値なしに限定し、`WarmReset`を特定した。red 2: warm reset同期transfer failure後にtarget read-backできるtestは`RecoveryRequired / WarmReset`で失敗した。green 2: 同期transferと初回closeのold-handle結果を保留し、bounded再列挙・CSR metadata・exact target read-back・read-back session closeを必須にしてidentity 8 testsを成功させた。再列挙timeoutと旧address read-backが引き続きrecovery-requiredになることも固定した。実機identity-only runは`Rewritten`、fresh Periodic pairはbond/namespace各1・A反映・neutral close、保存鍵Periodic/Direct reconnectはprofile bytes不変・A反映・neutral closeで成功した。各write後にphysical power cycleとprivate adapter-default baseline一致を確認し、最終的にbaseline、profile、比較copy、3 traceを`target/`から削除した。`adapter-tests`を含むclippy `-D warnings`、format、diff checkも成功した。refactor: 実機診断を製品の公開error/traceへ混ぜず、ignored test helperへ分離し、保持evidenceをclosed machine resultとUI観測だけに限定した |
| refactor-skipped | T09 | completion: Rust 1.87.0 all-target/all-feature check、stable default/all-feature testとbuild、all-target/all-feature clippy `-D warnings`、all-feature rustdoc `-D warnings`、rustfmt、diff checkが成功した。default libは268 passed / 1 ignored、all-feature libは321 passed / 4 ignoredで、対応するintegration、probe、example、doc testも成功した。公開API reviewではBumble型を露出せず、`ProfileIdentity::LocalAddress`、`CreateProfileOptions`、typed `AdapterIdentityRecoveryRequired`、feature-disabled順序、秘密値非保持を維持した。rustdocへCSR8510 A10 / WinUSB、volatile、profile/address/adapter一対一、physical recovery条件を補った。Cargo metadataはBumble 22 packageがpublic fork `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`の単一sourceであることを示し、remote branch headも同SHA、forkはpublic、upstream PRは0件だった。source baseline、README、platform supportの旧candidate SHAを修正し、過去hardware evidenceの当時SHAは変更していない。self-reviewでcritical/high findingはなく、single hardware environment、固定fork、crates.io namespaceをresidual riskとして残した。package、publish、tag、GitHub release、Linux/macOS実機、cross compile、Miriは対象外。completion gateと文書/API整合のitemで製品構造を変える必要がなかったため追加refactorを行わなかった |

## 7. 対象ファイル

- `Cargo.toml`
- `Cargo.lock`
- `src/error.rs`
- `src/profile/identity.rs`
- `src/profile/document.rs`
- `src/controller/create.rs`
- `src/controller/config.rs`
- `src/controller/runtime.rs`
- `src/runtime/transport/csr.rs`
- `src/runtime/transport/identity.rs`
- `src/runtime/transport/bumble.rs`
- `src/runtime/transport/mod.rs`
- `src/bin/swbt-probe.rs`
- `tests/`
- `docs/platform-support.md`
- `docs/troubleshooting.md`
- `README.md`（入口の変更が必要な場合だけ）
- `spec/complete/unit_011/evidence/`
- Bumble fork の `bumble-transport/src/host.rs` と対応 test

実装中に責務境界が確定した場合は filename を調整し、この一覧へ反映する。

## 8. 検証

TDD item ごとの targeted test に加え、完了時に変更範囲に応じて次を実行する。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --all-features
cargo build
cargo build --all-features
cargo doc --all-features --no-deps
git diff --check
```

Bumble fork は変更 package の unit test、`cargo fmt --check`、対象 package の clippy を実行する。hardware test は
固定 timeout、adapter、driver、console version、run index、cleanup、power-cycle recovery を evidence に残す。

### 8.1 完了結果

2026-08-01 JST に次を実行し、すべて成功した。

- `cargo +1.87.0 check --all-targets --all-features --locked`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo test --locked`
- `cargo test --all-targets --all-features --locked`
- `cargo build --locked`
- `cargo build --all-features --locked`
- `$env:RUSTDOCFLAGS="-D warnings"; cargo doc --all-features --no-deps --locked`
- `cargo fmt --check`
- `git diff --check`

default lib は 268 passed / 1 ignored、all-feature lib は 321 passed / 4 ignored で、対応する integration、
probe、example、doc test も成功した。実機を要求する ignored test は、T08 の明示手順で対象2件だけを実行した。
package、publish、tag、GitHub release、Linux/macOS実機、cross compile、Miriはunit_011の対象外であり未実行とした。

### 8.2 API、依存、文書 review

- 公開境界は `LocalAddress`、`ProfileIdentity`、`CreateProfileOptions`、`ErrorKind` に閉じ、Bumble型を露出しない
- local-address createは`bumble` feature無効時にtarget inspectionとadapter I/Oより前で拒否する
- write後に状態を確定できない失敗は`AdapterIdentityRecoveryRequired`で分岐でき、Display、Debug、source、traceに
  address、selector、path、packet、key、backend messageを出さない
- rustdocはCSR8510 A10 / WinUSB、volatile identity、profile/address/adapter一対一、physical recovery条件を説明する
- Cargo metadata上のBumble 22 packageはpublic fork `cb55e2d98dc7b7b0227c43772c9ae184034dd9a1`の単一sourceである
- remote `feat/external-host-vendor-command` headは同SHA、forkはpublic、Bumble upstream PRは0件である
- candidate source baseline、README、platform supportを現行SHAへ合わせ、過去evidenceの当時SHAは維持した

### 8.3 Self-review

| severity | finding | evidence | disposition |
|---|---|---|---|
| medium | `bumble` featureはpublic forkの固定SHAに依存し、同名crateのcrates.io namespaceでは現在のpackageを公開できない | `Cargo.toml`、Cargo metadata、unit_010 | `publish=false`を維持し、releaseはunit_010の未解決条件から分離しない |
| low | local-address実機はWindows 11 / WinUSB / CSR8510 A10 / Switch 2 22.5.0 / Pro各1環境で、反復成功率を測っていない | T08 evidence | 成功runを一般的な信頼性へ拡張せず、他OS、adapter、Joy-Con実機を先送りする |
| low | warm reset同期transfer failureはcommand到達の直接証明にならない | fake tests、D03/D04 | bounded再列挙、CSR metadata、exact target read-back、read-back closeの全成功を必須にする |
| low | diagnostic helperは対象USB adapterを書き換える | ignored `adapter-tests` | `cfg(test)`かつ明示ignoredに限定し、通常test/CIでは実行しない |

critical/high findingはない。追加差分に`unsafe`、広いlint抑制、製品入力境界の`unwrap` / `expect`はない。
test設計はpure codec、fake identity backend、scripted Bumble、profile/CLI integration、ignored adapter test、
machine evidence、UI観測を分離している。単一runのUI観測をprotocol、profile persistence、復旧の代替根拠にしていない。

## 9. 先送り事項

- non-CSR identity backend
- persistent local address
- duplicate address detection
- Linux / macOS hardware matrix
- upstream issue / PR はユーザ指示により作成しない。equivalent capabilityを持つofficial revisionへの復帰は先送り
- crates.io へ配布可能な Bumble dependency namespace

## 10. 完了チェックリスト

- [x] T01–T09 が `refactor-done` または理由付き `refactor-skipped`
- [x] test item ごとの commit がある
- [x] local-address create-profile が `UnsupportedCapability` ではなく実装済み
- [x] local-address existing profile の reconnect が実装済み
- [x] expected-address guard が pairing / advertising 前に働く
- [x] write 後 failure が typed recovery error になる
- [x] physical power cycle と元 identity 復帰を実機確認した
- [x] profile namespace が expected local address と一致する
- [x] address / key / selector / path / raw packet が公開 error と trace に出ない
- [x] duplicate address guidance と対応範囲が公開 docs にある
- [x] Bumble fork branch / commit / 差分 / rollback を記録した
- [x] default / all-features gate が成功した
- [x] `agentic-self-review` の指摘を採否記録した
- [x] `spec/complete/unit_011/` へ移動した

PR merge、default branch同期、作業branch削除は、このcompletion commit後に`pr-merge-cleanup`で実行し、
repository外のGitHub/ローカルbranch状態として検証する。
