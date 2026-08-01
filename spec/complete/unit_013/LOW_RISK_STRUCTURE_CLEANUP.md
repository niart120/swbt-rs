# 低リスク構造整理 仕様書

## 1. 概要

### 1.1 目的

外部入力、非同期状態遷移、永続化の安全境界を維持したまま、重複する値検査、crate内定数の利用時検査、
本番専用の重複adapter、意味を追加しない中継関数と内部型、過去の作業番号に依存する
`dead_code`抑制を削除する。crate root rustdocは公開契約へ絞る。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue | 低リスクな重複検査と薄い抽象化の削除 | `https://github.com/niart120/swbt-rs/issues/17` |
| user request | 後方互換性を過度に優先せず、不要な内部境界を積極的に削除する | 2026-08-02の対話 |
| initial spec | 公開API、module責務、入力値、テスト方針 | `spec/initial/api.md`、`architecture.md`、`testing.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| `Stick`利用者 | raw、normalized、方向量 | 各入力を一度だけ検査してraw軸へ変換する | 非有限値と各constructorの範囲外を拒否する |
| controller builder | profile path | 共通file storeで読み込み、同じtyped errorへ分類する | create-new/update契約を変えない |
| runtime | press、release、tap、neutral | 候補生成とreporting別commit順を維持する | Directは送信受理後だけcommitする |
| crate利用者 | crate root rustdoc | 現在の機能、feature、lifecycle、制約を確認できる | 実機run履歴は専用docs/evidenceで扱う |

## 2. 対象範囲

- `Stick`の方向量とnormalized軸を単一の検査・変換経路へ統合する
- model/HIDのcrate内定数に対する本番経路の`is_well_formed()`検査を削除する
- `FileProfileReader`を`FileProfileStore`へ統合する
- `neutral_candidate()`、`commit_candidate()`など意味を追加しない中継関数を削除する
- `TapPlan`とそのaccessorを削除し、検査結果を利用箇所で直接分解する
- 過去のTDD itemまたはmilestone番号だけを理由にした`dead_code`抑制を削除または現行feature条件へ限定する
- backendから受け取った後に本番判断へ使わないcontroller version、USB metadata、transport設定値を削除する
- testだけが呼ぶ診断eventと将来用lifecycle helperを削除する
- crate root rustdocから実機run履歴を除き、公開契約中心へ縮める

## 3. 対象外

- HID output report、profile JSON、Bluetooth address、pairing keyの外部入力検査
- worker command経路とqueue契約
- profile保存のcreate-new、atomic replace、lock契約
- PeriodicとDirectのstate commit順序
- stale session eventの破棄
- interrupt drain、disconnect、transport close、joinの順序
- Issue #17と無関係な公開API再設計

## 4. 関連 docs

- `spec/initial/architecture.md`
- `spec/initial/api.md`
- `spec/initial/testing.md`
- `spec/initial/QUALITY_GATES.md`
- `docs/platform-support.md`

## 5. 振る舞い仕様

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| stick変換 | constructorごとの許容範囲 | 有限性、範囲、符号適用、raw変換を一経路で行う | error messageはsemver契約にしない |
| model定数 | crate内のmodel/HID宣言 | 利用時検査を行わず、fixtureとmodel testで定義を検査する | descriptor長は配列型で固定する |
| profile read | existing profile path | `FileProfileStore`が読み、既存のerror kindへ分類する | fake `ProfileReadPort`はtest seamとして残せる |
| semantic input | neutral、tap | 一行wrapperや専用plan型を経由せず同じ候補と期限を使う | 検査とcommit順は残す |
| lint境界 | default / all-features build | 必要な`dead_code`抑制だけが現行feature条件と理由を持つ | 将来予定だけのcodeは残さない |
| crate docs | rustdoc閲覧 | API能力、feature、状態確定、制約が主になる | 実機履歴は`docs/`と`spec/complete/*/evidence`に置く |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| done | Stickのraw値、normalized境界、方向量境界、非有限値が単一変換経路でも成立する | characterization | integration | 4方向helperを同じ境界caseで検査した |
| done | model宣言とHID serviceがPython fixtureおよびmodel invariantに一致する | characterization | unit / integration | 利用時assertを削除し、fixture testを通した |
| done | profile buildのread順とtyped error分類が共通file storeへの統合後も成立する | regression | unit | fake read portを維持し、本番readerだけを統合した |
| done | Periodic/Directのneutral、tap、send失敗時commit結果が内部plan型削除後も成立する | regression | unit / integration | Direct受理後commitとPeriodic先行commitのtestを通した |
| done | default/all-featuresのclippyが過去番号に依存する抑制なしで成功する | regression | package | 番号由来の理由を0件にし、feature条件だけを残した |
| done | crate root rustdocが公開契約を説明し、実機run履歴を重複保持しない | docs | docs | 実機履歴を削除し、platform文書への案内を残した |

## 7. 設計メモ

- 構造変更が中心だが、内部error messageとprivate型の互換性は維持対象にしない。
- 値域はconstructorごとに異なる。検査関数を重ねるのではなく、許容範囲と方向を一つの変換関数へ渡す。
- `TapPlan`削除後も、空buttonとtap durationはtransport送信前に検査する。
- `dead_code`抑制は一括で文言だけ変更せず、削除後のdefault/all-features compiler結果で必要性を判定する。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `src/input/stick.rs` | modify | 単一検査・変換経路 |
| `src/model/mod.rs`、`src/model/hid.rs` | modify | 利用時well-formed検査削除 |
| `src/controller/build.rs`、`src/controller/mod.rs` | modify | file reader統合 |
| `src/controller/input.rs`、`src/runtime/{periodic,direct,worker}.rs` | modify | 一行wrapperとTapPlan削除 |
| `src/runtime/transport/*.rs` | modify / delete | 本番判断に使わないcapability/config投影を削除 |
| `src/adapter.rs`、`src/diagnostics/event.rs`、`src/runtime/lifecycle.rs` | modify / delete | backend重複parser、test専用event、将来用helperを削除 |
| `src/**/*.rs` | modify | stale `dead_code`抑制を削除し、feature条件だけへ限定 |
| `src/lib.rs` | modify | 公開契約と検証記録の分離 |
| `docs/platform-support.md` | reference | 実機環境と制約は既存記録を正本として変更しない |

## 9. 検証

| command | result | notes |
|---|---|---|
| `cargo fmt --all --check` | success | rustfmt差分なし |
| `cargo clippy --all-targets --locked -- -D warnings` | success | default feature境界 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | success | 全feature境界 |
| `cargo check --all-targets --all-features --locked` | success | 全targetをcompile確認 |
| `cargo test --all-targets --locked` | success | default feature全target。手動相互運用testは既存注記どおりignored |
| `cargo test --all-targets --all-features --locked` | success | fixture、profile、runtime、probeを含む。手動・実機testは既存注記どおりignored |
| `cargo build --locked` | success | 通常利用 |
| `cargo build --all-features --locked` | success | 全feature production経路 |
| `cargo test --doc --all-features --locked` | success | crate root doctest 1件成功 |
| `git diff --check` | success | whitespace errorなし |

## 10. 先送り事項

- 実機I/Oは構造整理の対象外であり、今回再実行していない。既存証跡の正本は`docs/platform-support.md`と
  `spec/complete/unit_006`から`unit_012`に置く。

## 11. Self-review

| severity | finding | evidence | disposition |
|---|---|---|---|
| none | Issue #17で保護対象とされた外部入力、profile atomicity、commit順、stale event、cleanup順に契約逸脱なし | 全target test、対象diffの読解 | accepted |
| none | 削除したcapability metadataとtransport設定値はbackend初期化後の本番判断に使われていなかった | production利用箇所検索、all-feature compile/test | deleted |
| none | 公開signatureとCargo metadataは不変。Stick error文言だけは内部契約どおり非固定 | `git diff`、`spec/initial/api.md` | accepted |
| low | 実USB/console経路は未実行 | 対象外。仮想runtime、Bumble adapter test、既存実機証跡を分離 | retained risk |

## 12. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを更新した
- [x] 検証結果または未実行理由を記録した
- [x] package / release / public APIに触れる場合のgateを記録した
