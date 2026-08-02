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
| done | T01: Python 0.6.0 Classic profileだけがtyped parse/canonical writeでき、旧Rust raw peer、unknown field、`address_type`、LE key fieldを秘密値非表示の`InvalidProfile`で拒否する | regression / edge | profile unit / integration | typed serdeへ置換済み |
| done | T02: single writerの更新はexpected bytesやfile lockなしでexisting regular profileをatomic replaceし、中断前後にold/newのcomplete profileだけを残す | behavior | store unit / key-store integration | `fs2`とstale-writer `WouldBlock`を削除済み |
| done | T03: create-profileはtargetを事前inspectせず、create-new競合だけを`ProfileAlreadyExists`へ写像し、feature無効時はfilesystem I/Oなしで`UnsupportedCapability`を返す | regression | controller unit / public integration | store port統合済み |

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | canonical fixtureへ揃えた上でroot/identity/key-store/peer/keyのunknown field、`address_type`、LE keyを拒否するtestを追加した。`cargo test -p swbt-rs --lib --all-features --locked profile::document_tests::canonical_classic_profile_rejects_unknown_legacy_and_non_classic_fields`は、現行`ProfileDocument`がroot extensionを保持してparseしたため期待どおり失敗した |
| green | T01 | strict field検証、`/P`必須化、16-byte link key検証、address uppercase正規化を追加した。`cargo test -p swbt-rs --all-features --locked profile`はprofile関連unit 39件とintegrationを成功し、手動cross-language writer 1件だけをignoredとした |
| refactor-done | T01 | `ProfileDocument`の保持値を未知field拒否付きserde struct、typed address map key、typed Classic bondへ置換し、runtime側の`Value` decodeを削除した。同じprofile testと`cargo clippy -p swbt-rs --all-targets --all-features --locked -- -D warnings`が成功した |
| red | T02 | store testをsingle-writerの`update(path, replacement)`契約へ変更した。focused testは現行traitがexpected bytesを含む3引数を要求したため`E0061`で期待どおりcompile失敗した |
| green | T02 | `ProfileUpdatePort`とBumble key-storeからexpected bytesを除去し、OS file lock、CAS、`fs2`依存を削除した。`cargo test -p swbt-rs --all-features --locked profile`はprofile関連unit 38件とintegrationを成功し、atomic writer破棄時のold profile保持も確認した |
| refactor-skipped | T02 | store portの統合はcreate-profile事前inspect削除と同時に行う方が境界を一度で確定できるためT03へ残した。`cargo clippy -p swbt-rs --all-targets --all-features --locked -- -D warnings`は成功した |
| red | T03 | create成功時のevent列から`InspectTarget`を除いた。focused testは実装が`InspectTarget → CreateNew → Open`を記録したため、期待する`CreateNew → Open`との差分で失敗した |
| green | T03 | create planからstore引数とtarget inspectを除去し、`create_new`だけを競合判定にした。all-featureのcreate-profile 8 unit / 3 public testと、no-featureのbackend contract 6 testが成功し、NUL入りpathでもfilesystemへ触れず`UnsupportedCapability`を返した |
| refactor-done | T03 | read/create/updateの3 traitをcrate-private `ProfileStore`へ統合し、feature無効時に不要なClassic bond projectionをcompile対象外にした。all-feature lib 262 passed / 1 ignored、all-featureとno-featureのclippy `-D warnings`が成功した |
| refactor-done | T01 / T02 | identityの`kind`専用enum、profile専用Classic bond DTO、`BondStore`のforwarding wrapperと二重boxingを削除した。unit variantだけのinternally tagged identityは余分なfieldを受理したため採用せず、既存strict rejection testが守るpayload structを維持した。all-feature profile unit 36件、no-feature profile unit 28件、Python fixture integration 5件が成功し、manual cross-language writer 1件だけをignoredとした |

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
| `src/runtime/transport/profile_key_store.rs` | modify | backend Classic bondへの直接projection、single-writer commit、二重boxing削除 |
| `src/runtime/transport/bumble.rs` | modify | factoryが返す`BondStore`を直接backendへ渡す |
| `tests/profile_compat.rs` | modify | Python canonical compatibilityとreject境界 |
| `tests/backend_unavailable_contract.rs` | modify | feature無効時のfilesystem非接触 |
| `tools/swbt-probe/tests/probe_cli.rs` | modify | profile inspect / verify用fixtureのcanonical化 |
| `tools/swbt-hardware-runner/src/scenarios/pro_profile.rs` | modify | stale-bond用fixtureのcanonical化 |
| `src/lib.rs` | modify | profileとconcurrencyの公開契約 |
| `spec/initial/*.md` | modify | schema/persistence/testingのIntent Delta反映 |
| `spec/complete/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md` | new | 完了仕様とTDD記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-rs --all-features --locked profile` | success | 着手前baseline。focused unit/integrationとPython fixtureが成功、manual cross-language writer 1件ignored |
| `cargo clippy -p swbt-rs --all-targets --all-features --locked -- -D warnings` | success | 着手前baseline |
| itemごとのfocused `cargo test` | success | T01 strict rejection、T02 single-writer store、T03 create orderingとfeature無効filesystem非接触をRED/GREENで確認 |
| post-green focused profile test | success | `cargo test -p swbt-rs --lib --all-features --locked profile`は36件、`cargo test -p swbt-rs --lib --no-default-features --locked profile`は28件、`cargo test -p swbt-rs --test profile_compat --all-features --locked`は5 passed / 1 manual ignored。identity unknown field拒否、canonical bytes、backend bond round-tripを維持 |
| `cargo test --workspace --all-targets --all-features --locked` | success | 初回にtoolの旧fixture 1件を検出してcanonical化。再実行はworkspace全target成功。本体262 passed / 1 manual ignored、profile互換5 passed / 1 manual ignored、実機adapter 5 ignored |
| `cargo test -p swbt-rs --all-targets --no-default-features --locked` | success | 本体248 passed / 1 manual ignored。backend-unavailable 6件を含む全integration成功 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | workspace 3 packageにwarningなし |
| `cargo clippy -p swbt-rs --all-targets --no-default-features --locked -- -D warnings` | success | feature無効境界にwarningなし |
| `rustup run 1.87.0 cargo check --workspace --all-targets --all-features --locked` | success | declared MSRVでworkspace 3 package成功 |
| `cargo build --workspace --all-features --locked` | success | Cargo dependency変更後の全feature build成功 |
| `cargo package -p swbt-rs --locked` | success | 113 files、1.2 MiBをpackageし、展開crateのcompile成功。publishは未実行 |
| `cargo test --doc -p swbt-rs --all-features --locked` | success | crate doctest 1件成功 |
| `cargo doc -p swbt-rs --all-features --no-deps --locked` | success | public rustdoc生成成功 |
| `cargo tree -p swbt-rs --no-default-features --edges normal --depth 1 --locked` | success | 直接依存は`atomic-write-file`、`serde`、`serde_json`。`fs2`なし |
| `cargo fmt --all --check` | success | workspace format差分なし |
| `git diff main...HEAD --check` | success | whitespace errorなし |
| docs-quality-review | success | README、crate rustdoc、`spec/initial`、完了仕様の契約一致と旧文言・仮テキスト残りなしを確認 |
| rust-api-boundary-review | success | 公開signature追加なし。既存`PairingProfile`の受理範囲と`create_profile`のerror順序を明記し、DTO/store traitはcrate-privateを維持。unsafe/async変更なし |
| agentic-self-review | success | requirement/scope/TDD/diff/gateを照合し、未解決findingなし |
| Rust writer → pinned Python reader | not run | repositoryのmanual testはignored。Rust側はpinned Python fixtureのread/canonical writeを検査したが、Python interpreterによる再読込は未実行 |
| Windows real-device reconnect | not run | 実機I/Oは対象コードの検証に必須ではなく、安全な専用profile copyを伴う実行指示がないため未実行 |

## 10. 先送り事項

- none

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを更新した
- [x] T01-T03をitem単位でRED/GREEN/refactor/commitした
- [x] Python 0.6.0 Classic profileだけをcanonical contractにした
- [x] unknown/legacy/LE profileを秘密値非表示で拒否した
- [x] single-writer persistenceからlock/CASを削除した
- [x] target事前inspectを削除した
- [x] public docsと`spec/initial`を更新した
- [x] default/all-feature/MSRV/package gateを記録した
- [x] hardware実行結果または未実行理由を記録した
- [x] self-reviewを完了した
- [x] `spec/complete/unit_017`へ移動した
