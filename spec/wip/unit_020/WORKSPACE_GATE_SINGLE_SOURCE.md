# workspace gate 単一正本化仕様書

## 1. 概要

### 1.1 目的

Cargo workspace の構成を `Cargo.toml` と別の固定 package 一覧へ複製する検査を廃止する。
CI とローカルでは対象を `--workspace` で明示し、core/runtime の依存境界は既存の
repository-local scriptを単一の実装として利用する。

### 1.2 起点 / source

| source | 内容 | path |
|---|---|---|
| GitHub Issue #30 | `check-workspace.ps1` が `swbt-core` 分離前のworkspace構成を要求し、現行mainで失敗する | `https://github.com/niart120/swbt-rs/issues/30` |
| user decision | workspace構成の固定検査自体を廃止し、Cargo manifestを正本とする | 2026-08-02の対話 |
| unit_015 | tool package分離時に3 package構成の検査を導入した | `spec/complete/unit_015/TOOLING_CRATE_SEPARATION.md` |
| unit_019 | `swbt-core` 分離とcore/runtime依存境界の現行契約を定義した | `spec/complete/unit_019/CORE_PACKAGE_AND_REQUIRED_RUNTIME_BACKEND.md` |

### 1.3 use case

| actor / boundary | 入力または状態 | 期待する観測結果 | 制約 |
|---|---|---|---|
| maintainer | workspace packageを追加、削除、変更する | `Cargo.toml`だけをworkspace構成の正本として更新できる | package名を別の検査へ転記しない |
| local developer | 通常の品質gateを実行する | default membersに依存せずworkspace全targetを検査する | 実機testはignoredのまま |
| CI | package-boundary jobを実行する | repository-local scriptと同じcore/runtime依存境界を検査する | BashとPowerShellで契約を二重実装しない |
| release maintainer | 公開packageを検査する | package別の`cargo package`で公開可能性とarchiveを検査する | publish設定をworkspace構成検査へ混在させない |

## 2. 対象範囲

- `tools/check-workspace.ps1`を削除する
- CIのcore/runtime依存graph検査を`tools/check-library-features.ps1`呼び出しへ置換する
- 通常のローカル品質gateでCargo commandへ`--workspace`と必要なtarget/lock条件を明示する
- operational docsから削除したscriptへの参照がないことを確認する
- Cargo workspace構成、dependency graph、公開packageの既存gateを実行する

## 3. 対象外

- workspace member、`workspace.default-members`、`publish`設定の変更
- package名またはpackage数を固定する新しい検査
- `check-library-features.ps1`の改名または依存境界の変更
- Rust実装、公開API、feature、versionの変更
- crates.io publish、tag、GitHub Release
- 完了済みwork unitに記録された過去の検証結果の書き換え

## 4. 関連 docs

- `AGENTS.md`
- `spec/initial/QUALITY_GATES.md`
- `spec/initial/testing.md`
- `spec/publishing.md`
- `spec/complete/unit_015/TOOLING_CRATE_SEPARATION.md`
- `spec/complete/unit_019/CORE_PACKAGE_AND_REQUIRED_RUNTIME_BACKEND.md`

## 5. 振る舞い仕様

### 5.1 Intent Delta

Issue #30が提示した案A、案Bはいずれもworkspace package一覧、default members、公開可否を
Cargo manifestとは別の検査へ複製する。採用方針では、workspace構成の固定検査を削除し、
`Cargo.toml`を唯一の正本とする。

| 振る舞い | 入力・状態 | 期待結果 | 備考 |
|---|---|---|---|
| local workspace gate | repository rootで通常gateを実行する | core、runtime、toolを明示的に対象とする | default membersを品質保証の前提にしない |
| dependency boundary gate | CIのpackage-boundary jobを実行する | `check-library-features.ps1`がcore非依存条件とruntime必須依存を検査する | CI内に同じpackage名・正規表現を複製しない |
| workspace構成変更 | `Cargo.toml`のmember/default/publishを変更する | Cargo commandとpackage/release reviewで影響を確認する | 固定package一覧との同期作業を要求しない |
| obsolete entry point | `tools/check-workspace.ps1`を探す | operational entry pointとして存在しない | 完了済みspecの履歴参照は保持する |

## 6. TDD Test List

