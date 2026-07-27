---
name: test-desiderata-review
description: "この Rust repo のテスト設計を Kent Beck の Test Desiderata 観点で見直す skill。unit、integration、doc test、feature の分担、flaky risk、assertion 粒度、package smoke、CI gate に迷うときに使う。"
---

# Test Desiderata Review

テストの価値と trade-off を明示する。すべての性質を同時に最大化しようとしない。

## 観点

- isolated: 外部サービス、時計、環境変数、ネットワークに依存しないか。
- deterministic: 実行順、scheduler、locale、path に左右されないか。
- fast: 通常 gate に入れられる速さか。
- precise: 失敗時にどの契約が壊れたか分かるか。
- representative: public API、CLI、package behavior を十分に表しているか。
- maintainable: fixture と assertion が意図を読ませているか。

## 方針

- library の純粋ロジックは unit test を優先する。
- CLI や Cargo metadata は小さい smoke test に分ける。
- feature 組合せはコンパイル確認と必要な統合テストを分ける。
- network、crates.io、GitHub Actions など外部状態は通常 unit gate に入れない。
- 実行していない外部 smoke を pass と書かない。

## Output

```markdown
### Test Desiderata Review

| test | value | trade-off | decision |
|---|---|---|---|

### Gaps

- ...
```
