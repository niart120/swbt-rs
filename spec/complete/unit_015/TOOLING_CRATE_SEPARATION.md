# 検証ツール crate 分離仕様書

## 1. 概要

### 1.1 目的

公開 `swbt-rs` package から `swbt-probe`、stable diagnostics schema の通常 build、実機認定 runner を分離する。検証資産と既存の transport／protocol 挙動を維持しながら、公開ライブラリの配布内容と責務を縮小する。

実機 runner は `swbt-hardware-runner` という単一 binary に統合する。既存の M5、M6、M7 runner は scenario subcommand として区別し、既存の操作列と evidence schema を維持する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue #19 | probe、diagnostics、実機 runner を公開ライブラリから分離する | `https://github.com/niart120/swbt-rs/issues/19` |
| Issue #19 追加分析 | schema 重複、probe test seam、runner 重複、package 境界の現状分析 | `https://github.com/niart120/swbt-rs/issues/19#issuecomment-5153306876` |
| user request | root の公開 package を維持し、実機 runner の entry point も単一化する | 2026-08-02 の対話 |
| initial architecture | CLI の動的分岐、diagnostics security、公開／非公開 module 境界 | `spec/initial/architecture.md` |
| test policy | hardware、packaging、CI の検証境界 | `spec/initial/testing.md` |
| publishing | crates.io archive の収録範囲と展開後検査 | `spec/publishing.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| 通常のライブラリ利用者 | feature なしで `swbt-rs` を build する | probe CLI、subscriber、stable diagnostics schema、実機 runner をコンパイルしない | `GamepadStatus` は利用できる |
| Bumble 利用者 | `bumble` feature で build する | adapter と runtime を利用でき、必要最小限の非安定 tracing を利用できる | stable schema は `diagnostics-schema` を明示しない限り契約しない |
| probe 利用者 | workspace の `swbt-probe` を実行する | 現在の command、終了コード、秘密値を含まない JSON／NDJSON を得る | tool crate は `publish = false`、公開 `swbt` API のみを使う |
| 実機検証者 | `swbt-hardware-runner <scenario>` を実行する | scenario ごとの既存操作列と evidence schema で検証できる | adapter、profile、key、USB serial、raw packet、error source chainを出力しない |
| crates.io 利用者 | `swbt-rs` package を取得する | probe、hardware runner、tool 専用 test を含まない archive を得る | 利用者向け example と library test fixture は必要な範囲で残す |

## 2. 対象範囲

- root の `swbt-rs` package を workspace root package として維持する
- `tools/swbt-probe` を `publish = false` の workspace member として作る
- `tools/swbt-hardware-runner` を `publish = false` の workspace member として作る
- `swbt-probe` の CLI、subscriber、test seam、integration test を tool crate へ移す
- stable diagnostics event を `diagnostics-schema` feature の内部実装にする
- `tracing` を optional dependency とし、`bumble` または `diagnostics-schema` から有効化する
- `ErrorKind::Trace` を公開 error から削除し、probe 内部 error に移す
- 三つの実機 runner を `swbt-hardware-runner` の scenario subcommand へ統合する
- runner の引数解析、秘密値を保持する引数の `Debug`、evidence record、status射影、profile検査、adapter reopenを共通実装へ寄せる
- README、利用者向け docs、CI、package検査を workspace 構成へ合わせる
- 保存済み fixture と hardware evidence を維持する

## 3. 対象外

- transport、protocol、pairing、reconnect、入力、neutral close の意味変更
- `GamepadStatus` または accepted counter の削除
- diagnostics event schema v1 の field、event名、意味の変更
- M5、M6、M7 evidence schema の統合または過去 evidence の書き換え
- `swbt-probe` と `swbt-hardware-runner` を一つの binary に統合すること
- 公開ライブラリを `crates/swbt` へ移動すること
- 保存済み evidence、fixture、完了済み work unit の削除
- 新しい controller model、reporting mode、実機操作の追加
- crates.io publish、tag、GitHub Release

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/testing.md`
- `spec/initial/roadmap.md`
- `spec/initial/QUALITY_GATES.md`
- `spec/publishing.md`
- `spec/complete/unit_006/M5_PRO_PERIODIC_HARDWARE.md`
- `spec/complete/unit_007/M6_PROFILE_RECONNECT_DIRECT.md`
- `spec/complete/unit_008/M7_JOYCON_L_R.md`
- `spec/complete/unit_009/M8_IMU_DIAGNOSTICS_PROBE.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| workspace default | repository root で package 未指定の Cargo command | root `swbt-rs` package を対象にする | tools を含む gate は `--workspace` を明示する |
| feature なしのライブラリ | `swbt-rs`、`--no-default-features` | `GamepadStatus`を含むlibraryがbuildでき、`tracing` dependencyとstable diagnostics emitterを含まない | profile処理のため`serde_json`は残る |
| Bumble feature | `swbt-rs --features bumble` | adapter/runtimeと必要最小限のdebug tracingがbuildできる | stable diagnostics schema は保証しない |
| diagnostics feature | `swbt-rs --features diagnostics-schema` | schema v1 eventを従来と同じtarget、field、値でemitする | public schema型は追加しない |
| probe CLI | `cargo run -p swbt-probe -- <command>` | 現行の6 command、usage、終了コード、JSON／NDJSON契約を維持する | `probe` featureとroot binaryは削除する |
| trace failure | traceのcreate、subscriber install、write、flush失敗 | probe固有の分類済みerrorとして終了1になる | 公開`swbt::ErrorKind`へtool errorを追加しない |
| runner入口 | `cargo run -p swbt-hardware-runner -- <scenario> ...` | `pro-periodic`、`pro-profile`、`joycon-profile`を一つのbinaryから選べる | helpとusage errorは実機を開かない |
| runner引数 | 各scenarioの既存flag、重複、欠落、範囲外 | 現在と同じ有効集合を受理し、無効入力は終了2にする | scenario名を除く引数の意味を変えない |
| runner evidence | 有効scenarioを実行する | 対応する`swbt.m5.pro-periodic`、`swbt.m6.pro-profile`、`swbt.m7.joycon-profile` schemaを出す | 保存済みevidenceとの意味を維持する |
| 秘密値保護 | adapter selector、profile path、local address、key、serial、raw packet、source errorが存在する | Debug、stdout、stderr、trace、evidenceに値を出さない | error kindや安全な件数・状態は出してよい |
| package archive | `cargo package -p swbt-rs --locked` | tool crate、probe CLI、hardware runner、実機 evidenceを含まない | `examples/type_model.rs`は残す |
| tool API boundary | tool crateをbuildする | 公開`swbt` APIだけでbuildできる | libraryの`pub(crate)`をtool都合で公開しない |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | Cargo metadataがroot library、private probe、private hardware runnerの3 packageをworkspace memberとして返し、package未指定ではroot packageを選ぶ | new | package | red: root packageだけだった。green: metadata検査scriptが3 package、root default member、toolの`publish = false`を確認。package境界だけの最小変更なので追加refactorなし |
| refactor-skipped | private `swbt-probe` が現行command、終了コード、profile安全出力、trace schema検査を公開`swbt` APIだけで維持する | regression | integration | red: private packageにbinaryと依存がなくCLI testがcompile失敗。green: unit 11件、CLI integration 8件成功。rootの`probe` feature、binary、`ErrorKind::Trace`を除去。既存test seamを維持でき、追加refactorなし |
| refactor-done | featureなしのlibraryは`GamepadStatus`を維持してstable diagnosticsをemitせず、feature有効時はschema v1 eventを従来どおりemitする | regression | unit | red: default graphに`tracing`があり、新record framingをsubscriberが拒否。green: feature graph検査、default/all-feature library、probe test成功。refactor: `to_value()`をproduction emitにも使い、field組立てを一箇所へ統合 |
| refactor-skipped | `swbt-hardware-runner` が三つのscenarioと既存flag集合を単一entry pointで受理し、欠落・重複・不正な組み合わせを実機open前に終了2で拒否する | new | unit | red: entry parser未実装でcompile失敗。green: scenario/parser 12件、CLI integration 2件成功。既存操作をそのままmodule移動し、共通化は次itemへ分離したため追加refactorなし |
| refactor-done | 各runner scenarioが既存の操作列、status、profile postflight、adapter reopen、evidence schemaを維持し、共通出力が秘密値を含まない | regression | unit | green: 既存14件を維持し、共通引数解析の回帰testを加えた15件が成功。refactor: key/value引数、evidence基底項目、status/error射影、adapter reopenを`support`へ集約。機種別profile検査と入力列はscenarioに維持。実機I/Oは明示承認後の別gate |
| refactor-skipped | 公開`swbt-rs` archiveがtool専用source/testとhardware runnerを含まず、展開後にdefault/all-feature buildとtestが成功する | regression | package | green: 113 files / 1.3 MiB（圧縮224.9 KiB）。`tools/`、`src/bin/`、旧実機exampleは0件。展開archiveのdefault/all-feature testとbuildをofflineで確認。package includeは既に最小境界だったため追加refactorなし |
| refactor-skipped | workspace CI command、README、platform support、troubleshootingの実行例が新しいpackageと単一runner入口を指す | regression | docs | green: 二つのtool helpを実行し、旧feature/bin/example command残りを検索。CIはworkspace gateとroot限定gateを明示。docs reviewでrepository checkout限定とarchive非収録を明記。文章変更のみなので追加refactorなし |

## 7. 設計メモ

### 7.1 workspace

root `Cargo.toml` は `[package]` を維持したまま `[workspace]` を追加する。`default-members = ["."]` とし、既存のrepository root commandが意図せず実機toolまで対象に広がらないようにする。workspace全体のgateでは`--workspace`を明示する。

### 7.2 diagnostics

`GamepadStatus`だけではsubcommand順序、各accepted reportの時刻、session終了系列を復元できない。stable evidenceの意味を保つため、event生成はlibrary内部に残す。ただし`diagnostics-schema` featureがないbuildではno-op emitterを使い、schema実装をコンパイルしない。

schema producerとprobe subscriberの契約検査は必要だが、test専用`to_value()`による第三のfield組立ては除去する。library側は実際の`tracing` emitを検査し、probe側はsubscriberが許可fieldだけをNDJSONへ保存することを検査する。

### 7.3 単一 hardware runner

binary名は`swbt-hardware-runner`とする。

```text
swbt-hardware-runner pro-periodic ...
swbt-hardware-runner pro-profile ...
swbt-hardware-runner joycon-profile ...
swbt-hardware-runner help
```

scenario subcommandを残す理由は、三つの証跡schema、pair／reconnect前提、profile事前条件、入力sequenceが異なるためである。entry point、引数処理の枠組み、出力、伏字、status、reopen処理は共通化するが、違いを一つの巨大な設定enumへ隠さない。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `Cargo.toml` / `Cargo.lock` | modify | workspace、feature、binary、dependency境界 |
| `src/bin/swbt-probe.rs` / `src/bin/swbt-probe/**` | delete | private tool crateへ移動 |
| `src/diagnostics/event.rs` / `src/runtime/status.rs` | modify | feature付きschema emitterと通常buildのno-op |
| `src/error.rs` | modify | `ErrorKind::Trace`削除 |
| `tests/probe_cli.rs` | delete | probe crateのintegration testへ移動 |
| `examples/pro_periodic_hardware.rs` | delete | 単一runner scenarioへ移動 |
| `examples/pro_profile_hardware.rs` | delete | 単一runner scenarioへ移動 |
| `examples/joycon_profile_hardware.rs` | delete | 単一runner scenarioへ移動 |
| `tools/swbt-probe/**` | new | probe package、CLI、subscriber、test |
| `tools/swbt-hardware-runner/**` | new | 単一binary、scenario、共通support、test |
| `.github/workflows/ci.yml` | modify | root packageとworkspace gateの区別 |
| `README.md` / `CHANGELOG.md` / `docs/*.md` / crate rustdoc | modify | 現在の入口、feature、配布範囲、非互換変更へ更新 |
| `spec/publishing.md` | modify | workspace後のpackage commandとarchive境界 |

## 9. 検証

| command | result | notes |
|---|---|---|
| `.\tools\check-workspace.ps1` | success | redは`workspace packages differ: swbt-rs`。greenは`workspace contract passed` |
| `cargo test -p swbt-probe --all-targets --locked` | success | unit 11件、CLI integration 8件 |
| `cargo clippy -p swbt-probe --all-targets --locked -- -D warnings` | success | warningなし |
| `cargo run -p swbt-probe --locked -- help` | success | workspace packageのbinary入口と6 commandを確認 |
| `cargo test -p swbt-rs --all-targets --all-features --locked` | success | library 269 passed / 1 ignored、hardware 5 ignored、profile compatibility 1 ignored、他target成功 |
| `.\tools\check-library-features.ps1` | success | featureなしgraphに`tracing`なし、`diagnostics-schema` graphに`tracing`あり |
| `cargo test -p swbt-rs --all-targets --no-default-features --locked --quiet` | success | library 253 passed / 1 ignored、profile compatibility 1 ignored、他target成功 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | 3 package、warningなし |
| `cargo test --doc -p swbt-rs --all-features --locked` | success | doctest 1件 |
| `cargo build -p swbt-rs --no-default-features --locked` | success | featureなしlibrary build |
| `cargo test -p swbt-hardware-runner --all-targets --locked` | success | scenario/parser/support 13件、単一binary CLI integration 2件。schema v1、scenario次元、秘密値非出力、従来のM5 error分類を確認 |
| `cargo clippy -p swbt-hardware-runner --all-targets --locked -- -D warnings` | success | warningなし |
| `cargo run -p swbt-hardware-runner --locked -- help` | success | 単一binaryから3 scenarioを案内し、実機を開いていない |
| `cargo fmt --all --check` | success | workspace全体の整形差分なし |
| `cargo +1.87.0 check --workspace --all-targets --all-features --locked` | success | MSRVで3 packageの全targetを確認 |
| `cargo check --workspace --all-targets --all-features --locked` | success | 3 packageの全target |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | success | workspace全体、warningなし |
| `cargo test --workspace --all-targets --all-features --locked` | success | root 269 passed / 1 ignored、hardware 5 ignored、profile compatibility 1 ignored、probe 19件、runner 15件、他target成功 |
| `cargo test -p swbt-rs --all-targets --locked` | success | featureなしlibrary 253 passed / 1 ignored、profile compatibility 1 ignored、他target成功 |
| `cargo build --workspace --all-features --locked` | success | Cargo metadata変更後の3 package build |
| `cargo build -p swbt-rs --no-default-features --locked` | success | 通常利用境界 |
| `cargo tree -p swbt-rs --no-default-features --edges normal --locked` | success | `atomic-write-file`、`fs2`、`serde_json`だけで、`tracing`、Bumble、`rusb`なし |
| `cargo test --doc --workspace --all-features --locked` | success | root doctest 1件、tool doctest 0件 |
| `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --no-deps --workspace --all-features --locked` | success | 公開libraryとworkspace toolの文書warningなし |
| `cargo run -p swbt-probe --locked -- help` | success | 6 commandを確認、実機を開いていない |
| `cargo run -p swbt-hardware-runner --locked -- help` | success | 単一binaryから3 scenarioを確認、実機を開いていない |
| `cargo package -p swbt-rs --locked --list` | success | 113 files。`tools/`、`src/bin/`、旧実機exampleは0件 |
| `cargo package -p swbt-rs --locked` | success | 113 files / 1.3 MiB、圧縮224.9 KiB。archive verify成功 |
| 展開archiveのdefault/all-feature buildとtest（`--offline`） | success | default 253 passed / 1 ignored、all-feature 269 passed / 1 ignored。実機test 5件とprofile compatibility 1件はignore。両feature構成のbuild成功 |
| `cargo deny --locked check` | not run | local環境に`cargo-deny`が未導入。CIのdependency-policy jobは更新対象外で、remote CIは未実行 |
| `git diff main...HEAD --check` | success | whitespace errorなし |
| Windows実機でのPeriodic reconnect 2件とfresh pairing 1件 | failed | 2026-08-02。保存鍵付きPro profile 2件はどちらも`NoBond`で接続前に停止し、外部SHA-256比較でprofile不変。fresh pairingは120秒で`ConnectionTimeout`。失敗後の176-byte空profileは型検査に成功し、adapter reopen、closed lifecycle、neutral snapshotを確認。Ready、入力送信、neutral closeの実機経路とSwitch UIは未到達 |

## 10. 先送り事項

- Linux／macOS実機検証は本work unitでは行わない。既存のplatform support境界を維持する。
- `swbt-probe` と `swbt-hardware-runner` の統合は、利用目的と出力契約が異なるため対象外とする。
- 公開済み`ErrorKind::Trace`の削除はdownstream source互換性を失うため、次のcrates.io公開は`0.2.0`以降とする。本work unitではversion更新や公開を行わない。

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test List を更新した
- [x] probeとrunnerが公開APIだけでbuildできる
- [x] featureなしのlibraryがprobe専用schemaとsubscriberをコンパイルしない
- [x] runnerの単一entry pointと既存scenario操作を維持した
- [x] 秘密値をtrace／evidence／errorへ出さない
- [x] 公開packageの収録内容と依存境界を縮小した
- [x] README、docs、CI、publishing手順を更新した
- [x] 検証結果または未実行理由を記録した
- [x] package / release / public API に触れる場合の gate を記録した
