# M0 リポジトリ・依存・型モデル基盤 仕様書

## 1. 概要

### 1.1 目的

Rust 移植の最初の作業単位として、library crate、Bumble 依存、CI、controller model と reporting mode、モデルごとの入力能力を固定する。M1 以降の protocol と runtime は、この作業単位で確定した型と model 宣言を使う。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| user request | roadmap 順で言語移植を完遂し、TDD 項目単位で commit する | 対話上の依頼 |
| roadmap | M0 repository / dependency / type model と exit criteria | `spec/initial/roadmap.md` |
| type model | `Controller<M, R>`、model-valid input、共通値型 | `spec/initial/type-modeling.md` |
| public contract | library target `swbt` と初期公開 API | `spec/initial/api.md` |
| architecture | model 宣言の単一正本と module 境界 | `spec/initial/architecture.md` |
| test policy | model / mapping audit、値型、package gate | `spec/initial/testing.md` |
| source baseline | Python と Bumble の固定 commit | `spec/initial/source-baseline.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| Rust 利用者 | `swbt` を library dependency として参照する | model、reporting、input、controller の公開型を import できる | Bumble 型と内部 protocol 型は公開しない |
| 静的 Rust API | controller model と reporting mode を選ぶ | `Controller<M, R>` と 6 alias が選択を型として保持する | runtime setter で kind/mode を変更しない |
| 動的入力境界 | `ButtonKind` と対象 model | 対応 button だけ `Button<M>` へ変換できる | 非対応 button は `UnsupportedInput` |
| report encoder の後続実装 | model と button | Python 基準断面と一致する byte index / mask を取得できる | 論理 ID を wire bit と同一視しない |
| package/CI | MSRV または stable toolchain | fmt、check、clippy、test、doc を再現できる | Rust 1.87 を MSRV とする |

## 2. 対象範囲

- library target 名 `swbt`、Rust 2024、MSRV 1.87、MIT license の package metadata
- placeholder binary の削除と crate root の `#![forbid(unsafe_code)]`
- Bumble 基準 commit `bbac2a6803b8cab0920ab725a23aa408fc4fed85` への exact revision 依存
- model marker、reporting marker、sealed trait、kind projection
- model 宣言からの profile 名、supported button、stick 能力、wire mapping の一意な導出
- `ButtonKind`、`Button<M>`、`ButtonSet<M>` と model-specific alias
- `Stick`、`ImuFrame`、`ImuSamples` の値型と入力検査
- `InputState<M>` と model-specific alias
- `Controller<M, R>`、`ControllerBuilder<M, R>` と 6 controller alias の型基盤
- M0 の公開 API を使う example と rustdoc
- fmt、MSRV/stable check、clippy、test、doctest、doc を実行する GitHub Actions workflow
- M0 の package、API、model mapping、文書整合の gate

## 3. 対象外

- `0x30` report 生成、`0x01` / `0x10` parser、subcommand、SPI、IMU wire encoding
- worker thread、command channel、scheduler、Periodic / Direct の状態確定
- `build()` / `create_profile()` の filesystem・transport orchestration
- USB、HCI、Classic、SDP、HIDP、pairing、reconnect
- Python fixture generator と golden fixture
- 実機、adapter-only、Miri、fuzz、benchmark
- Direct / Periodic の runtime 操作 method
- profile schema v2 の read/write
- compile-fail / compiler UI test

## 4. 関連 docs

