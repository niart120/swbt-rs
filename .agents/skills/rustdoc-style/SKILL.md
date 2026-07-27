---
name: rustdoc-style
description: "この Rust repo の公開 API rustdoc を、型・error・feature 条件・README/docs と整合させる skill。ユーザが rustdoc、/// コメント、公開 API 説明、Examples、Errors、Panics、Safety、README/docs との整合を依頼したときに使う。"
---

# Rustdoc Style

rustdoc を、利用者が呼び出し方、失敗条件、feature 条件を判断できる公開契約として書く。

## 規則

- 公開 item は一文で役割を示し、必要な前提・副作用・所有権・error を具体的に続ける。
- 失敗する `Result` には `# Errors`、panic の可能性には `# Panics`、unsafe function には `# Safety` を書く。
- 公開 API の example は最小の利用例にし、実行可能な例だけを載せる。
- feature 条件、プラットフォーム制約、blocking / async の性質を隠さない。
- README は利用開始、rustdoc は item ごとの契約に分け、同じ説明を二重管理しない。

## 手順

1. 公開 module、re-export、対象 docs、`Cargo.toml` の feature を確認する。
2. API 契約を先に確定し、rustdoc を追加・更新する。
3. doctest を実行可能な範囲で `cargo test --doc --all-features` により確認する。
4. 公開 docs も変えた場合は `docs-quality-review` を使う。

## 確認候補

```powershell
cargo fmt --check
cargo test --doc --all-features
cargo clippy --all-targets --all-features -- -D warnings
```
