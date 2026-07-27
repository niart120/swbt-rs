# swbt-rs

`swbt-rs` は、NX 互換の仮想 Bluetooth HID 入力デバイスを扱う
[`swbt-python`](https://github.com/niart120/swbt-python) を Rust へ移植するプロジェクトです。
Bluetooth stack には
[`bumble-rs`](https://github.com/chaitanyarahalkar/bumble-rs) の利用を検討しています。

## 現在の状態

このリポジトリは実装開始前の初期段階です。現在含まれているのは Rust バイナリ crate の
最小構成と、仕様・テスト・品質確認を作業単位ごとに進めるための開発基盤です。

- Bluetooth transport、HID protocol、controller API は未実装です。
- `bumble-rs` の依存追加と commit 固定は未実施です。
- Bluetooth adapter や対象機器を使う実機検証は未実施です。

## 開発

必要な Rust toolchain は、`Cargo.toml` と CI を導入する作業単位で確定します。
現在のローカル確認は次の command で実行できます。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --all-features
git diff --check
```

作業境界とリポジトリ固有の手順は [AGENTS.md](AGENTS.md) と
[SKILLS.md](SKILLS.md) にあります。

## ライセンス

ライセンスは未設定です。
