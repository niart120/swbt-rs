# profile model / store 簡素化仕様書

## 1. 概要

### 1.1 目的

profileの受理形式を`swbt-python 0.6.0`がBumble 0.0.233のClassic pairingで生成する
schema v2へ一本化する。旧Rust形式、unknown extension、未使用のLE key fieldは受理しない。

保存は同一profile pathを一つのlive controller runtimeだけが更新する契約へ絞る。create-newの
no-replace、更新のatomic replacement、complete fileの同期は維持し、target事前inspect、file lock、
expected全バイトCASを削除する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue | profile modelと永続化契約を簡素化する | `https://github.com/niart120/swbt-rs/issues/21` |
| Issue comment | typed情報と生JSONの二重保持、lockとatomic replace、store seamの追加分析 | `https://github.com/niart120/swbt-rs/issues/21#issuecomment-5153311093` |
| user decision | 旧Rust互換とextension保持を削除し、Python 0.6.0 Classic経路へ一本化する | 2026-08-02のユーザ指示 |
| user decision | 同一pathの複数process/controller更新は非対応、target事前inspectを削除する | 2026-08-02のユーザ指示 |
| pinned source | schema v2 envelope、temporary write、create hard link、update replace | `niart120/swbt-python@84d2723b:src/swbt/transport/_pairing_profile.py` |
| pinned source | current namespaceを一peerへ置換するBumble key-store adapter | `niart120/swbt-python@84d2723b:src/swbt/transport/_bumble_key_store.py` |
| dependency source | Classic `PairingKeys.to_dict()`のfield shape | `bumble==0.0.233:bumble/keys.py` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| Python profile利用者 | Python 0.6.0が生成したPro/Joy-Conのschema v2 profile | 対応する`PairingProfile<M>`として読め、同じ意味のcanonical JSONを書ける | Classic `/P` peer一件だけを扱う |
| 不正profileの利用者 | 旧Rust raw peer、unknown field、LE key field、`address_type`を含むJSON | `InvalidProfile`で拒否される | key、address、pathをerrorへ出さない |
| profile作成利用者 | absent target | complete profileをno-replaceで公開してからtransportを開く | 事前inspectを行わずcreate-new結果を正とする |
| live controller runtime | current profileへのpairing key更新 | oldまたはnewのcomplete JSONだけがpathから観測される | 同一pathの並行writerは非対応 |
| feature無効利用者 | `bumble`なしのcreate-profile | filesystemを検査・変更せず`UnsupportedCapability` | target existenceよりbackend capabilityを先に返す |

## 2. 対象範囲

- schema v2 envelope、identity、namespace、Classic public peer、Classic link keyの型付きserde model
- Python 0.6.0 Classic fixtureのreadとcanonical write
- unknown field、旧Rust raw peer、`address_type`、LE key fieldの拒否
- secret-safe `Debug` / error
- create-new no-replaceとupdate atomic replacement
- 単一writer契約への変更とlock/CAS削除
- target事前inspectと不要なcrate-private store portの削除
- public rustdoc、crate docs、`spec/initial`の現行契約更新
- Cargo dependencyの`serde`追加と`fs2`削除

## 3. 対象外

- schema version 3の導入
- profile migration、互換alias、unknown extensionの保持
- 同一profile pathの複数process/controller更新の検出・直列化・競合解決
- LE pairing keyの保存
- backup、世代管理、破損profileの自動復元
- controller input、HID protocol、readiness、cleanup semanticsの変更
- Bumble backend APIの変更
- crates.io publish、version、tag、GitHub Release

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/migration-strategy.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/initial/source-baseline.md`
- `spec/complete/unit_007/M6_PROFILE_RECONNECT_DIRECT.md`
- `spec/complete/unit_016/CREATE_PROFILE_RUNTIME_SIMPLIFICATION.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| canonical envelope | `format=swbt.profile`、schema 2、対応kind、対応identity、key store | typed profileへ変換できる | root/identity/key-storeはunknown fieldを拒否 |
| canonical peer | uppercase/lowercaseのBluetooth address入力 | typed addressとして検証し、出力はuppercase `XX:XX:XX:XX:XX:XX/P` | raw peerと`/R`等は拒否 |
| canonical Classic key | `link_key.value` 16 bytes hex、`authenticated` bool、`link_key_type` u8 | Bumble `ClassicBond`へ変換できる | field欠落、unknown、`address_type`、LE keyは拒否 |
| deterministic write | typed profile | sorted key、2-space indent、末尾newlineのPython可読JSON | lossless byte round-tripは契約にしない |
| namespace cardinality | namespaceごとに0または1 peer | parse、load、upsertが同じ制約を守る | upsertはcurrent peerを置換する |
| secret safety | invalid profile、key-store失敗、Debug | path、address、key、raw JSONを表示しない | error kindは既存分類を維持 |
| create-new | absent target | complete JSONを同期後、no-replaceで一度だけ公開 | existing file/directory/symlinkは置換しない |
| create conflict | create直前にtargetが出現 | `ProfileAlreadyExists`、competitor bytes不変、transport未open | preflight inspectなし |
| feature無効create | valid builder/path、`bumble`なし | `UnsupportedCapability`、profile filesystem I/Oなし | existing targetの先行判定を廃止 |
| atomic update | single writerが既存regular fileを更新 | 中断前はold、commit後はnewのvalid profile | file syncとsupported OSのparent syncを維持 |
| unsupported concurrency | 同一pathの複数live writer | 結果・競合検出を保証しない | lock、CAS、`WouldBlock`を契約から削除 |

