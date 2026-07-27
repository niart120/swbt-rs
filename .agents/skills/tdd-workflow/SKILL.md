---
name: tdd-workflow
description: "この Rust repo の spec/wip、spec/initial、TDD Test List から Canon TDD を進める orchestration skill。ユーザが TDD、テストリスト、red/green/refactor、仕様から実装への進行を求めるときに使う。"
---

# TDD Workflow

`spec-format`、`tdd-test-list`、`tdd-one-cycle`、`refactor-after-green` を接続する。

## Git Context

- 変更前に branch と `git status --short` を確認する。
- default branch への直接 commit はユーザの明示指示がある場合を除き避ける。
- dirty worktree では既存変更を読んで、ユーザ変更を破棄しない。

## Workflow

1. 関連する `spec/initial/*.md` と `spec/wip` を読む。
2. 作業仕様がなければ `spec-format` で作る。
3. `tdd-test-list` で振る舞いベースの item に分ける。
4. 次に扱う item を 1 つだけ選ぶ。
5. `tdd-one-cycle` で red / green / refactor を進める。
6. green 後の構造変更は `refactor-after-green` と `tidy-first` で behavior change と分ける。
7. test quality に迷う場合は `test-desiderata-review` を使う。
8. その item の test、実装、必要な refactor、TDD Test List の状態更新を 1 つの論理変更として commit する。commit 前に対象 item の検証を実行し、結果を記録する。
9. 次の item がある場合は 4 へ戻る。複数 item を 1 commit にまとめない。
10. Test List をすべて消化し、仕様の完了条件と必要な gate を満たしたら、`pr-merge-cleanup` を使って PR 作成、merge、default branch 同期、branch cleanup を行う。

## Rules

- red から green の途中で見つけた別の振る舞いは list に追加し、今の item に混ぜない。
- refactor は green 後に行う。
- formatter / linter だけを refactor と呼ばない。
- Cargo metadata に触れたら `cargo build --all-features` と必要に応じた `cargo package` を追加する。
- Test List の item を完了扱いにするのは、その item に対応する検証が成功し、commit が作成された後だけにする。
- docs / spec の item は、リンク、参照先、frontmatter、構文などの機械的契約か、実行可能な製品契約に限る。説明文の固定文字列、見出し、禁止語、checkbox、状態語を green の条件にしない。