| status | item | type | layer | notes |
|---|---|---|---|---|
| refactor-skipped | T01 CIのpackage-boundary jobがrepository-local dependency boundary scriptを実行し、同じ依存契約のBash実装を持たない | regression | CI / package | CI command契約とscript実行が成功。重複削除後の追加refactorなし |
| refactor-skipped | T02 通常のlocal gateがworkspace全体を明示的に対象とし、旧workspace検査がoperational surfaceに残らない | regression / cleanup | package / docs | command契約、file不在、operational参照不在を確認。追加refactorなし |

status は `todo`、`red`、`green`、`refactor-done`、`refactor-skipped`、`deferred` を使う。

### 6.1 TDD cycle evidence

| phase | item | evidence |
|---|---|---|
| red | T01 | CI command契約のPowerShell assertionは`package-boundary job does not invoke check-library-features.ps1`で失敗した |
| green | T01 | CIのinline Bash処理を`pwsh ./tools/check-library-features.ps1`へ置換した。command契約のassertionとscript実行が成功した |
| refactor-skipped | T01 | package名、依存名、正規表現のCI重複は削除済みで、追加の構造変更は不要と判断した |
| red | T02 | PowerShell assertionは旧scriptの存在と、`AGENTS.md`、`QUALITY_GATES.md`にworkspace明示commandがないことを検出した |
| green | T02 | 旧scriptを削除し、両文書のfmt/clippy/test/build commandをworkspace構成へ同期した。file不在、command契約、operational参照不在を確認した |
| refactor-skipped | T02 | workspace構成を別helperへ移さずCargo manifestを正本にするため、追加の構造変更は行わなかった |

## 7. 設計メモ

- workspace package一覧の固定検査は、`Cargo.toml`と独立した正本にならず、正当な構成変更でも更新漏れを起こす。
- `cargo check/test/clippy --workspace`はCargoが認識するmemberを対象とする。未登録directoryの探索やpackage数の固定は本unitの契約にしない。
- default membersは引数なしCargo commandの利便性として残すが、品質gateの検査範囲には使用しない。
- tool packageの`publish = false`はrelease policyであり、workspace構成検査へ混在させない。公開対象はpackage別gateで検査する。
- `check-library-features.ps1`はworkspace構成ではなく依存graphのarchitecture contractを検査するため残す。

## 8. 対象ファイル

| path | change | 内容 |
|---|---|---|
| `.github/workflows/ci.yml` | modify | inline Bash依存graph検査をrepo-local script呼び出しへ置換 |
| `tools/check-workspace.ps1` | delete | Cargo manifestと重複する固定workspace検査を削除 |
| `AGENTS.md` | modify | 通常gateのworkspace対象を明示 |
| `spec/initial/QUALITY_GATES.md` | modify | local gateのworkspace対象、all-targets、lock条件を明示 |
| `spec/wip/unit_020/WORKSPACE_GATE_SINGLE_SOURCE.md` | new / modify | Intent Delta、TDD状態、検証結果を記録 |

## 9. 検証

| command | result | notes |
|---|---|---|
| CI command契約のPowerShell assertion | success | T01 redはrepo-local script未呼出しで失敗。greenはscript呼出しとinline実装不在を確認 |
| `pwsh -NoProfile -File ./tools/check-library-features.ps1` | success | `core/runtime package boundary passed` |
| local gate command契約、旧script不在、operational参照のPowerShell assertion | success | redは旧scriptと暗黙gateを検出。greenは両文書のcommand、file不在、参照不在を確認 |
| `cargo fmt --all --check` | not run | workspace formatting |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | not run | workspace static gate |
| `cargo test --workspace --all-targets --all-features --locked` | not run | workspace test gate |
| `cargo build --workspace --all-features --locked` | not run | CI/package変更に対するbuild gate |
| `cargo package -p swbt-core --locked` | not run | 公開core archive gate |
| `cargo package -p swbt-rs --locked` | not run | 公開runtime archive gate |
| `git diff --check` | not run | whitespace gate |
| docs-quality-review | not run | 対象文書の役割、事実、仮テキスト、参照を確認する |
| agentic-self-review | not run | requirements、scope、gate、未検証事項を確認する |

## 10. 先送り事項

- none

## 11. チェックリスト

- [x] 対象範囲と対象外を確認した
- [x] TDD Test Listを作成した
- [x] CIの依存境界検査をrepo-local scriptへ一本化した
- [x] workspace構成の固定検査を削除した
- [x] 通常gateがworkspace全体を明示的に対象とする
- [ ] 検証結果または未実行理由を記録した
- [x] CI/package変更に必要なbuild/package gateを記録した
- [ ] docs-quality-reviewとagentic-self-reviewを完了した