### 5.1 Intent Delta

初期仕様とM6はunknown key field保持、汎用Bumble key field、lock contention拒否を契約にした。
本unitでは、公開実態のなかった旧Rust形式と将来互換用extensionを維持しない。

- 正本は`swbt-python 0.6.0`とBumble 0.0.233がClassic profileとして生成するJSONだけとする。
- unknown fieldを黙って破棄・保持せず、profile入力境界で拒否する。
- 同一pathは一つのlive controller runtimeが所有する。並行writerの競合検出は提供しない。
- no-replace create、atomic update、complete writeの同期は並行更新対応とは独立して維持する。
- target existenceの事前snapshotは予約にならないため廃止し、create-new結果だけを競合判定に使う。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| todo | T01: Python 0.6.0 Classic profileだけがtyped parse/canonical writeでき、旧Rust raw peer、unknown field、`address_type`、LE key fieldを秘密値非表示の`InvalidProfile`で拒否する | regression / edge | profile unit / integration | green後に生`Value`保持をtyped serdeへ置換する |
| todo | T02: single writerの更新はexpected bytesやfile lockなしでexisting regular profileをatomic replaceし、中断前後にold/newのcomplete profileだけを残す | behavior | store unit / key-store integration | `fs2`とstale-writer `WouldBlock`を削除する |
| todo | T03: create-profileはtargetを事前inspectせず、create-new競合だけを`ProfileAlreadyExists`へ写像し、feature無効時はfilesystem I/Oなしで`UnsupportedCapability`を返す | regression | controller unit / public integration | green後にcrate-private store portを整理する |

## 7. 設計メモ

Tidy decision:

- classification: mixed
- action: split
- reason: schema受理範囲、並行更新、feature無効時error順序はbehavior change。typed serdeとstore seam削除は各green後のstructure change。
- verification: itemごとのfocused test、default/all-feature gate、MSRV、package、rustdoc、Python fixture。

型付きserde DTOはcrate-privateとし、公開`ControllerKind`、`ProfileIdentity`へserde traitを追加しない。
Profile modelはtyped fieldだけを保持し、raw documentやextension mapを持たない。addressとhex keyはparse時に
domain validationし、内部mutation後のshape再検証を不要にする。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | direct `serde`追加、`fs2`削除 |
| `src/profile/document.rs` | modify | canonical typed serde model、validation、serialization |
| `src/profile/document_tests.rs` | modify | canonical/legacy/unknown/key境界test |
| `src/profile/store.rs` | modify | single-writer atomic store、preinspect/lock/CAS/port削除 |
| `src/profile/mod.rs` | modify | crate-private export整理 |
| `src/controller/create.rs` | modify | authoritative create-new flow |
| `src/controller/mod.rs` | modify | feature dispatchとstore注入整理 |
| `src/controller/build.rs` | modify | read seam整理 |
| `src/controller/*_tests.rs` | modify | create/buildのobservable contract更新 |
| `src/runtime/transport/profile_key_store.rs` | modify | typed Classic key projectionとsingle-writer commit |
| `tests/profile_compat.rs` | modify | Python canonical compatibilityとreject境界 |
| `tests/backend_unavailable_contract.rs` | modify | feature無効時のfilesystem非接触 |
| `src/lib.rs` | modify | profileとconcurrencyの公開契約 |
| `spec/initial/*.md` | modify | schema/persistence/testingのIntent Delta反映 |
| `spec/wip/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md` | new | 作業仕様とTDD記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-rs --all-features --locked profile` | success | 着手前baseline。focused unit/integrationとPython fixtureが成功、manual cross-language writer 1件ignored |
| `cargo clippy -p swbt-rs --all-targets --all-features --locked -- -D warnings` | success | 着手前baseline |
| docs-quality-review | success | source、対象範囲、Intent Delta、参照先、未実行gate、仮テキスト残りを確認 |
| `cargo test -p swbt-rs --all-features --locked <item filter>` | not run | 各TDD itemで記録する |
| `cargo test --workspace --all-targets --all-features --locked` | not run | completion gate |
| `cargo test -p swbt-rs --all-targets --no-default-features --locked` | not run | feature無効contract |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | not run | completion gate |
| `rustup run 1.87.0 cargo check --workspace --all-targets --all-features --locked` | not run | MSRV gate |
| `cargo build --workspace --all-features --locked` | not run | Cargo dependency/public behavior変更gate |
| `cargo package -p swbt-rs --locked` | not run | publishable package smoke。publishは行わない |
| `cargo fmt --all --check` | not run | completion gate |
| `git diff --check` | not run | completion gate |
| Windows real-device reconnect | not run | software変更後、利用可能なhardwareと安全なprofile copyがある場合だけ実行する |

## 10. 先送り事項

- none

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを更新した
- [ ] T01-T03をitem単位でRED/GREEN/refactor/commitした
- [ ] Python 0.6.0 Classic profileだけをcanonical contractにした
- [ ] unknown/legacy/LE profileを秘密値非表示で拒否した
- [ ] single-writer persistenceからlock/CASを削除した
- [ ] target事前inspectを削除した
- [ ] public docsと`spec/initial`を更新した
- [ ] default/all-feature/MSRV/package gateを記録した
- [ ] hardware実行結果または未実行理由を記録した
- [ ] self-reviewを完了した
- [ ] `spec/complete/unit_017`へ移動した
