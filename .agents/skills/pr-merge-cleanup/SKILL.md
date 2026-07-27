---
name: pr-merge-cleanup
description: "この Rust repo の作業ブランチを GitHub plugin 経由の PR で default branch に取り込み、ローカル同期と branch cleanup まで行う skill。ユーザが PR 作成、マージ、ブランチ後片付け、main/master へ入れる、PR cleanup を依頼したときに使う。remote 未設定、default branch 上、dirty worktree、required check 失敗では停止する。"
---

# PR Merge Cleanup

作業ブランチの変更を GitHub plugin 経由の PR で default branch に取り込み、local sync と branch cleanup まで行う。

GitHub の認証、PR 作成、check 状態の確認、merge には接続済みの GitHub plugin を使う。`gh` コマンドは使わない。

## Preconditions

- `origin` remote が設定済み。
- default branch を確認できる。
- 作業ブランチ上である。
- `git status --short` が clean。
- 必要な commit が完了している。
- GitHub への push / PR / merge 権限がある。

remote 未設定の現段階では stop condition として扱い、push や PR 作成を試みない。

## Workflow

1. `git branch --show-current` を確認する。
2. `git remote get-url origin` と default branch を確認する。
3. default branch 上なら停止する。
4. `git status --short` が空であることを確認する。
5. `git log --oneline <default>..HEAD` で commit log を作る。
6. `agentic-self-review` の結果と実行 gate を PR 本文へ反映する。
7. `git push -u origin <branch>` で push する。
8. GitHub plugin で PR を作成する。PR 本文はこの skill の `PR Body` を満たす。
9. GitHub plugin で required check、mergeable state、未解決 review を確認する。
10. check が通り、mergeable state に問題がなければ GitHub plugin で merge する。勝手に squash / merge commit の方針を変えない。
11. default branch へ戻り、`git pull --ff-only origin <default>` で同期する。
12. local の作業 branch を削除する。GitHub plugin が remote branch の削除を提供する場合は remote branch も削除する。提供しない場合は、remote branch の削除は未実施として報告し、`gh` や認証を迂回する手段を使わない。
13. PR 番号、URL、merge commit、削除 branch、gate 状態を報告する。

## Stop Conditions

- `origin` remote がない。
- default branch 上にいる。
- dirty worktree がある。
- required check が未完了または失敗。
- mergeable state が blocked / dirty / unknown。
- PR 本文に gate の必須情報が不足している。

## PR Body

- Summary: 変更目的を 1-3 行で書く。
- Related: spec、docs、issue、作業指示元。
- Changes: file list ではなく論理単位。
- Commit Log: `<default>..HEAD`。
- Testing: command と結果。未実行 gate は理由。
- Agentic SDD Gates: used / not used と evidence。
- Review Notes: scope drift、先送り事項。
