---
name: agentic-self-review
description: "この Rust repo の仕様変更、実装、TDD cycle、PR 前に品質 gate、未実行テスト、未検証リスク、Subagent 指摘の採否を整理する self-review skill。作業完了前、handoff、PR 本文準備、gate 結果報告で使う。"
---

# Agentic Self Review

完了宣言ではなく、何が確認済みで何が未検証かを人間が判断できる形に圧縮する。

## Process

1. 対象の `spec/wip`、Intent Delta、non-goals を確認する。
2. diff と仕様の明示要件を照合する。
3. 実行した command、validator、test、hook を evidence として記録する。
4. 未実行 gate は `not run`、対象外は `not applicable` と書く。
5. docs / spec / skill / PR body を変更した場合は `docs-quality-review` で文言、置き場所、根拠を確認する。
6. docs / spec / skill だけの変更でも、仮テキスト残りと skill validation を確認する。
7. 問題がある場合は findings を先に出す。
8. 問題がない場合も、残る test gap と未検証事項を明記する。

## Gates

| Gate | Evidence |
|---|---|
| Requirements | `AGENTS.md`、`spec/initial`、対象 spec との照合。 |
| Scope | 対象範囲、対象外、先送り事項。 |
| TDD / Tests | red / green 履歴、`cargo test` 結果、未実行理由。 |
| Static | `cargo fmt --check`、`cargo clippy`、skill validation。 |
| Package | `cargo build`、`cargo package`、Cargo metadata 検査。 |
| Integration Review | diff、scope drift、指摘の採否。 |

## Report

```markdown
## Agentic SDD Report

### Work
- spec:
- intent delta:
- non-goals:

### Findings
| severity | finding | evidence | disposition |
|---|---|---|---|

### Gates
| gate | result | evidence |
|---|---|---|

### Next
- deferred:
- next candidate:
```

## Rules

- evidence が弱い項目を pass にしない。
- docs / skill のみ変更では Rust test を省略できるが、skill validation と residue check は実行する。
- README や公開 docs の変更では、agent 向けの手順や作業履歴が混ざっていないか確認する。
- Cargo metadata に触れた場合は `cargo build --all-features` を省略しない。
