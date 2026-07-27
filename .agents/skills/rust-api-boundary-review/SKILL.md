---
name: rust-api-boundary-review
description: "この Rust repo の公開 API、所有権、lifetimes、error、async、feature flag、unsafe の境界をレビューする skill。ユーザが Rust の型、公開 API、borrow checker、Result、thiserror、async、Send/Sync、feature、unsafe の見直しを依頼したときに使う。"
---

# Rust API Boundary Review

利用者に見える契約と、所有権・並行性・失敗の境界を確認する。型を複雑にすること自体は目的にしない。

## 手順

1. `Cargo.toml`、公開 module、変更差分、関連 docs を読む。
2. `pub` の追加・変更について、引数、戻り値、error、状態変化、feature 条件を確認する。
3. 所有権、borrow、clone、`Send` / `Sync`、非同期 cancellation、lock と `.await` の境界を確認する。
4. `Result` の error variant が呼び出し側の回復・分岐に必要な情報を提供するか確認する。
5. `unsafe`、FFI、OS/ネットワーク I/O は安全性条件と入力検証を確認する。
6. `cargo clippy --all-targets --all-features -- -D warnings` と影響する test を実行する。

## 規則

- 実装詳細を不要に `pub` にしない。公開型の非網羅 enum 化、field の公開、trait object 化は互換性への影響を明示する。
- `String` だけで error を表現しない。呼び出し側が判定する失敗は variant または型にする。
- `Clone`、`Arc`、`Mutex` は borrow error を隠す逃げ道として導入しない。共有が必要な理由と競合条件を示す。
- `unsafe` は最小ブロックに閉じ、安全性条件を `// SAFETY:` で近接して説明する。
- feature 依存の API は、無効時の可視性・コンパイル・docs を確認する。

## 確認候補

```powershell
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Report

```markdown
### Rust API Boundary Review

| severity | file:line | boundary | finding | disposition |
|---|---|---|---|---|
```
