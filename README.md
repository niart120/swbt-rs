# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack の実装基盤には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) を使います。

## 現在の状態

このリポジトリは M0 の基盤実装段階です。Cargo package は library target `swbt` を公開し、
仕様・テスト・品質確認を作業単位ごとに進めます。

- Bluetooth transport、HID protocol、controller の runtime / lifecycle API は未実装です。
- core `bumble` は基準 commit `bbac2a6803b8cab0920ab725a23aa408fc4fed85` に固定しています。
- Bumble と transport の統合処理は未実装です。
- Bluetooth adapter や対象機器を使う実機検証は未実施です。

## 開発

MSRV は Rust 1.87 です。現在のローカル確認は次の command で実行できます。

```powershell
cargo fmt --all --check
cargo check --all-targets --all-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo test --doc --all-features --locked
cargo build --all-features --locked
git diff --check
```

作業境界とリポジトリ固有の手順は [AGENTS.md](AGENTS.md) と
[SKILLS.md](SKILLS.md) にあります。

現在利用できる model-valid input 型は
[examples/type_model.rs](examples/type_model.rs) で確認できます。

## ライセンス

MIT ライセンスです。全文は [LICENSE](LICENSE) にあります。
