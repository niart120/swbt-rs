---
name: tdd-test-list
description: "この Rust repo の仕様、use case、public API、Cargo package behavior から振る舞いベースの TDD Test List を作成または更新する skill。ユーザがテストリスト、TDD item、red/green/refactor の候補を依頼したとき、または spec/wip の TDD Test List を扱うときに使う。"
---

# TDD Test List

実装前に、観測可能な振る舞いを小さい test item に分ける。

## Input

- `AGENTS.md`
- 関連する `spec/initial/*.md`
- 対象の `spec/wip/unit_XXX/FEATURE_NAME.md`
- 既存 test と実装状態

## Rules

- item は外部から観測できる入力、状態、期待結果で書く。
- 実装順、file list、内部 helper 名だけを item にしない。
- public API、CLI、package metadata、error behavior は観測面を明示する。
- docs layer の item は、Markdown link、参照先、frontmatter、構文、schema、生成物、実行可能な製品契約を対象にする。自然言語の固定文字列、見出し、説明順、禁止語、checkbox、状態語を item にしない。
- prose の正確さ、対象読者への説明、根拠、未検証表示は `docs-quality-review` によるレビュー項目とし、test の green に変換しない。
- 新しく見つけた振る舞いは、今の red/green に混ぜず list へ追加する。

## Item Table

```markdown
| status | item | type | layer | notes |
|---|---|---|---|---|
| todo |  | new / regression / edge / characterization | unit / integration / package / docs |  |
```

status:

- `todo`
- `red`
- `green`
- `refactor-done`
- `refactor-skipped`
- `deferred`

## Priority

1. 仕様の prerequisite を優先する。
2. unit test で固定できる振る舞いを先に扱う。
3. Cargo metadata、CI、release behavior は `cargo build`、`cargo package`、smoke を含めて別 item にする。
