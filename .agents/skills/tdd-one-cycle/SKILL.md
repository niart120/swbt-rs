---
name: tdd-one-cycle
description: "この Rust repo の TDD Test List から 1 item だけを選び、red、green、必要な refactor まで 1 cycle 進める skill。ユーザが TDD の 1 サイクル、red/green/refactor、テストを1つ消化、spec/wip の TDD item 実装を依頼したときに使う。"
---

# TDD One Cycle

TDD Test List の 1 item だけを扱う。複数 item をまとめて green にしない。

## Preconditions

- 対象 item が 1 つに絞られている。
- 関連 spec と docs を読んでいる。
- 変更前の git status を確認している。

## Cycle

1. item の観測対象と non-goals を確認する。
2. 失敗する test を先に書く。
3. red が期待した理由で失敗したことを確認する。
4. 最小の実装で green にする。
5. 同じ command で green を確認する。
6. 必要なら `refactor-after-green` を使う。
7. `spec/wip` の TDD Test List と検証欄を更新する。

## Verification

通常候補:

```powershell
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

対象 tree や project metadata がまだない場合は、未実行理由を spec に書く。

## Output

```text
TDD status:
- item:
- state: red | green | refactor-done | refactor-skipped
- command:
- notes:
```
