---
name: tidy-first
description: "この Rust repo で振る舞い変更と構造変更を分ける skill。TDD 中の実装、public API、Cargo metadata、test fixture、docs/skill の整理で、先に構造を整えるか、green 後に refactor するか、変更を分割するか判断するときに使う。"
---

# Tidy First

振る舞い変更と構造変更を同じ判断に混ぜない。

## 分類

| 種別 | 意味 |
|---|---|
| behavior change | API、CLI、例外、出力、package metadata、test の期待結果を変える。 |
| structure change | 観測可能な振る舞いを変えず、命名、分割、重複、責務境界、test fixture を整える。 |

## 判断

- behavior change のために小さい構造整理が必要なら、先に tidy を小さく入れてから behavior change へ進む。
- green 後に重複や責務混在が見えたら `refactor-after-green` を使う。
- package metadata や public API に触れる変更は、構造変更に見えても observable behavior への影響を確認する。
- 大きい整理は今の TDD item に混ぜず `spec/dev-journal.md` または次の `spec/wip` 候補に残す。

## Output

```text
Tidy decision:
- classification: behavior | structure | mixed
- action: tidy-first | after-green | split | defer
- reason:
- verification:
```
