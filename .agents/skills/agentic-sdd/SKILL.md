---
name: agentic-sdd
description: "この Rust repo の Agentic Spec-Driven Development を開始、継続する入口 skill。ユーザが Agentic SDD、次の作業単位、spec/initial からの実装、spec/wip の plan/tasks/implement/gate loop、Intent Delta、TDD と gate を含む進行を求めるときに使う。"
---

# Agentic SDD

`AGENTS.md`、`SKILLS.md`、`spec/initial/*.md`、対象の `spec/wip` を Constitution として読み、次の作業単位を 1 つだけ選んで進める。

## Bootstrap

1. `AGENTS.md` と `SKILLS.md` を読む。
2. 関連する `spec/initial/*.md` を読む。
3. 既存 `spec/wip/unit_*` と `spec/complete/unit_*` を確認する。
4. 変更を伴う作業では branch と `git status --short` を確認する。
5. ユーザ入力から Intent Delta を抽出する。なければ `none`。
6. 実装済み範囲と既存 test を確認する。
7. 次の作業単位を 1 つだけ選ぶ。

開始時は短く提示する。

```text
Agentic SDD bootstrap:
- Constitution:
- Git Context:
- Intent Delta:
- Selected Work:
- Non-goals:
- Gates:
```

## Work Selection

| 候補 | 選択条件 |
|---|---|
| roadmap / initial spec | `spec/initial` に明示された優先順がある。 |
| spec TDD item | `spec/wip` の TDD Test List に小さい振る舞いがある。 |
| release / package item | Cargo metadata、CI、crates.io、公開 API に影響する。 |
| cleanup item | green 後の責務分離、命名、重複除去が必要。 |

- 選択していない work unit や対象外機能は実装しない。
- 大きい作業は TDD item へ分割する。
- 仕様なしで大きな実装へ入らない。
- Cargo metadata、CI、release workflow に触れる場合は `cargo package` と `cargo build --all-features` を gate に含める。

## Implementation Loop

1. 作業仕様がなければ `spec-format` で `spec/wip` を作る。
2. TDD が適する場合は `tdd-workflow` を使う。
3. red の理由が期待した失敗であることを確認する。
4. green を作る。
5. green 後に必要な構造変更だけ `tidy-first` と `refactor-after-green` で分ける。
6. gate 失敗時は、Spec、Plan、Tasks、Implement のどこへ戻るか決める。
7. 終了時は `agentic-self-review` で gate と未検証リスクを整理する。

## Git Context

- read-only 調査では branch 作成は不要。
- 作業 branch 上で clean ならそのまま進める。
- default branch 上で clean なら、変更前に作業 branch を作るかユーザ指示を確認する。
- dirty worktree では既存変更を読み、ユーザ変更を破棄しない。
- remote 未設定なら `pr-merge-cleanup` は PR 作成前に止まる。