- `spec/initial/source-baseline.md`
- `spec/initial/type-modeling.md`
- `spec/initial/api.md`
- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/migration-strategy.md`
- `spec/initial/roadmap.md`
- `spec/initial/QUALITY_GATES.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| library package | 通常の Cargo build | crate 名 `swbt` を import でき、binary target と `Hello, world!` がない | package 名は `swbt-rs` を維持 |
| model projection | Pro / JoyConL / JoyConR | kind、profile 名、`ModelSpec` が一対一で一致する | model trait は sealed |
| reporting projection | Periodic / Direct | `ReportingKind` を型から一意に得る | reporting trait は sealed |
| logical button | `0x00..=0x13` | 重複のない stable logical code として扱える | wire offset には使わない |
| model-valid button | model と `ButtonKind` | supported 集合だけ変換に成功し、stable logical order で列挙できる | 重複入力は正規化 |
| explicit wire mapping | model-valid button | Python 基準 commit と一致する report byte index / mask が得られる | Joy-Con L/R の SL/SR を区別 |
| stick | raw または normalized 値 | `0..=4095` を保持し、範囲外・非 finite を `InvalidInput` にする | center は 2048 |
| IMU | raw 6 軸値または 1/3 frame | signed 16-bit を保持し、1 frame は 3 frame へ展開する | wire encoding は M1 |
| model-valid state | model-specific button/stick/IMU | neutral と置換後の完全状態を型付きで保持する | 公開 API から非対応 stick を設定できない |
| controller type surface | model と reporting | generic 正本と 6 alias を通常の Rust code から参照できる | runtime は M2 |
| reproducible dependency | Cargo resolution | Bumble 系 direct dependency は固定した 1 revision だけを使う | branch 追従と path override を禁止 |
| CI gate | pull request / push | MSRV と stable で M0 の標準 gate を実行する | 将来 milestone の job は先回りしない |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | package が `swbt` library target、Rust 1.87、MIT を公開し、placeholder binary を含まない | new | package | red: `swbt` import 失敗。green: library target と metadata を確認。追加の構造変更なし |
| refactor-done | Pro / JoyConL / JoyConR と Periodic / Direct が sealed trait を通じて一意な kind/spec を投影する | new | unit | red: 公開 module/type 不在。green: 2 passed。model の重複宣言を macro の単一正本へ統合 |
| refactor-skipped | model ごとの supported `ButtonKind` だけが `Button<M>` へ変換され、`ButtonSet<M>` が重複を除いて論理順に列挙する | new | unit | red: input/error surface 不在。green: 3 passed。model 宣言へ supported 集合と associated constants を集約済み |
| refactor-done | 全 supported button が Python 基準 commit と一致する byte index / mask を持ち、非対応組み合わせに mapping がない | characterization | unit | red: mapping type/function 不在。green: 2 passed。M1 encoder から使える `pub(crate)` lookup を保持し、同じ mapping を動的変換にも使用 |
| refactor-done | `Stick` が 12-bit raw 値と非対称 normalized 変換を保持し、不正値を拒否する | new | unit | red: Stick/capability trait 不在。green: 5 passed。model の stick bool を capability 宣言へ統合 |
| refactor-skipped | `ImuFrame` と `ImuSamples` が signed 16-bit 六軸値と 1/3 frame の順序を保持する | new | unit | red: IMU 型不在。green: 3 passed。追加の構造変更なし。物理単位と wire conversion は M1 |
| refactor-skipped | `InputState<M>` が model-valid button、stick 能力、3 IMU frame を含む neutral / replacement state を保持する | new | unit | red: state alias 不在。green: 3 passed。private 完全状態で不正な public constructor なし |
| refactor-skipped | `Controller<M, R>`、`ControllerBuilder<M, R>` と 6 alias を公開 API から参照できる | new | integration | red: controller surface 不在。green: 2 passed。`Controller` は `Send` / 非 `Sync` とし、runtime method/field は M2 のまま |
| refactor-done | Cargo が Bumble 基準 commit の direct dependency を単一 revision で解決し、lockfile に固定する | new | package | red: direct dependency 不在。green: metadata/lock/build。crates.io 同名 package との衝突を検出し publish を無効化 |
| refactor-done | M0 の公開 example と rustdoc が通常の build で compile する | new | integration | red: example target 不在。green: example/doc/doctest。公開文面から milestone 内部語を除去 |
| refactor-skipped | GitHub Actions が MSRV/stable の fmt、check、clippy、test、doctest、doc gate を定義する | new | package | red: workflow 不在。green: YAML と同一 local commands。remote run は PR 後 |

## 7. 設計メモ

