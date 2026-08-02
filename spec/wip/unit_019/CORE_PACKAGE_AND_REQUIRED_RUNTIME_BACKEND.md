# core package 分離・runtime backend 必須化仕様書

## 1. 概要

### 1.1 目的

backend 非依存の model、input、profile、protocol 実装を公開 package `swbt-core` へ分離する。
runtime package `swbt-rs`（library target `swbt`）は `swbt-core` に依存し、Bumble backend と
USB discovery を常に組み込む。これにより、runtime を backend なしで構築するためだけに存在する
fallback 分岐と `dead_code` 抑制を削除し、backend 非依存 build の正本を `swbt-core` に移す。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub issue | `bumble` の標準・必須化と core crate 分離案の比較 | https://github.com/niart120/swbt-rs/issues/22 |
| issue comment | `swbt-core` と runtime `swbt` の責務案、feature 無効 build が生む抑制 | https://github.com/niart120/swbt-rs/issues/22#issuecomment-5153313291 |
| user decision | Issue #22 を二つの work-unit で進め、第二段で core 分離と Bumble 必須化を行う | 2026-08-02 の対話 |
| unit_018 | runtime lifecycle、error、接続 API の簡素化を先に完了 | `spec/complete/unit_018/RUNTIME_LIFECYCLE_AND_CONNECTION_API_SIMPLIFICATION.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| pure value 利用者 | `swbt-core` だけを依存へ追加 | model-valid input、profile JSON、profile inspection、共有 error 型を利用できる | Bumble、libusb、`tracing`、profile writer を依存 graph に含めない |
| `swbt` 利用者 | 従来の `swbt::{ProInputState, PairingProfile, ...}` を使用 | core 型が同一型として再公開され、既存 module path と crate-root path を維持する | wrapper 型を複製しない |
| runtime 利用者 | default または `--no-default-features` で `swbt` を構築 | Bumble transport と USB discovery が常に存在する | backend 不在を表す `UnsupportedCapability` 分岐を通らない |
| maintainer | pure protocol test と dependency graph を検査 | protocol test は `swbt-core` で実行され、runtime backend を link しない | raw protocol engine は本 unit で安定公開 API にしない |
| tool package | `swbt-probe` / `swbt-hardware-runner` を構築 | 廃止した `bumble` feature を指定せず runtime を利用する | `diagnostics-schema` と `adapter-tests` は用途別 feature として残す |

## 2. 対象範囲

- workspace member `crates/swbt-core` を追加し、package 名 `swbt-core`、library 名 `swbt_core` とする
- `error`、`input`、`model`、profile 値・JSON・inspection、pure protocol engine を `swbt-core` へ移す
- `swbt-core` の通常依存を `serde` と `serde_json` に限定する
- filesystem profile writer、controller、worker、adapter、transport、reporting、runtime diagnostics は `swbt` に残す
- `swbt` は既存の core 公開型と `error` / `input` / `model` / `profile` module path を同一型として再公開する
- protocol engine、model の wire metadata、profile bond mutation は rustdoc 非表示の runtime support 境界に置く
- profile document と Bumble `ClassicBond` の間は backend 非依存の内部値を介して変換する
- `bumble` feature を削除し、`swbt-bumble-backend`、`rusb`、backend 利用に必要な `tracing` を必須依存にする
- `adapter-tests` は実機 test の opt-in marker、`diagnostics-schema` は安定 event schema の opt-in markerとして残す
- backend 無効時の公開 fallback、`cfg(feature = "bumble")`、対応する `dead_code` 抑制と test を削除する
- pure-core CI、MSRV、Clippy、rustdoc、package metadata、README、初期仕様、変更履歴を新境界へ同期する

## 3. 対象外

- package version の `0.2.0` 更新、crates.io publish、tag、GitHub Release
- raw protocol engine を利用者向け安定 API として設計し直すこと
- `Error` / `ErrorKind` を core 用と runtime 用の別型へ分割すること
- `diagnostics-schema` を default 有効にすること
- `adapter-tests` を通常 test に含めること
- 別 backend の trait package 化、backend 選択 feature の追加
- profile schema、wire bytes、controller/runtime の観測挙動変更
- USB adapter、Switch UI、実機 pair/reconnect の再検証

## 4. 関連 docs

- `spec/initial/api.md`
- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/initial/QUALITY_GATES.md`
- `spec/complete/unit_015/TOOLING_CRATE_SEPARATION.md`
- `spec/complete/unit_017/PROFILE_MODEL_AND_STORE_SIMPLIFICATION.md`
- `spec/complete/unit_018/RUNTIME_LIFECYCLE_AND_CONNECTION_API_SIMPLIFICATION.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| core public values | `swbt_core` から model/input/profile/error を使用 | 現行 `swbt` と同じ検証、JSON、型制約、error kind | `swbt` は同一型を再公開する |
| profile secrecy | core で profile parse/serialize/inspect | path、address、link key を error/debug へ出さない | file writer は runtime 側 |
| pure protocol | `swbt-core` の protocol test/fixture | 現行の byte-for-byte 出力と malformed input 契約を維持 | runtime support 面は rustdoc 非表示 |
| core dependency graph | `cargo tree -p swbt-core` | Bumble、`rusb`、`tracing`、`atomic-write-file` を含まない | default/no-default の差は設けない |
| runtime dependency graph | `cargo tree -p swbt-rs --no-default-features` | `swbt-core`、Bumble、`rusb`、`tracing` を含む | `--no-default-features` は diagnostics schema 等を無効にするだけ |
| runtime open | default/no-default build で不正 selector を open | backend 不在ではなく `TransportOpen` | USB を開く前に selector validation で失敗させる test を使う |
| adapter discovery | `list_adapters()` | 常に descriptor discovery 実装へ進む | 実機 I/O 成功は本 unit で主張しない |
| profile create | default/no-default build で既存 target | backend 判定より前の no-replace `ProfileAlreadyExists` | backend 無効 fallback を削除する |
| feature metadata | downstream/tool が `bumble` feature を指定 | 廃止された feature として解決しない | tool manifest と docs は指定を削除する |

### 5.1 Intent Delta

- backend 非依存 build の単位を `swbt-rs --no-default-features` から `swbt-core` へ変更する。
- `swbt-rs` の default/no-default build はともに Bumble runtime を含む。
- `bumble` は選択可能な feature ではなく runtime package の構成要素とする。
- model/input/profile の既存 `swbt` path は互換再公開とし、新規利用者は runtime 不要なら
  `swbt-core` を直接選べる。
- protocol engine は package 境界を越えるため Rust の可視性上は到達可能になるが、rustdoc 非表示の
  runtime support とし、本 unit では semver 安定面に含めない。

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-done | T01 `swbt-core` から model-valid input と profile 値を直接利用でき、`swbt` の既存 path が同一型を再公開する | behavior / compatibility | package integration | profile fixture と public contract test を core package へ移した |
| refactor-done | T02 pure protocol の全 unit/fixture test が `swbt-core` で同じ bytes と error を返し、`swbt` runtime が一つの protocol 実装を利用する | regression / architecture | core unit / runtime build | raw protocol 面は rustdoc 非表示 |
| refactor-done | T03 default/no-default の `swbt` が常に Bumble backend を含み、不正 selector は `TransportOpen`、既存 profile target は `ProfileAlreadyExists` となる | behavior / package | public integration / Cargo graph | `bumble` cfg、fallback、dead-code 抑制を削除した |
| refactor-done | T04 core/runtime/tool の feature metadata、CI、README、rustdoc、初期仕様、package archive が同じ package 境界を示す | regression / docs | metadata / docs / package | core archiveは検証済み。root archive生成はcore公開後へ先送り |

status は `todo`、`red`、`green`、`refactor-done`、`refactor-skipped`、`deferred` を使う。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | `cargo test -p swbt-core --test public_values` は `ButtonKind`、`PairingProfile`、`ProInputState` 等が `swbt_core` に存在しない E0432/E0433 で失敗した |
| green | T01 | error/model/input/profile valueと対応fixture/testを `swbt-core` へ移した。core 17 unit tests、37 integration tests成功、profile manual gate 1件ignored。`swbt` の同一型再公開test 2件も成功した |
| refactor-done | T01 | runtimeだけの reporting/controller assertionをroot integration testへ残し、core profileのbond値をbackend型から分離した。core/runtime対象Clippy `-D warnings`、fmt、diff checkが成功した |
| red | T02 | `cargo test -p swbt-core --test protocol_runtime_support` は `swbt_core::__private::SwitchHidProtocol` が存在しない E0432 で失敗し、protocol実装がrootに残る境界を検出した |
| green | T02 | protocol source、65 protocol unit tests、fixture audit 2件、fixtureをcoreへ移した。core全体は82 unit testsとcore integrationが成功し、root runtime 185 testsも同じengine経由で成功した |
| refactor-done | T02 | root `src/protocol.rs` をprivate re-export bridgeだけにし、source二重化を避けた。coreの通常依存はserde/serde_jsonだけで、Bumble、rusb、tracing、atomic writer不在。core/root対象Clippy `-D warnings`が成功した |
| red | T03 | no-defaultでproduction backendへ到達する公開testはactual `UnsupportedCapability` / expected `TransportOpen` で失敗した。直接依存graphにもBumble/rusb/tracingが存在しなかった |
| green | T03 | Bumble/rusb/tracingを必須依存にし、no-default公開backend testは`TransportOpen`で成功した。default/no-defaultのroot全target testは各185 library passed/1 ignored、public runtime backend 4 passed。all-featureだけadapter実機5件をignoredで列挙した |
| refactor-done | T03 | `bumble` feature、backend不在fallback test、77個のfeature-disabled `dead_code`抑制、全`cfg(feature = "bumble")`を削除した。tool manifestもfeature指定をやめ、workspace all-featureとroot no-default Clippy `-D warnings`が成功した |
| red | T04 | 現行CIと`tools/check-library-features.ps1`はroot no-default graphからBumble/rusb/tracingが消える旧契約を検査し、README、公開rustdoc、初期仕様にも廃止した`bumble` featureとbackend無効時`UnsupportedCapability`の説明が残っていた |
| green | T04 | feature検査script、core全target 82 unitと各integration、runtime no-default backend契約4件、workspace doctest 1件、core/root rustdoc `-D warnings`が成功した。metadataはcore通常依存2件、root feature 3件とmandatory backendを示した |
| refactor-done | T04 | README、CHANGELOG、core README、公開rustdoc、初期仕様、CIを同じ境界へ同期した。core packageはMIT本文を含む56 files、287.9 KiBでverify成功。rootは通常/`--no-verify`とも未公開coreのregistry解決で失敗し、`cargo package --list`で収録対象だけを確認してrelease unitへ先送りした |

## 7. 設計メモ

- `swbt-core` の portable は backend/USB/thread runtime 非依存を意味する。profile inspection の
  portable filesystem I/O と `std` は許容し、`no_std` は本 unit の契約にしない。
- `Error` と `ErrorKind` は型の二重化を避けるため core が所有し、`swbt` が再公開する。
  runtime-only variant を含む共有語彙の分割は別の API redesign として対象外にする。
- root `swbt::profile` は core の公開値と runtime 内部の file writer を束ねる compatibility module とする。
- protocol source を二重保持しない。core の runtime support re-exportを root の private `protocol` module が使う。
- backend bond 型を profile document に直接持ち込まない。link key bytes/type/authenticated の
  backend 非依存値へ変換してから runtime adapter で `ClassicBond` と相互変換する。
- `tracing` は adapter discovery/runtime の必須依存になるが、schema-v1 event の emit は引き続き
  `diagnostics-schema` で制御する。
- root package の clean verification と archive生成は、`--no-verify`でも未公開 `swbt-core` の
  registry解決を要求する。core package gateとrootの収録対象一覧を先に確認し、root archive/verifyは
  release work-unitでcore公開後に実行する。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | workspace member、core dependency、Bumble必須化、feature整理 |
| `crates/swbt-core/Cargo.toml` / `README.md` / `LICENSE` | new | publishable core package metadataと利用入口 |
| `crates/swbt-core/src/**` | new / move | error、input、model、profile、protocol、runtime support |
| `crates/swbt-core/tests/**` | new / move | public value/profile/protocol contract と fixture |
| `src/lib.rs` / `src/profile/**` / `src/protocol.rs` | modify | core再公開、runtime file store、private protocol bridge |
| `src/{adapter,controller,runtime}/**` | modify | mandatory backend path と cfg/dead-code抑制削除 |
| `tests/**` | modify / move / delete | backend無効契約をmandatory backend契約へ置換、core test移動 |
| `tools/*/Cargo.toml` | modify | 廃止 `bumble` feature指定削除 |
| `.github/workflows/ci.yml` | modify | pure-core gate とruntime mandatory graph |
| `README.md` / `CHANGELOG.md` | modify | package選択、破壊的feature変更、移行方法 |
| `spec/initial/{api,architecture,testing,roadmap,migration-strategy}.md` | modify | Intent Delta反映 |
| `spec/wip/unit_019/CORE_PACKAGE_AND_REQUIRED_RUNTIME_BACKEND.md` | new / modify | 作業仕様とTDD証拠 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test -p swbt-core --all-targets` | success | T01: 17 unit、37 integration passed、manual profile 1 ignored。protocolはT02前 |
| `cargo tree -p swbt-core --edges normal` | success | T02: direct dependencyはserde/serde_jsonのみ。Bumble、rusb、tracing、atomic writerなし |
| `cargo test -p swbt-rs --test core_reexports --all-features` | success | T01: crate root/module pathの同一型再公開 2 passed |
| `cargo test -p swbt-rs --all-targets --all-features` | success | T03: library 185 passed/1 ignored、integration成功、adapter実機5 ignored |
| `cargo test -p swbt-rs --all-targets --no-default-features` | success | T03: library 185 passed/1 ignored、runtime backend 4 passed、他integration成功。adapter実機targetは0 tests |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | pending | workspace lint |
| `cargo +1.87.0 test --workspace --all-targets --all-features --locked` | pending | MSRV |
| `cargo test --doc --workspace --all-features --locked` | success | core/root crate example各1件passed、tool 0件 |
| `cargo rustdoc -p swbt-core --locked -- -D warnings` | success | backend非依存の公開値とhidden runtime supportを検査 |
| `cargo rustdoc -p swbt-rs --all-features --locked -- -D warnings` | success | core同一型再公開とmandatory backend説明を検査 |
| `cargo build --workspace --all-features --locked` | pending | libraryとtool package |
| `cargo package -p swbt-core --locked --allow-dirty` | success | MIT本文を含む56 files、287.9 KiB、55.8 KiB compressed、archive verify成功 |
| `cargo package -p swbt-rs --locked --allow-dirty` | expected failure | crates.ioに`swbt-core` 0.1.0が未公開のため解決不能。`--no-verify`も同じ段階で失敗 |
| `cargo package -p swbt-rs --allow-dirty --list` | success | root packageの収録対象を確認。archive生成/verifyはcore公開後に再実行 |
| `cargo fmt --all -- --check` | pending | workspace formatting |
| `git diff --check` | pending | branch差分 |
| hardware / USB / Switch UI | not run | package/feature境界変更であり対象外 |

## 10. 先送り事項

- package version更新と `swbt-core`→`swbt-rs` の順の crates.io publishはrelease work-unitで行う。
- raw protocol APIの安定公開、backend trait package、別backend選択は要求が具体化した後に設計する。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を作成した
- [ ] 各 TDD item を個別に検証・commitした
- [ ] 検証結果または未実行理由を記録した
- [ ] package / release / public API gateを記録した
- [ ] `rust-api-boundary-review`、`rustdoc-style`、`docs-quality-review` を完了した
- [ ] `agentic-self-review` を完了した
- [ ] 完了条件を満たして `spec/complete/unit_019` へ移動した