- `ButtonKind` の論理コードと wire mapping は別の型・表として保持する。
- model 宣言に同じ supported button 集合を重複記述しない。model audit は宣言から得た集合、kind、profile 名、stick 能力、mapping の整合を検査する。
- `Controller<M, R>` と builder は M0 では型基盤だけを公開する。I/O と lifecycle を持つ構築処理は M2 へ残す。
- model 固有の method absence は Rust の型定義で表し、専用 compile-fail test は追加しない。
- MIT は commit `b46164b` で採用済みであり、M0 では実装済み metadata と初期仕様の記録を一致させる。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | library、MSRV、exact dependency |
| `src/main.rs` | delete | placeholder binary を削除 |
| `src/lib.rs` | new | crate root、公開 re-export、unsafe 禁止 |
| `src/error.rs` | new | M0 の入力境界で使う typed error |
| `src/model/` | new | model 宣言、kind、spec、能力 trait |
| `src/reporting/` | new | reporting marker、kind、sealed trait |
| `src/input/` | new | button、mapping、stick、IMU、state |
| `src/controller/` | new | generic type と alias |
| `src/profile/` | new | M0 で必要な公開 projection |
| `tests/` | new | model、mapping、値型、公開 surface |
| `examples/` | new | M0 公開 API の compile 例 |
| `.github/workflows/ci.yml` | new | MSRV/stable gate |
| `README.md` | modify | 実装状態と利用入口 |
| `spec/initial/source-baseline.md` | modify | MIT 採用済み状態へ同期 |
| `spec/wip/unit_001/M0_FOUNDATION.md` | new / modify | 作業仕様と検証記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo test --test library_target` | pass | red は `no external crate swbt`。library target 追加後は 1 passed |
| `cargo test --test model_contract` | pass | red は model/reporting module 不在。green と refactor 後は 2 passed |
| `cargo test --test button_contract` | pass | red は input/error surface 不在。green は 3 passed |
| `cargo test model::tests` | pass | red は wire mapping type/function 不在。Python `84d2723b` の `src/swbt/protocol/buttons.py` に対して 2 passed |
| `cargo test --test stick_contract` | pass | red は Stick/capability trait 不在。green は 5 passed |
| `cargo +1.87 test --test stick_contract` | pass | Rust 1.87.0 で 5 passed |
| `cargo test --test imu_contract` | pass | red は IMU 型不在。green は 3 passed |
| `cargo test --test input_state_contract` | pass | red は state alias 不在。green は 3 passed |
| `cargo test --test controller_type_contract` | pass | red は controller surface 不在。green は 2 passed |
| `cargo metadata --locked --no-deps --format-version 1` の Bumble dependency 検査 | pass | direct `bumble` は基準 SHA の git source 1 件 |
| `cargo build --all-features --locked` | pass | git 版 `bumble 0.1.0` を含めて build 成功 |
| `cargo +1.87 build --all-features --locked` | pass | Rust 1.87.0 で git 版 Bumble を含めて build 成功 |
| `cargo package --allow-dirty` | fail | git dependency に version がないため拒否。crates.io の同名別 package を使う偽の成功を避けるため `publish = false` |
| `.github/workflows/ci.yml` の PyYAML parse | pass | 隔離した `uvx --with pyyaml` 環境で構文検査 |
| `cargo +1.87 check --all-targets --all-features --locked` | pass | CI `check-msrv` と同一 |
| CI stable commands | pass | fmt / check / clippy / test / doc を workflow と同じ引数で実行 |
| `cargo metadata --no-deps --format-version 1` の package contract 検査 | pass | `rust_version=1.87`、`license=MIT`、library `swbt` が 1 件、binary が 0 件 |
| `cargo check --example type_model --locked` | pass | red は example target 不在。恒久名へ整理後に compile 成功 |
| `cargo test --doc --all-features --locked` | pass | crate-level example 1 passed |
| `cargo doc --no-deps --all-features --locked` | pass | missing-docs warning なし |
| `cargo fmt --check` | not run | 実装後に実行 |
| `cargo +1.87 check --all-targets --all-features` | not run | MSRV |
| `cargo check --all-targets --all-features` | not run | stable/current toolchain |
| `cargo clippy --all-targets --all-features -- -D warnings` | not run | static gate |
| `cargo test --all-features` | not run | unit / integration / doctest |
| `cargo build --all-features` | not run | public API / metadata gate |
| `cargo package` | not run | package 内容と metadata |
| `git diff --check` | not run | whitespace |
| GitHub required checks | not run | PR 作成後に確認 |

## 10. 先送り事項

- Python fixture generator と byte-for-byte protocol fixtureは M1。
- worker、builder の build/create-profile behavior、runtime semantics は M2。
- Bumble external HCI と adapter-only 検証は M3。
- Bumble git dependency の crates.io package 名衝突は `spec/dev-journal.md` に記録し、M9 の package / release 前に仕様化する。
- roadmap 主鎖と alpha.2 の diagnostics/probe 順序の不整合は `spec/dev-journal.md` に記録し、M6 完了前に仕様化する。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [ ] TDD Test List を更新した
- [ ] 検証結果または未実行理由を記録した
- [x] package / release / public API に触れる場合の gate を記録した
